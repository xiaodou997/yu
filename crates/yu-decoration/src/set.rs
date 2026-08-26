//! 不可变的装饰集合。

use std::sync::Arc;

use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, VisualOffset};
use yu_text::{AnchorMapError, ChangeSet};

use crate::decoration::{Decoration, DecorationRange};
use crate::hidden::{Bias, HiddenIndex};

/// `map` 失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapError {
    /// 装饰集合绑定的 Revision 与 ChangeSet 的输入 Revision 不符。
    ///
    /// 不变量 D2 要求装饰集合与 Revision 绑定；把一个 revision 的装饰按另一个
    /// revision 的改动去迁移，结果不会报错，只会静默错位。所以这里拒绝。
    RevisionMismatch {
        set: Revision,
        changes: Revision,
    },
    AnchorMap(AnchorMapError),
}

impl core::fmt::Display for MapError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RevisionMismatch { set, changes } => {
                write!(formatter, "装饰集合属于 {set:?}，而改动来自 {changes:?}")
            }
            Self::AnchorMap(error) => write!(formatter, "锚点迁移失败：{error:?}"),
        }
    }
}

impl core::error::Error for MapError {}

/// [`DecorationSet::merge`] 失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeError {
    /// 参与合并的某个集合属于另一个 Revision。
    RevisionMismatch { expected: Revision, found: Revision },
    /// 参与合并的某个集合看到的源码长度不同。
    ///
    /// 不取 max 也不截断：长度不同意味着两个 extension 看的是不同的文档，
    /// 它们的 offset 不可比。合并会得到一份看起来正常、位置全错的集合——
    /// 正是这个项目最危险的那类失败。
    SourceLenMismatch {
        expected: ByteOffset,
        found: ByteOffset,
    },
}

impl core::fmt::Display for MergeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RevisionMismatch { expected, found } => {
                write!(formatter, "要合并 {expected:?}，遇到 {found:?}")
            }
            Self::SourceLenMismatch { expected, found } => {
                write!(formatter, "源码长度应为 {expected:?}，遇到 {found:?}")
            }
        }
    }
}

impl core::error::Error for MergeError {}

/// 一份 revision 的全部装饰。
///
/// 不可变、可克隆（克隆是 `Arc` 克隆）、可安全并发读取——不变量 D2。
#[derive(Clone)]
pub struct DecorationSet {
    revision: Revision,
    source_len: ByteOffset,
    /// 按 [`DecorationRange::order_key`] 定序。区间查询在它上面二分。
    ranges: Arc<[DecorationRange]>,
    /// 会隐藏 source 的那些装饰合并之后的区间：升序、不重叠、不相邻。
    hidden_spans: Arc<[(ByteOffset, ByteOffset)]>,
    /// 由 `hidden_spans` 建出来的映射索引。
    hidden: HiddenIndex,
}

impl DecorationSet {
    /// 从若干 extension 的产出建集合。
    ///
    /// `ranges` 不需要有序：这里会按 `(from, side, priority)` 定序
    /// （不变量 D6），于是合并结果与它们被加进来的先后无关。
    #[must_use]
    pub fn new(
        revision: Revision,
        source_len: ByteOffset,
        ranges: impl IntoIterator<Item = DecorationRange>,
    ) -> Self {
        let mut ranges: Vec<DecorationRange> = ranges
            .into_iter()
            .filter(|entry| entry.range.end() <= source_len)
            .collect();
        ranges.sort_by_key(DecorationRange::order_key);

        let hidden = merge_hidden(&ranges);
        let index = HiddenIndex::build(&hidden, source_len.get());
        Self {
            revision,
            source_len,
            ranges: ranges.into(),
            hidden_spans: hidden
                .iter()
                .map(|&(from, to)| (ByteOffset::new(from), ByteOffset::new(to)))
                .collect(),
            hidden: index,
        }
    }

    /// 合并多个 extension 各自产出的集合（不变量 D6）。
    ///
    /// # 为什么不是「把所有 `DecorationRange` 收集起来调 [`DecorationSet::new`]」
    ///
    /// 因为 D6 的要点是每个 extension 的产出是一个**独立的单位**：可以单独
    /// 缓存、单独重算、单独失效。把它们摊平成一个 range 列表就丢掉了这个边界，
    /// 于是「只有强调的规则变了」也得让所有 extension 重跑一遍。借用而不是
    /// 取所有权也是这个理由——调用方要留着各自的产出。
    ///
    /// 合并结果与 `sets` 的先后无关：定序键 [`DecorationRange::order_key`]
    /// 是全序且只依赖装饰自身的值。extension 之间因此不需要相互感知。
    ///
    /// `revision` 与 `source_len` 显式传入而不是从第一个集合里取：空输入
    /// 也要有确定的结果，而且从输入里推断会让「所有集合都错但彼此一致」
    /// 这种情况静静通过。
    ///
    /// # Errors
    ///
    /// 任一集合的 Revision 或源码长度与给定的不符。
    pub fn merge<'a>(
        revision: Revision,
        source_len: ByteOffset,
        sets: impl IntoIterator<Item = &'a Self>,
    ) -> Result<Self, MergeError> {
        let mut ranges = Vec::new();
        for set in sets {
            if set.revision != revision {
                return Err(MergeError::RevisionMismatch {
                    expected: revision,
                    found: set.revision,
                });
            }
            if set.source_len != source_len {
                return Err(MergeError::SourceLenMismatch {
                    expected: source_len,
                    found: set.source_len,
                });
            }
            ranges.extend_from_slice(&set.ranges);
        }
        Ok(Self::new(revision, source_len, ranges))
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn source_len(&self) -> ByteOffset {
        self.source_len
    }

    /// 投影后的视觉字节数。
    #[must_use]
    pub fn visual_len(&self) -> VisualOffset {
        VisualOffset::new(self.hidden.visual_len())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// 全部装饰，按定序返回。
    #[must_use]
    pub fn all(&self) -> &[DecorationRange] {
        &self.ranges
    }

    /// 被藏起来的 source 区间，升序、不重叠、不相邻。
    ///
    /// 这是**映射索引的原料**，不是另算一遍：`source_to_visual` 的那棵树
    /// 就建在这份数据上。想拼出视觉文本的调用方（`yu-editor::VisualText`）
    /// 必须用它，自己再遍历一遍 [`DecorationSet::all`] 去数「哪些字节被
    /// 藏了」就是第二个实现——不变量 D4 说那件事只能有一个实现。
    #[must_use]
    pub fn hidden_spans(&self) -> &[(ByteOffset, ByteOffset)] {
        &self.hidden_spans
    }

    /// 与 `from..=to` 相接或相交的装饰，按定序返回。
    ///
    /// **端点是闭的**：起点等于 `to`、或终点等于 `from` 的装饰也算命中。
    ///
    /// 这个方向是有意选的。查询的典型用途是「取出 viewport 里要画的装饰」，
    /// 而一个空 range 的 widget 正好落在 viewport 边界上时，半开区间会让它
    /// 整个消失——不报错，只是少画一个东西。多给出一条的代价是多画一次，
    /// 少给出一条的代价是内容不见了，两者不对称。
    ///
    /// 要「全部装饰」用 [`DecorationSet::all`]，不要用 `in_range(0, len)`：
    /// 那是在依赖这里的端点语义，而端点语义是这个方法最容易被改的部分。
    ///
    /// O(log n + k)：上界二分，之后线性过滤。
    #[must_use]
    pub fn in_range(&self, from: ByteOffset, to: ByteOffset) -> Vec<DecorationRange> {
        // 装饰按 `from` 有序，二分给出「起点 <= to」的上界。下界给不出来：
        // 一条很长的装饰可能起点很靠前却盖到查询区间里，所以仍要从头过滤。
        // 区间树能去掉这一步，但那要等到有证据表明它是热点。
        let upper = self
            .ranges
            .partition_point(|entry| entry.range.start() <= to);
        self.ranges[..upper]
            .iter()
            .filter(|entry| entry.range.end() >= from)
            .copied()
            .collect()
    }

    /// source 偏移 → visual 偏移。
    ///
    /// 隐藏区间的视觉宽度是零，它的「前」与「后」是同一个视觉偏移，
    /// 所以这个方向不需要 bias。
    ///
    /// # 这里不校验 UTF-8 边界
    ///
    /// 装饰集合**不持有源码**——它是一组区间加一个 Revision，建立与迁移都
    /// 不需要碰文本。因此它无法回答「这个偏移是不是字符边界」，也就不做这个
    /// 校验：落在字符中间的偏移会得到一个算术上正确、语义上无意义的答案。
    ///
    /// 校验属于持有文本的那一层（`yu-text` 的偏移校验）。
    /// `docs/specs/coordinates.md` 要求「不能静默取整」——这里没有取整，
    /// 是把校验的责任明确地留在了上面，而不是悄悄替调用方决定。
    #[must_use]
    pub fn source_to_visual(&self, source: ByteOffset) -> VisualOffset {
        VisualOffset::new(self.hidden.visual_for_source(source.get()))
    }

    /// visual 偏移 → source 偏移。
    #[must_use]
    pub fn visual_to_source(&self, visual: VisualOffset, bias: Bias) -> ByteOffset {
        ByteOffset::new(self.hidden.source_for_visual(visual.get(), bias))
    }

    /// 随一次 Transaction 迁移到新的 Revision（不变量 D3）。
    ///
    /// # 边界 bias
    ///
    /// 迁移复用 [`ChangeSet::map_anchor`]，起点用 [`Affinity::After`]、
    /// 终点用 [`Affinity::Before`]。也就是**紧贴装饰边界键入的字符落在装饰
    /// 之外**：在隐藏的 `##` 后面打字，新字符是可见正文，不会被一起吞掉。
    ///
    /// 复用 Anchor 而不是另写一套偏移平移，是为了让装饰与 selection 的边界
    /// 语义**由构造保证一致**。两处各写一遍的话，它们会在某个 edge case 上
    /// 分叉，而分叉的表现是光标和高亮差一个字节——不报错。
    ///
    /// # Errors
    ///
    /// Revision 不匹配，或锚点迁移失败。
    pub fn map(&self, changes: &ChangeSet) -> Result<Self, MapError> {
        if self.revision != changes.before() {
            return Err(MapError::RevisionMismatch {
                set: self.revision,
                changes: changes.before(),
            });
        }
        let mut mapped = Vec::with_capacity(self.ranges.len());
        for entry in self.ranges.iter() {
            let start = changes
                .map_anchor(TextAnchor::new(
                    self.revision,
                    entry.range.start(),
                    Affinity::After,
                ))
                .map_err(MapError::AnchorMap)?
                .offset();
            let end = changes
                .map_anchor(TextAnchor::new(
                    self.revision,
                    entry.range.end(),
                    Affinity::Before,
                ))
                .map_err(MapError::AnchorMap)?
                .offset();
            // 起点被推到终点之后，说明这条装饰覆盖的 source 被删光了。
            let end = end.max(start);
            let Some(range) = yu_core::TextRange::new(start, end) else {
                continue;
            };
            // 覆盖范围被删空的隐藏类装饰直接丢掉：一个零宽的 `Replace`
            // 什么也不隐藏，留着只会让集合里堆满看不见的东西。
            if range.is_empty() && entry.decoration.hides_source() {
                continue;
            }
            mapped.push(DecorationRange {
                range,
                decoration: entry.decoration,
                priority: entry.priority,
            });
        }
        let new_len = mapped_source_len(self.source_len, changes);
        Ok(Self::new(changes.after(), new_len, mapped))
    }
}

/// 把会隐藏 source 的装饰合并成升序、不重叠、不相邻的区间。
///
/// 相邻也要合并：两段中间隔零个可见字节的隐藏区间落在同一个视觉偏移上，
/// 不合并的话 `visual_to_source` 就得在查询时沿着后继找「还有没有下一段也贴
/// 在这里」。合并把这件事挪到构造期，一次做完。
fn merge_hidden(ranges: &[DecorationRange]) -> Vec<(u64, u64)> {
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for entry in ranges {
        if !entry.decoration.hides_source() || entry.range.is_empty() {
            continue;
        }
        let (from, to) = (entry.range.start().get(), entry.range.end().get());
        match merged.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    merged
}

/// 一次改动之后的文档长度。
fn mapped_source_len(before: ByteOffset, changes: &ChangeSet) -> ByteOffset {
    let mut length = i128::from(before.get());
    for change in changes.changes() {
        length += i128::from(change.new_range().len()) - i128::from(change.old_range().len());
    }
    ByteOffset::new(u64::try_from(length.max(0)).unwrap_or(0))
}

/// 让 `Decoration` 在本模块可见（`merge_hidden` 通过 `hides_source` 用到）。
const _: fn(Decoration) -> bool = Decoration::hides_source;

/// 不变量 D2：装饰集合不可变、与 Revision 绑定，**可安全并发读取**。
///
/// 「可安全并发读取」在 Rust 里就是 `Send + Sync`，而它是由字段推导出来的，
/// 不是声明出来的——今天成立不代表明天成立。往树里塞一个 `Rc` 或 `Cell`
/// 做缓存，编译照过、测试全绿，只有把集合发给后台任务的那一刻才炸，而那
/// 条路径此刻还不存在（S4 只建数据结构，G1 的后台快照读取要到后面才接）。
/// 这一行让它在编译期就失败。
const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<DecorationSet>();
};

#[cfg(test)]
mod tests {
    use super::{DecorationSet, MapError, MergeError};
    use crate::decoration::{Decoration, DecorationRange, StyleId, WidgetId, WidgetSide};
    use crate::hidden::Bias;
    use yu_core::{ByteOffset, Revision, TextRange, VisualOffset};
    use yu_text::{Edit, TextBuffer, Transaction};

    fn range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end)).expect("有序")
    }

    fn replace(start: u64, end: u64) -> DecorationRange {
        DecorationRange::new(range(start, end), Decoration::Replace)
    }

    fn set(source_len: u64, ranges: Vec<DecorationRange>) -> DecorationSet {
        DecorationSet::new(Revision::INITIAL, ByteOffset::new(source_len), ranges)
    }

    fn visual(value: u64) -> VisualOffset {
        VisualOffset::new(value)
    }

    /// **相邻的隐藏区间必须被合并。**
    ///
    /// 两段中间隔零个可见字节的隐藏区间落在同一个视觉偏移上。不合并的话
    /// `visual_to_source(.., After)` 会停在第一段的末尾，少跳过第二段——
    /// 光标会卡在两段语法字符的中间，不报错。
    ///
    /// `## ` 后面紧跟着一个被隐藏的 `**` 就是这个形状。
    #[test]
    fn adjacent_hidden_ranges_collapse_to_one_visual_position() {
        // source: `##` `**` `bold`   隐藏 0..2 与 2..4
        let decorations = set(8, vec![replace(0, 2), replace(2, 4)]);
        assert_eq!(decorations.visual_len(), visual(4));
        assert_eq!(
            decorations.visual_to_source(visual(0), Bias::After),
            ByteOffset::new(4),
            "After 必须跳过**全部**连续的隐藏语法"
        );
        assert_eq!(
            decorations.visual_to_source(visual(0), Bias::Before),
            ByteOffset::new(0)
        );
    }

    /// 拆成几个 extension 各自产出，再合并，必须与一次性构造完全相同。
    #[test]
    fn merging_extensions_equals_building_in_one_go() {
        let mark = DecorationRange::new(range(2, 6), Decoration::Mark { style: StyleId(1) });
        let one = set(12, vec![replace(0, 2), mark]);
        let two = set(12, vec![replace(7, 9)]);
        let merged = DecorationSet::merge(Revision::INITIAL, ByteOffset::new(12), [&one, &two])
            .expect("同一个 revision 与长度");
        let direct = set(12, vec![replace(0, 2), mark, replace(7, 9)]);

        assert_eq!(merged.all(), direct.all());
        assert_eq!(merged.visual_len(), direct.visual_len());
    }

    /// 合并结果不依赖 extension 的先后——不变量 D6 的「不得相互感知」。
    #[test]
    fn merging_does_not_depend_on_extension_order() {
        let one = set(12, vec![replace(0, 2)]);
        let two = set(
            12,
            vec![DecorationRange::new(
                range(0, 2),
                Decoration::Mark { style: StyleId(4) },
            )],
        );
        let forward = DecorationSet::merge(Revision::INITIAL, ByteOffset::new(12), [&one, &two])
            .expect("同一个 revision 与长度");
        let backward = DecorationSet::merge(Revision::INITIAL, ByteOffset::new(12), [&two, &one])
            .expect("同一个 revision 与长度");
        assert_eq!(forward.all(), backward.all());
    }

    /// **相邻的隐藏区间来自不同 extension 时同样要合并。**
    ///
    /// 这是 merge 独有的风险：单个集合内部的相邻合并由 `merge_hidden` 保证，
    /// 而两段分别来自两个 extension 时，只有合并后重新走一遍构造才会塌成
    /// 一个视觉位置。不塌的话光标会卡在两段语法字符中间，不报错。
    #[test]
    fn adjacent_hidden_ranges_from_different_extensions_still_collapse() {
        let one = set(8, vec![replace(0, 2)]);
        let two = set(8, vec![replace(2, 4)]);
        let merged = DecorationSet::merge(Revision::INITIAL, ByteOffset::new(8), [&one, &two])
            .expect("同一个 revision 与长度");
        assert_eq!(merged.visual_len(), visual(4));
        assert_eq!(
            merged.visual_to_source(visual(0), Bias::After),
            ByteOffset::new(4),
            "After 必须跳过来自两个 extension 的连续隐藏区间"
        );
    }

    /// 空输入是合法的，结果是一份什么都不装饰的集合。
    #[test]
    fn merging_nothing_yields_an_empty_set() {
        let merged =
            DecorationSet::merge(Revision::INITIAL, ByteOffset::new(9), []).expect("空输入合法");
        assert!(merged.is_empty());
        assert_eq!(merged.visual_len(), visual(9));
    }

    /// Revision 不符必须拒绝，不能静默合并——同 `map` 的理由。
    #[test]
    fn merging_across_revisions_is_rejected() {
        let one = set(8, vec![replace(0, 2)]);
        let later = Revision::INITIAL.next().expect("不会溢出");
        let other = DecorationSet::new(later, ByteOffset::new(8), vec![replace(4, 6)]);
        assert_eq!(
            DecorationSet::merge(Revision::INITIAL, ByteOffset::new(8), [&one, &other]).err(),
            Some(MergeError::RevisionMismatch {
                expected: Revision::INITIAL,
                found: later,
            })
        );
    }

    /// 源码长度不符同样拒绝：两个 extension 看的是不同的文档，offset 不可比。
    #[test]
    fn merging_sets_built_against_different_lengths_is_rejected() {
        let one = set(8, vec![replace(0, 2)]);
        let other = set(9, vec![replace(4, 6)]);
        assert_eq!(
            DecorationSet::merge(Revision::INITIAL, ByteOffset::new(8), [&one, &other]).err(),
            Some(MergeError::SourceLenMismatch {
                expected: ByteOffset::new(8),
                found: ByteOffset::new(9),
            })
        );
    }

    #[test]
    fn overlapping_hidden_ranges_merge() {
        let decorations = set(10, vec![replace(1, 6), replace(3, 8)]);
        assert_eq!(decorations.visual_len(), visual(3));
        assert_eq!(
            decorations.visual_to_source(visual(1), Bias::After),
            ByteOffset::new(8)
        );
    }

    /// `Mark` 不隐藏任何东西，视觉长度必须与 source 一致（不变量 D5 的反面）。
    #[test]
    fn marks_do_not_change_the_visual_length() {
        let decorations = set(
            10,
            vec![DecorationRange::new(
                range(2, 6),
                Decoration::Mark { style: StyleId(1) },
            )],
        );
        assert_eq!(decorations.visual_len(), visual(10));
        for offset in 0..=10 {
            assert_eq!(
                decorations.source_to_visual(ByteOffset::new(offset)),
                visual(offset)
            );
        }
    }

    /// 非空 range 的 widget 会盖住并隐藏它覆盖的 source；空 range 的不会。
    #[test]
    fn widgets_hide_only_what_they_cover() {
        let covering = set(
            10,
            vec![DecorationRange::new(
                range(2, 6),
                Decoration::Widget {
                    widget: WidgetId(1),
                    side: WidgetSide::Before,
                },
            )],
        );
        assert_eq!(covering.visual_len(), visual(6));

        let inserted = set(
            10,
            vec![DecorationRange::new(
                range(4, 4),
                Decoration::Widget {
                    widget: WidgetId(1),
                    side: WidgetSide::Before,
                },
            )],
        );
        assert_eq!(inserted.visual_len(), visual(10));
    }

    /// **不变量 D5**：`Replace` 让视觉宽度为零，但 source 长度不变、
    /// 可被光标穿越——`Before` 与 `After` 落在它的两端，中间的每个 source
    /// 偏移都还能被表达。
    #[test]
    fn replaced_source_stays_addressable() {
        let decorations = set(10, vec![replace(3, 7)]);
        assert_eq!(decorations.source_len(), ByteOffset::new(10));
        for offset in 3..=7 {
            assert_eq!(
                decorations.source_to_visual(ByteOffset::new(offset)),
                visual(3),
                "被隐藏的 source 仍然要有一个视觉位置"
            );
        }
    }

    fn change_set(source: &str, at: u64, remove: u64, insert: &str) -> yu_text::ChangeSet {
        let mut buffer = TextBuffer::new(source.to_owned());
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(range(at, at + remove), insert)],
        );
        buffer
            .apply(&transaction)
            .expect("测试编辑应当合法")
            .change_set()
            .clone()
    }

    /// **边界 bias（不变量 D3）**：紧贴装饰边界键入的字符落在装饰**之外**。
    ///
    /// 在被隐藏的 `##` 正后面打字，新字符必须是可见正文。反过来（装饰把新
    /// 字符吞进去）的表现是「打了字但看不见」，而且不报错。
    #[test]
    fn typing_at_a_decoration_boundary_lands_outside_it() {
        let source = "## title";
        let decorations = DecorationSet::new(
            Revision::INITIAL,
            ByteOffset::new(source.len() as u64),
            vec![replace(0, 3)],
        );
        // 在隐藏区间的末端（offset 3）插入一个字符。
        let changes = change_set(source, 3, 0, "X");
        let mapped = decorations.map(&changes).expect("同 revision 应当能迁移");
        assert_eq!(mapped.len(), 1);
        assert_eq!(
            mapped.in_range(ByteOffset::new(0), ByteOffset::new(9))[0].range,
            range(0, 3),
            "装饰不该把新字符吞进去"
        );
        assert_eq!(mapped.visual_len(), visual(6), "`X` 必须可见");

        // 在起点（offset 0）插入同样不该被吞。
        let changes = change_set(source, 0, 0, "Y");
        let mapped = decorations.map(&changes).expect("同 revision 应当能迁移");
        assert_eq!(
            mapped.in_range(ByteOffset::new(0), ByteOffset::new(9))[0].range,
            range(1, 4)
        );
    }

    /// 覆盖范围被删光的隐藏类装饰要被丢掉——一个零宽的 `Replace` 什么也不
    /// 隐藏，留着只会让集合里堆满看不见的东西。
    #[test]
    fn hidden_decorations_whose_source_is_deleted_are_dropped() {
        let source = "## title";
        let decorations = DecorationSet::new(
            Revision::INITIAL,
            ByteOffset::new(source.len() as u64),
            vec![replace(0, 3)],
        );
        let changes = change_set(source, 0, 3, "");
        let mapped = decorations.map(&changes).expect("同 revision 应当能迁移");
        assert!(mapped.is_empty());
        assert_eq!(mapped.source_len(), ByteOffset::new(5));
        assert_eq!(mapped.visual_len(), visual(5));
    }

    /// **不变量 D2**：装饰集合与 Revision 绑定。拿另一个 revision 的改动去
    /// 迁移不会报错，只会静默错位，所以必须拒绝。
    #[test]
    fn mapping_across_a_revision_gap_is_rejected() {
        let source = "## title";
        let changes = change_set(source, 0, 0, "X");
        let stale = DecorationSet::new(
            changes.after(), // 已经是新 revision 了
            ByteOffset::new(source.len() as u64),
            vec![replace(0, 3)],
        );
        assert!(matches!(
            stale.map(&changes),
            Err(MapError::RevisionMismatch { .. })
        ));
    }

    /// 端点是闭的：正好落在查询边界上的空装饰不能消失。
    ///
    /// 这一条是 `tests/map_properties.rs` 的 property test 抓出来的——
    /// 原来的半开上界让文档末尾的空装饰整个查不到，而手写用例没覆盖到
    /// 「装饰恰好在 `to` 上」这个位置。
    #[test]
    fn range_queries_include_empty_decorations_sitting_on_the_boundary() {
        let at_end = set(
            20,
            vec![DecorationRange::new(
                range(20, 20),
                Decoration::Widget {
                    widget: WidgetId(1),
                    side: WidgetSide::Before,
                },
            )],
        );
        assert_eq!(
            at_end
                .in_range(ByteOffset::new(0), ByteOffset::new(20))
                .len(),
            1,
            "文档末尾的 widget 不能因为查询区间是半开的就消失"
        );
        assert_eq!(at_end.all().len(), 1);

        let at_start = set(
            20,
            vec![DecorationRange::new(
                range(5, 5),
                Decoration::Widget {
                    widget: WidgetId(1),
                    side: WidgetSide::After,
                },
            )],
        );
        assert_eq!(
            at_start
                .in_range(ByteOffset::new(5), ByteOffset::new(9))
                .len(),
            1
        );
    }

    #[test]
    fn range_queries_return_everything_that_overlaps() {
        let decorations = set(
            20,
            vec![
                replace(0, 2),
                DecorationRange::new(range(4, 10), Decoration::Mark { style: StyleId(1) }),
                replace(12, 14),
            ],
        );
        let found = decorations.in_range(ByteOffset::new(5), ByteOffset::new(13));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].range, range(4, 10));
        assert_eq!(found[1].range, range(12, 14));
    }
}
