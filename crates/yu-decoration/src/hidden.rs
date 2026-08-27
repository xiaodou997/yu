//! source ↔ visual 映射的实现：一棵带 summary 的持久化和树。
//!
//! 不变量 D4 要求 O(log n) 的双向映射且 round-trip 无损，D2 要求不可变、
//! 与 Revision 绑定、可安全并发读取。
//!
//! # 文档被切成什么
//!
//! 把所有会隐藏 source 的装饰（`Replace` 与 `Widget`）的区间**合并成不重叠、
//! 不相邻的升序区间**之后，文档就成了一串交替的段：
//!
//! ```text
//!   可见  隐藏   可见   隐藏  可见
//!   ├──┤ ├───┤ ├────┤ ├──┤ ├───┤
//!   `## ` 被隐藏，`Title` 可见
//! ```
//!
//! 每个 [`Segment`] 记一段可见字节和紧随其后的隐藏字节。视觉字节流就是把
//! 所有 `hidden` 去掉之后剩下的东西，于是：
//!
//! ```text
//!   source = Σ (visible + hidden)
//!   visual = Σ  visible
//! ```
//!
//! 这两个和就是树节点携带的 summary。查询是一次自根向下的下降，每层用
//! summary 决定进哪个孩子——这就是 O(log n) 的来源。
//!
//! # 为什么合并是必须的
//!
//! 两段相邻（中间零个可见字节）的隐藏区间落在**同一个视觉偏移**上。不合并的
//! 话，`visual_to_source(v, After)` 得沿着后继一路找「还有没有下一段也贴在这个
//! 位置」——v1 的 `yu-projection` 就是这么做的，一个 `for` 循环挂在热路径上。
//! 合并之后这件事在构造期一次做完，查询里不再有这个循环。

use std::sync::Arc;

/// 一段可见字节，以及紧随其后被隐藏的字节。
///
/// 不变式（由 [`HiddenIndex::build`] 保证）：
///
/// - 只有第一段允许 `visible == 0`（文档以隐藏语法开头）；
/// - 只有最后一段允许 `hidden == 0`（文档以可见文本结尾）。
///
/// 两条合起来使「相邻的隐藏区间」不可能被表示出来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Segment {
    pub visible: u64,
    pub hidden: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    pub source: u64,
    pub visual: u64,
}

impl Summary {
    fn of(segment: Segment) -> Self {
        Self {
            source: segment.visible + segment.hidden,
            visual: segment.visible,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            source: self.source + other.source,
            visual: self.visual + other.visual,
        }
    }
}

/// 分支因子。16 让百万级区间的树高停在 5 层左右，同时每个节点仍然装得进
/// 一两条 cache line。这个数字没有魔力，换成 8 或 32 都能工作。
const BRANCHING: usize = 16;

struct Node {
    summary: Summary,
    kind: NodeKind,
}

enum NodeKind {
    Leaf(Box<[Segment]>),
    Internal(Box<[Arc<Node>]>),
}

/// 查询时的偏好。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bias {
    /// 落在隐藏区间上时，解析到它**之前**的 source 位置。
    Before,
    /// 落在隐藏区间上时，解析到它**之后**的 source 位置。
    #[default]
    After,
}

/// 一份 revision 的 source ↔ visual 映射。
#[derive(Clone)]
pub(crate) struct HiddenIndex {
    root: Option<Arc<Node>>,
    summary: Summary,
}

impl HiddenIndex {
    /// 从**已合并**的隐藏区间与文档总长度建索引。
    ///
    /// `hidden` 必须升序、不重叠、不相邻，且都落在 `source_len` 之内。
    /// 调用方是 [`crate::set::DecorationSet`]，合并在那里做。
    pub(crate) fn build(hidden: &[(u64, u64)], source_len: u64) -> Self {
        let mut segments = Vec::with_capacity(hidden.len() + 1);
        let mut cursor = 0_u64;
        for (index, &(from, to)) in hidden.iter().enumerate() {
            // 不只要求升序不重叠，还要求**不相邻**。相邻的两段落在同一个视觉
            // 偏移上，会让中间出现一个 `visible == 0` 的段，而
            // `source_for_visual(.., After)` 只看当前段的 `hidden`——它会停在
            // 第一段的末尾，少跳过后面那段。这个断言把「合并没做干净」变成
            // 构造期的失败，而不是查询期一个差几个字节的错误答案。
            debug_assert!(
                index == 0 || from > cursor,
                "隐藏区间必须升序、不重叠、不相邻；{from} 紧贴或越过了 {cursor}"
            );
            debug_assert!(to <= source_len, "隐藏区间不得越过文档末尾");
            debug_assert!(from < to, "空的隐藏区间应该在合并时就被丢掉");
            segments.push(Segment {
                visible: from - cursor,
                hidden: to - from,
            });
            cursor = to;
        }
        // 末尾的可见部分。文档以隐藏语法结尾时它是空段，但仍然要在，
        // 好让 `source_len` 与 summary 对得上。
        segments.push(Segment {
            visible: source_len.saturating_sub(cursor),
            hidden: 0,
        });

        let summary = segments
            .iter()
            .copied()
            .map(Summary::of)
            .fold(Summary::default(), Summary::add);
        Self {
            root: build_tree(segments),
            summary,
        }
    }

    /// 索引自己记的 source 总长。
    ///
    /// 生产路径上不需要它——`DecorationSet` 有自己的 `source_len`。留着是为了
    /// 让测试能校验「summary 的和与传入的长度一致」，那是建树时最容易悄悄
    /// 算错的量。
    #[cfg(test)]
    pub(crate) fn source_len(&self) -> u64 {
        self.summary.source
    }

    pub(crate) fn visual_len(&self) -> u64 {
        self.summary.visual
    }

    /// source 偏移 → visual 偏移。
    ///
    /// 落在隐藏区间内部的 source 偏移全部解析到该区间的视觉位置——隐藏区间
    /// 的视觉宽度是零，它的「前」和「后」是同一个视觉偏移，所以这个方向
    /// **不需要 bias**。
    pub(crate) fn visual_for_source(&self, source: u64) -> u64 {
        let Some(root) = &self.root else {
            return 0;
        };
        let source = source.min(self.summary.source);
        let mut node = root.as_ref();
        let mut remaining = source;
        let mut visual = 0_u64;
        loop {
            match &node.kind {
                NodeKind::Internal(children) => {
                    let mut next = None;
                    for child in children.iter() {
                        if remaining <= child.summary.source {
                            next = Some(child.as_ref());
                            break;
                        }
                        remaining -= child.summary.source;
                        visual += child.summary.visual;
                    }
                    // `remaining <= summary.source` 在最后一个孩子上一定成立，
                    // 因为上面已经把 source 夹在总长之内。
                    node = next.expect("下降必然落在某个孩子上");
                }
                NodeKind::Leaf(segments) => {
                    for segment in segments.iter() {
                        if remaining <= segment.visible {
                            return visual + remaining;
                        }
                        remaining -= segment.visible;
                        visual += segment.visible;
                        if remaining <= segment.hidden {
                            // 落在隐藏区间里（含它的末端）。
                            return visual;
                        }
                        remaining -= segment.hidden;
                    }
                    return visual;
                }
            }
        }
    }

    /// visual 偏移 → source 偏移。
    ///
    /// 一个视觉偏移可能对应一整段 source（那段被隐藏了）。`bias` 选哪一端。
    pub(crate) fn source_for_visual(&self, visual: u64, bias: Bias) -> u64 {
        let Some(root) = &self.root else {
            return 0;
        };
        let visual = visual.min(self.summary.visual);
        let mut node = root.as_ref();
        let mut remaining = visual;
        let mut source = 0_u64;
        loop {
            match &node.kind {
                NodeKind::Internal(children) => {
                    let mut next = None;
                    for child in children.iter() {
                        if remaining <= child.summary.visual {
                            next = Some(child.as_ref());
                            break;
                        }
                        remaining -= child.summary.visual;
                        source += child.summary.source;
                    }
                    node = next.expect("下降必然落在某个孩子上");
                }
                NodeKind::Leaf(segments) => {
                    for segment in segments.iter() {
                        if remaining < segment.visible {
                            return source + remaining;
                        }
                        if remaining == segment.visible {
                            // 正好停在可见段与隐藏段的交界上。
                            let at = source + segment.visible;
                            return match bias {
                                Bias::Before => at,
                                Bias::After => at + segment.hidden,
                            };
                        }
                        remaining -= segment.visible;
                        source += segment.visible + segment.hidden;
                    }
                    return source;
                }
            }
        }
    }
}

/// 自底向上建树。
///
/// 自底向上而不是逐个插入：装饰集合是**每个 revision 整体重建**的
/// （不变量 D2），没有「往已有集合里插一条」这个操作。一次性建树既简单，
/// 又天然平衡——不需要旋转，也就不会有旋转写错这一类 bug。
fn build_tree(segments: Vec<Segment>) -> Option<Arc<Node>> {
    if segments.is_empty() {
        return None;
    }
    let mut level: Vec<Arc<Node>> = segments
        .chunks(BRANCHING)
        .map(|chunk| {
            let summary = chunk
                .iter()
                .copied()
                .map(Summary::of)
                .fold(Summary::default(), Summary::add);
            Arc::new(Node {
                summary,
                kind: NodeKind::Leaf(chunk.to_vec().into_boxed_slice()),
            })
        })
        .collect();

    while level.len() > 1 {
        level = level
            .chunks(BRANCHING)
            .map(|chunk| {
                let summary = chunk
                    .iter()
                    .map(|child| child.summary)
                    .fold(Summary::default(), Summary::add);
                Arc::new(Node {
                    summary,
                    kind: NodeKind::Internal(chunk.to_vec().into_boxed_slice()),
                })
            })
            .collect();
    }
    level.pop()
}

#[cfg(test)]
mod tests {
    use super::{Bias, HiddenIndex};

    /// 不走树的参照实现：从头线性扫。树的下降必须与它逐点一致。
    ///
    /// 留着它是因为「O(log n) 的下降」和「显然正确的线性扫」是两份独立的
    /// 推理，下面的用例拿它们互相校验。只有一份实现的时候，写错了没人知道。
    fn reference_visual_for_source(hidden: &[(u64, u64)], source_len: u64, source: u64) -> u64 {
        let source = source.min(source_len);
        let mut removed = 0_u64;
        for &(from, to) in hidden {
            if source <= from {
                break;
            }
            removed += source.min(to) - from;
        }
        source - removed
    }

    fn reference_source_for_visual(
        hidden: &[(u64, u64)],
        source_len: u64,
        visual: u64,
        bias: Bias,
    ) -> u64 {
        // 所有映射到 `visual` 的 source 偏移构成一个闭区间，取它的两端。
        let mut first = None;
        let mut last = 0_u64;
        for source in 0..=source_len {
            if reference_visual_for_source(hidden, source_len, source) == visual {
                if first.is_none() {
                    first = Some(source);
                }
                last = source;
            }
        }
        match bias {
            Bias::Before => first.unwrap_or(0),
            Bias::After => last,
        }
    }

    fn check(hidden: &[(u64, u64)], source_len: u64) {
        let index = HiddenIndex::build(hidden, source_len);
        assert_eq!(index.source_len(), source_len);
        for source in 0..=source_len {
            assert_eq!(
                index.visual_for_source(source),
                reference_visual_for_source(hidden, source_len, source),
                "source {source} 在 {hidden:?} / len {source_len} 上不一致"
            );
        }
        for visual in 0..=index.visual_len() {
            for bias in [Bias::Before, Bias::After] {
                assert_eq!(
                    index.source_for_visual(visual, bias),
                    reference_source_for_visual(hidden, source_len, visual, bias),
                    "visual {visual} / {bias:?} 在 {hidden:?} / len {source_len} 上不一致"
                );
            }
        }
    }

    #[test]
    fn descent_matches_the_linear_reference() {
        check(&[], 0);
        check(&[], 10);
        check(&[(0, 3)], 10);
        check(&[(0, 10)], 10);
        check(&[(2, 5)], 10);
        check(&[(7, 10)], 10);
        check(&[(0, 2), (5, 7)], 10);
        check(&[(1, 2), (3, 4), (5, 6), (7, 8)], 9);
    }

    /// 树高必须真的超过一层，否则上面那些用例只测到了叶子里的线性扫描，
    /// 下降的分支逻辑一次都没跑到。
    #[test]
    fn descent_is_exercised_across_multiple_tree_levels() {
        let hidden: Vec<(u64, u64)> = (0..200)
            .map(|index| (index * 5 + 1, index * 5 + 3))
            .collect();
        let source_len = 200 * 5 + 4;
        check(&hidden, source_len);
    }

    /// round-trip 无损（不变量 D4）：任何 visual 偏移换成 source 再换回来，
    /// 必须回到原处。
    #[test]
    fn visual_round_trips_through_source() {
        let hidden = [(0, 2), (5, 7), (11, 15)];
        let index = HiddenIndex::build(&hidden, 20);
        for visual in 0..=index.visual_len() {
            for bias in [Bias::Before, Bias::After] {
                let source = index.source_for_visual(visual, bias);
                assert_eq!(
                    index.visual_for_source(source),
                    visual,
                    "visual {visual} / {bias:?} round-trip 丢了"
                );
            }
        }
    }
}
