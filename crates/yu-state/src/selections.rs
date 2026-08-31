//! 一组选区：多光标的那个「一组」。
//!
//! # 为什么是一个类型，不是 `Vec<EditorSelection> + primary`
//!
//! 三条理由，每一条都能指到别处的代码：
//!
//! 1. **不变式有一个真正的执法点。** `yu_text` 的 `validate_edits` 不但拒绝
//!    重叠的 edit，还拒绝两个**空** edit 落在同一个偏移。两个相邻光标各按一次
//!    退格就会撞到同一点——「互不重叠」因此不是洁癖，是一条不满足就直接
//!    `EditError::OverlappingEdits` 的前置条件。裸 `Vec` 会把「谁负责合并」
//!    散到 `EditorDocument` 里那几十处赋值上，每一处都要记得合并一次。
//! 2. **`revision` 不能有 N 份。** [`EditorSelection`] 自带 `revision`，一个
//!    `Vec<EditorSelection>` 就是 N 份必须相等的值，也就是 N−1 个可以对不上的
//!    机会。这里只存一份，构造时校验全体一致。
//! 3. **映射之后必须合并，而合并只能有一个地方。** `ChangeSet::map_anchor`
//!    逐个偏移独立映射，两个不同的偏移**可以映射到同一个偏移**（删掉两个光标
//!    之间的文字）。[`Selections::map_through`] 是全仓唯一的收敛点。
//!
//! # 不变式
//!
//! - **至少一个**：没有空构造，`primary()` 永远给得出答案。
//! - **同一个 revision**：全体与 [`Selections::revision`] 相等。
//! - **有序**：按 `ordered_range().start()` 升序。
//! - **互不重叠**：相邻两条满足 `prev.end() <= next.start()`，且这个等号成立时
//!   两条都必须非空。
//! - **primary 在界内**。
//!
//! 最后那条比 `validate_edits` 严格一点，这是有意的：一个光标停在一段选区的
//! 边界上，用户看到的是「选区边上多了一根竖线」，而打字的结果是同一个位置被
//! 插了两次。相邻的两段**非空**选区则必须留着——`aa` 在 `aaaa` 里的两处匹配
//! 就是 `0..2` 与 `2..4`，把它们并掉等于把「全部选中」变成「全选」。
//!
//! # 这里为什么没有 `preferred_x`
//!
//! 纵向移动的粘滞列是一个 f32 视觉列。这个 crate 的模块文档（`lib.rs`）把
//! 边界画在「依赖只有 `yu-core` 与 `yu-text`，一个布局或投影类型都没有」，
//! 所以它留在 `yu-editor`。代价是多光标纵向移动不吃粘滞列，理由与还债条件
//! 写在 `EditorDocument::move_vertical_with_loader` 上。

use yu_core::{ByteOffset, Revision};
use yu_text::{ChangeSet, TextSnapshot};

use crate::selection::{EditorSelection, SelectionError};

/// 一份 revision 上的一组选区，外加「哪一条是主选区」。
///
/// 不变式见模块文档。构造只有 [`Selections::single`] 与
/// [`Selections::new`] 两个入口，两者都归一化。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selections {
    revision: Revision,
    ranges: Vec<EditorSelection>,
    primary: usize,
}

impl Selections {
    /// 一条选区的那一份。多光标之前的全部行为都从这里过。
    #[must_use]
    pub fn single(selection: EditorSelection) -> Self {
        Self {
            revision: selection.revision(),
            ranges: vec![selection],
            primary: 0,
        }
    }

    /// 归一化一组选区：排序、合并、定位 primary。
    ///
    /// `primary` 是输入里哪一条是主选区的下标；它被并进别的选区时，合并出来的
    /// 那一条继承这个身份。空输入是错误——[`Selections`] 永远至少有一条。
    ///
    /// # Errors
    ///
    /// 输入为空、某一条不属于 `snapshot` 的 revision、`primary` 越界，或者
    /// 合并出来的端点在这一版源码上不合法。
    pub fn new(
        snapshot: &TextSnapshot,
        ranges: impl IntoIterator<Item = EditorSelection>,
        primary: usize,
    ) -> Result<Self, SelectionError> {
        let ranges: Vec<_> = ranges.into_iter().collect();
        if ranges.is_empty() || primary >= ranges.len() {
            return Err(SelectionError::InvalidRange);
        }
        for selection in &ranges {
            if selection.revision() != snapshot.revision() {
                return Err(SelectionError::StaleRevision {
                    expected: snapshot.revision(),
                    actual: selection.revision(),
                });
            }
            selection.utf16_range(snapshot)?;
        }
        Self::normalize(snapshot, ranges, primary)
    }

    /// 排序 + 合并。`primary` 是输入下标，返回的是归一化之后的那一份。
    fn normalize(
        snapshot: &TextSnapshot,
        ranges: Vec<EditorSelection>,
        primary: usize,
    ) -> Result<Self, SelectionError> {
        // 带着「我是不是 primary」一起排：合并之后 primary 的身份要跟到并出来
        // 的那一条上，按下标记会在合并里失效。
        let mut tagged: Vec<(EditorSelection, bool)> = ranges
            .into_iter()
            .enumerate()
            .map(|(index, selection)| (selection, index == primary))
            .collect();
        tagged.sort_by_key(|(selection, _)| {
            let range = selection.ordered_range();
            (range.start(), range.end())
        });

        let mut merged: Vec<(EditorSelection, bool)> = Vec::with_capacity(tagged.len());
        for (selection, is_primary) in tagged {
            let Some((previous, previous_primary)) = merged.last().copied() else {
                merged.push((selection, is_primary));
                continue;
            };
            if !touches(previous, selection) {
                merged.push((selection, is_primary));
                continue;
            }
            // 并出来的那一条的方向与 affinity 归 primary；两条都不是 primary
            // 时归靠前的那一条。让方向由「用户最后动的那一根光标」说了算。
            let keep = if is_primary { selection } else { previous };
            let combined = merge(snapshot, previous, selection, keep)?;
            merged.pop();
            merged.push((combined, previous_primary || is_primary));
        }

        let primary = merged
            .iter()
            .position(|(_, is_primary)| *is_primary)
            .unwrap_or(0);
        Ok(Self {
            revision: snapshot.revision(),
            ranges: merged.into_iter().map(|(selection, _)| selection).collect(),
            primary,
        })
    }

    /// 把每一条映射过一次成功的编辑，然后重新归一化。
    ///
    /// **归一化不能省。** 两个不同的偏移可以映射到同一个偏移（删掉它们之间的
    /// 文字），映射之后不合并就会留下一对重叠的选区，而它们产出的下一个
    /// Transaction 会被 `validate_edits` 拒掉——用户看到的是「打字突然没反应」。
    ///
    /// # Errors
    ///
    /// 某一条映射失败，或者合并出来的端点不合法。
    pub fn map_through(
        &self,
        change_set: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Self, SelectionError> {
        let mut mapped = Vec::with_capacity(self.ranges.len());
        for selection in &self.ranges {
            mapped.push(selection.map_through(change_set, snapshot)?);
        }
        Self::normalize(snapshot, mapped, self.primary)
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// 主选区：光标移动、滚动、AX 的单数属性、「当前命中」全部跟着它。
    #[must_use]
    pub fn primary(&self) -> EditorSelection {
        self.ranges[self.primary]
    }

    #[must_use]
    pub const fn primary_index(&self) -> usize {
        self.primary
    }

    /// 全部选区，按文档顺序。
    #[must_use]
    pub fn as_slice(&self) -> &[EditorSelection] {
        &self.ranges
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// 恒为假——[`Selections`] 至少有一条。留着它是因为 clippy 见到 `len` 就要
    /// 找 `is_empty`，而写一个总是返回 `false` 的比给 `len` 加豁免更诚实。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// 是不是真的多光标。降级路径用它决定要不要走 primary 那条。
    #[must_use]
    pub fn is_multiple(&self) -> bool {
        self.ranges.len() > 1
    }

    /// 塌回一条：只留 primary。
    #[must_use]
    pub fn collapsed_to_primary(&self) -> Self {
        Self::single(self.primary())
    }
}

/// 两条选区要不要并成一条。
///
/// 规则：区间相交要并；只在一点上相接时，**其中一条是空的**才并。
///
/// 后半句是这条规则的全部重量所在。`0..2` 与 `2..4` 是「全部选中 `aa`」在
/// `aaaa` 上的正确答案，并掉就成了「全选」；而 `0..2` 与停在 2 的一个光标
/// 画出来是「选区边上多一根竖线」，打字会把同一个位置插两次。
fn touches(previous: EditorSelection, next: EditorSelection) -> bool {
    let previous = previous.ordered_range();
    let next = next.ordered_range();
    if next.start() < previous.end() {
        return true;
    }
    next.start() == previous.end() && (previous.is_empty() || next.is_empty())
}

fn merge(
    snapshot: &TextSnapshot,
    previous: EditorSelection,
    next: EditorSelection,
    keep: EditorSelection,
) -> Result<EditorSelection, SelectionError> {
    let previous = previous.ordered_range();
    let next = next.ordered_range();
    let start = ByteOffset::new(previous.start().get().min(next.start().get()));
    let end = ByteOffset::new(previous.end().get().max(next.end().get()));
    let backward = keep.anchor() > keep.focus();
    let (anchor, focus) = if backward { (end, start) } else { (start, end) };
    EditorSelection::range(snapshot, anchor, focus, keep.affinity())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::CaretAffinity;
    use yu_text::{Edit, TextBuffer, Transaction};

    fn caret(snapshot: &TextSnapshot, offset: u64) -> EditorSelection {
        EditorSelection::cursor(snapshot, ByteOffset::new(offset), CaretAffinity::Downstream)
            .expect("test caret should be valid")
    }

    fn span(snapshot: &TextSnapshot, anchor: u64, focus: u64) -> EditorSelection {
        EditorSelection::range(
            snapshot,
            ByteOffset::new(anchor),
            ByteOffset::new(focus),
            CaretAffinity::Downstream,
        )
        .expect("test selection should be valid")
    }

    fn starts(selections: &Selections) -> Vec<(u64, u64)> {
        selections
            .as_slice()
            .iter()
            .map(|selection| {
                let range = selection.ordered_range();
                (range.start().get(), range.end().get())
            })
            .collect()
    }

    /// 全部不变式的一次检查。性质用例都以它收尾，因为「有序」「不重叠」
    /// 「至少一个」「primary 在界内」漏掉任何一条都不报错。
    fn assert_invariants(selections: &Selections) {
        assert!(!selections.as_slice().is_empty(), "至少要有一条选区");
        assert!(
            selections.primary_index() < selections.len(),
            "primary 越界"
        );
        for selection in selections.as_slice() {
            assert_eq!(
                selection.revision(),
                selections.revision(),
                "选区与这一组的 revision 不一致"
            );
        }
        for pair in selections.as_slice().windows(2) {
            let previous = pair[0].ordered_range();
            let next = pair[1].ordered_range();
            assert!(
                previous.start() <= next.start(),
                "选区没有按起点排序：{previous:?} 在 {next:?} 之前"
            );
            assert!(
                previous.end() <= next.start(),
                "选区重叠：{previous:?} 与 {next:?}"
            );
            assert!(
                previous.end() != next.start() || (!previous.is_empty() && !next.is_empty()),
                "相接的两条里有空选区：{previous:?} 与 {next:?}"
            );
        }
    }

    #[test]
    fn a_single_selection_is_its_own_primary() {
        let buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let selections = Selections::single(caret(&snapshot, 2));
        assert_eq!(selections.len(), 1);
        assert!(!selections.is_multiple());
        assert_eq!(selections.primary(), caret(&snapshot, 2));
        assert_invariants(&selections);
    }

    #[test]
    fn out_of_order_input_comes_back_sorted() {
        let buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let selections = Selections::new(
            &snapshot,
            [
                caret(&snapshot, 4),
                caret(&snapshot, 1),
                caret(&snapshot, 2),
            ],
            0,
        )
        .expect("carets should normalize");
        assert_eq!(starts(&selections), vec![(1, 1), (2, 2), (4, 4)]);
        // primary 是输入里的第 0 条（偏移 4），排序之后它在末尾。
        assert_eq!(selections.primary().focus(), ByteOffset::new(4));
        assert_invariants(&selections);
    }

    #[test]
    fn two_carets_on_one_offset_merge() {
        let buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let selections = Selections::new(&snapshot, [caret(&snapshot, 3), caret(&snapshot, 3)], 0)
            .expect("duplicate carets should merge");
        assert_eq!(starts(&selections), vec![(3, 3)]);
        assert_invariants(&selections);
    }

    /// 这一条是 `validate_edits` 的 `duplicate_empty` 在选区层的对应物：
    /// 不合并，下一次插入就是 `EditError::OverlappingEdits`。
    #[test]
    fn overlapping_spans_merge_into_their_union() {
        let buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let selections =
            Selections::new(&snapshot, [span(&snapshot, 0, 3), span(&snapshot, 2, 5)], 0)
                .expect("overlapping spans should merge");
        assert_eq!(starts(&selections), vec![(0, 5)]);
        assert_invariants(&selections);
    }

    /// **相邻的非空选区必须留着。** `aa` 在 `aaaa` 里的两处匹配就是这个形状，
    /// 并掉等于把「选中全部匹配」变成「全选」。
    #[test]
    fn adjacent_non_empty_spans_stay_separate() {
        let buffer = TextBuffer::new("aaaa");
        let snapshot = buffer.snapshot();
        let selections =
            Selections::new(&snapshot, [span(&snapshot, 0, 2), span(&snapshot, 2, 4)], 0)
                .expect("adjacent spans should survive");
        assert_eq!(starts(&selections), vec![(0, 2), (2, 4)]);
        assert_invariants(&selections);
    }

    /// 而一个停在选区边界上的**空**光标要并进去：画出来是「选区边上多一根
    /// 竖线」，打字会把同一个位置插两次。
    #[test]
    fn a_caret_on_a_span_boundary_merges_into_it() {
        let buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let trailing = Selections::new(&snapshot, [span(&snapshot, 1, 3), caret(&snapshot, 3)], 0)
            .expect("caret at the end should merge");
        assert_eq!(starts(&trailing), vec![(1, 3)]);
        let leading = Selections::new(&snapshot, [caret(&snapshot, 1), span(&snapshot, 1, 3)], 0)
            .expect("caret at the start should merge");
        assert_eq!(starts(&leading), vec![(1, 3)]);
        assert_invariants(&trailing);
        assert_invariants(&leading);
    }

    #[test]
    fn merging_keeps_the_primary_identity_and_its_direction() {
        let buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        // 输入的第 1 条（反向的 5→2）是 primary，它与第 0 条重叠。
        let selections =
            Selections::new(&snapshot, [span(&snapshot, 0, 3), span(&snapshot, 5, 2)], 1)
                .expect("overlapping spans should merge");
        assert_eq!(starts(&selections), vec![(0, 5)]);
        assert_eq!(selections.primary_index(), 0);
        // primary 反向，所以并出来的那一条也反向：anchor 在末尾。
        assert_eq!(selections.primary().anchor(), ByteOffset::new(5));
        assert_eq!(selections.primary().focus(), ByteOffset::new(0));
        assert_invariants(&selections);
    }

    #[test]
    fn an_empty_set_is_rejected() {
        let buffer = TextBuffer::new("abc");
        let snapshot = buffer.snapshot();
        assert!(matches!(
            Selections::new(&snapshot, [], 0),
            Err(SelectionError::InvalidRange)
        ));
    }

    #[test]
    fn an_out_of_bounds_primary_is_rejected() {
        let buffer = TextBuffer::new("abc");
        let snapshot = buffer.snapshot();
        assert!(matches!(
            Selections::new(&snapshot, [caret(&snapshot, 0)], 1),
            Err(SelectionError::InvalidRange)
        ));
    }

    #[test]
    fn a_selection_from_another_revision_is_rejected() {
        let buffer = TextBuffer::new("abc");
        let stale = caret(&buffer.snapshot(), 1);
        let mut next = TextBuffer::new("abc");
        next.apply(&Transaction::new(
            next.revision(),
            [Edit::new(
                yu_core::TextRange::new(ByteOffset::new(0), ByteOffset::new(0))
                    .expect("empty range"),
                "z",
            )],
        ))
        .expect("edit should apply");
        assert!(matches!(
            Selections::new(&next.snapshot(), [stale], 0),
            Err(SelectionError::StaleRevision { .. })
        ));
    }

    /// **删掉两个光标之间的字，它们会落到同一个偏移。** 映射之后不归一化就会
    /// 留下一对重叠的选区，下一次插入被 `validate_edits` 拒掉——用户看到的是
    /// 「打字突然没反应」。
    #[test]
    fn carets_that_collide_after_an_edit_merge() {
        let mut buffer = TextBuffer::new("a-b");
        let snapshot = buffer.snapshot();
        let selections = Selections::new(&snapshot, [caret(&snapshot, 1), caret(&snapshot, 2)], 0)
            .expect("two carets should normalize");
        assert_eq!(selections.len(), 2);

        let applied = buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(
                    yu_core::TextRange::new(ByteOffset::new(1), ByteOffset::new(2))
                        .expect("ordered"),
                    "",
                )],
            ))
            .expect("deletion should apply");
        let mapped = selections
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("selections should map");

        assert_eq!(starts(&mapped), vec![(1, 1)]);
        assert_invariants(&mapped);
    }

    #[test]
    fn mapping_keeps_distinct_carets_distinct_and_shifts_them() {
        let mut buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let selections = Selections::new(&snapshot, [caret(&snapshot, 1), caret(&snapshot, 4)], 1)
            .expect("two carets should normalize");
        let applied = buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(
                    yu_core::TextRange::new(ByteOffset::new(0), ByteOffset::new(0))
                        .expect("empty range"),
                    "XY",
                )],
            ))
            .expect("insert should apply");
        let mapped = selections
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("selections should map");

        assert_eq!(starts(&mapped), vec![(3, 3), (6, 6)]);
        assert_eq!(mapped.primary().focus(), ByteOffset::new(6));
        assert_invariants(&mapped);
    }

    #[test]
    fn collapsing_keeps_only_the_primary() {
        let buffer = TextBuffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let selections = Selections::new(
            &snapshot,
            [
                caret(&snapshot, 1),
                caret(&snapshot, 3),
                caret(&snapshot, 5),
            ],
            2,
        )
        .expect("carets should normalize");
        let collapsed = selections.collapsed_to_primary();
        assert_eq!(starts(&collapsed), vec![(5, 5)]);
        assert_invariants(&collapsed);
    }

    /// 多字节文本上的归一化：偏移必须落在字符边界，合并不能把它们算错。
    #[test]
    fn normalization_holds_on_multibyte_text() {
        let buffer = TextBuffer::new("a🙂b🙂c");
        let snapshot = buffer.snapshot();
        let selections = Selections::new(
            &snapshot,
            [
                span(&snapshot, 6, 10),
                span(&snapshot, 1, 5),
                caret(&snapshot, 5),
            ],
            0,
        )
        .expect("carets should normalize");
        // `caret(5)` 停在 `1..5` 的末尾上，要并进去；`6..10` 与它不相接。
        assert_eq!(starts(&selections), vec![(1, 5), (6, 10)]);
        assert_invariants(&selections);
    }
}
