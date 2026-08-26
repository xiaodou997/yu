//! 把重叠的 `Decoration::Mark` 压平成不重叠的样式段。
//!
//! # 为什么需要压平
//!
//! `DecorationSet` 允许 Mark 重叠且**应该**允许——`**[文字](url)**` 里外层
//! 是加粗、内层是链接，两条 Mark 盖在一起是这件事的忠实表达。但
//! `yu-layout::StyledRun` 要求无缝铺满且不重叠：一个视觉字节只能有一种字型。
//!
//! 中间这一步就在这里。它是**产品决定**，不是数据结构的性质，所以住在
//! `yu-editor`（允许认识 Markdown 的那一层），不住在 `yu-decoration`。
//!
//! # 谁赢
//!
//! 1. **优先级高的赢**（`DecorationRange::priority`）；
//! 2. 同优先级，**窄的赢**。
//!
//! 第二条就是 v1 `Projection::style_for` 的「取最内层」。它也是 link 与
//! image 那条「正文显式排 `Plain`」之所以有效的原因：链接正文比外层的加粗窄，
//! 所以赢。不显式说出来的话链接正文会继承外层，画面变了而不报错。
//!
//! 宽窄相同又同优先级时按 `StyleId` 定序——这一条不是为了「对」，是为了
//! **确定**：两个 extension 在同一段 source 上盖了同样宽的 Mark 时，结果不
//! 能取决于谁先跑（不变量 D6）。

use std::collections::BTreeSet;

use yu_core::{ByteOffset, StyleId, TextRange};

/// 一段 source 上胜出的样式。`None` 表示这一段没有任何 Mark 盖着。
pub(crate) type StyleSegment = (TextRange, Option<StyleId>);

/// 一条候选 Mark。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    pub range: TextRange,
    pub style: StyleId,
    pub priority: i32,
}

/// 把 `marks` 压平成铺满 `bounds` 的一串不重叠样式段。
///
/// 产出**无缝铺满** `bounds`：没有 Mark 盖着的地方给一段 `None`。漏掉那些
/// 空档会让视觉文本少掉几个字——既不 panic 也不报错，正是这个项目最怕的
/// 那类失败。
pub(crate) fn flatten(bounds: TextRange, marks: &[Mark]) -> Vec<StyleSegment> {
    if bounds.is_empty() {
        return Vec::new();
    }
    let (low, high) = (bounds.start().get(), bounds.end().get());

    // 边界只可能出现在某条 Mark 的两端，中间不会变。
    let mut cuts = BTreeSet::from([low, high]);
    for mark in marks {
        for edge in [mark.range.start().get(), mark.range.end().get()] {
            if low < edge && edge < high {
                cuts.insert(edge);
            }
        }
    }

    let mut segments: Vec<StyleSegment> = Vec::new();
    let cuts: Vec<u64> = cuts.into_iter().collect();
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let winner = winner_over(marks, from, to);
        match segments.last_mut() {
            // 相邻且同样式的两段合成一段：布局那边少一次换字型，
            // 而且比对时不会因为「怎么切」不同而假红。
            Some((range, style)) if *style == winner && range.end().get() == from => {
                if let Some(merged) = TextRange::new(range.start(), ByteOffset::new(to)) {
                    *range = merged;
                    continue;
                }
            }
            _ => {}
        }
        if let Some(range) = TextRange::new(ByteOffset::new(from), ByteOffset::new(to)) {
            segments.push((range, winner));
        }
    }
    segments
}

/// `from..to` 上胜出的 Mark。`from..to` 保证不跨任何 Mark 的边界。
fn winner_over(marks: &[Mark], from: u64, to: u64) -> Option<StyleId> {
    marks
        .iter()
        .filter(|mark| mark.range.start().get() <= from && to <= mark.range.end().get())
        .min_by_key(|mark| {
            let width = mark.range.end().get() - mark.range.start().get();
            // 优先级取负，于是「大的赢」变成 `min_by_key` 的「小的赢」。
            (-mark.priority, width, mark.style.0)
        })
        .map(|mark| mark.style)
}

#[cfg(test)]
mod tests {
    use super::{Mark, flatten};
    use yu_core::{ByteOffset, StyleId, TextRange};

    fn range(from: u64, to: u64) -> TextRange {
        TextRange::new(ByteOffset::new(from), ByteOffset::new(to)).expect("测试区间升序")
    }

    fn mark(from: u64, to: u64, style: u32) -> Mark {
        Mark {
            range: range(from, to),
            style: StyleId(style),
            priority: 0,
        }
    }

    fn flat(bounds: (u64, u64), marks: &[Mark]) -> Vec<(u64, u64, Option<u32>)> {
        flatten(range(bounds.0, bounds.1), marks)
            .into_iter()
            .map(|(covered, style)| {
                (
                    covered.start().get(),
                    covered.end().get(),
                    style.map(|style| style.0),
                )
            })
            .collect()
    }

    /// 没有 Mark 时也要铺满：整段一个 `None`。
    #[test]
    fn the_output_tiles_the_whole_range() {
        assert_eq!(flat((0, 10), &[]), vec![(0, 10, None)]);
        assert_eq!(
            flat((0, 10), &[mark(3, 6, 1)]),
            vec![(0, 3, None), (3, 6, Some(1)), (6, 10, None)]
        );
    }

    /// 嵌套时窄的赢。`**[文字](url)**` 就是这个形状：外层加粗盖住整段，
    /// 链接正文另有一条更窄的 Mark。
    #[test]
    fn the_narrower_mark_wins_when_nested() {
        assert_eq!(
            flat((0, 10), &[mark(0, 10, 1), mark(3, 6, 2)]),
            vec![(0, 3, Some(1)), (3, 6, Some(2)), (6, 10, Some(1))]
        );
    }

    /// 优先级压过宽窄。
    #[test]
    fn priority_beats_width() {
        let wide = Mark {
            priority: 5,
            ..mark(0, 10, 1)
        };
        assert_eq!(
            flat((0, 10), &[wide, mark(3, 6, 2)]),
            vec![(0, 10, Some(1))]
        );
    }

    /// 宽窄与优先级都一样时按 `StyleId` 定序。
    ///
    /// 这一条不是为了「对」，是为了**确定**：结果不能取决于哪个 extension
    /// 先跑，否则就是 D6 禁止的相互感知，而且是静默的。
    #[test]
    fn ties_break_on_style_id_not_on_argument_order() {
        let forward = flat((0, 4), &[mark(0, 4, 7), mark(0, 4, 2)]);
        let backward = flat((0, 4), &[mark(0, 4, 2), mark(0, 4, 7)]);
        assert_eq!(forward, backward);
        assert_eq!(forward, vec![(0, 4, Some(2))]);
    }

    /// 相邻且同样式的两段要合成一段。
    ///
    /// 不合的话产出会随「Mark 怎么切」变化，而那是上游的实现细节；差分会
    /// 因此假红，真正的差异反而淹掉。
    #[test]
    fn touching_segments_with_the_same_style_merge() {
        assert_eq!(
            flat((0, 8), &[mark(0, 4, 1), mark(4, 8, 1)]),
            vec![(0, 8, Some(1))]
        );
    }

    /// 部分重叠：重叠的那一段归更窄的那条，两侧各归各的。
    #[test]
    fn partial_overlap_splits_at_both_edges() {
        assert_eq!(
            flat((0, 12), &[mark(0, 8, 1), mark(4, 10, 2)]),
            vec![(0, 4, Some(1)), (4, 10, Some(2)), (10, 12, None)]
        );
    }

    /// 空区间不产出任何段。
    #[test]
    fn an_empty_range_yields_nothing() {
        assert_eq!(flat((5, 5), &[]), Vec::new());
    }
}
