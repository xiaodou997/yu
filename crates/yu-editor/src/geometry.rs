//! 源码区间的两个小工具。
//!
//! 它们跟着表格几何一起从 `yu-layout` 搬过来。`map_source_range` 是
//! 「一段源码区间穿过一次编辑之后还在哪」，`source_range_contains` 是
//! 「这段在不在那段里」——都不认识 Markdown，但只有认识 Markdown 的那些
//! 几何还需要它们。

use std::fmt;

use yu_core::{Affinity, TextAnchor, TextRange};
use yu_layout::LayoutError;
use yu_text::ChangeSet;

/// 把上一层的错误按 `Display` 搬进 [`LayoutError`]。
///
/// `yu-layout` 在依赖图的下面（不变量 E2），它不认识 `ProjectionError`
/// 这种类型。跨层的错误只能带走说明文字——与 `LayoutError::Shaping` 是
/// 同一个理由，够定位，不够反解。
pub(crate) fn upstream(error: impl fmt::Display) -> LayoutError {
    LayoutError::Upstream(error.to_string())
}

pub(crate) const fn source_range_contains(outer: TextRange, inner: TextRange) -> bool {
    outer.start().get() <= inner.start().get() && inner.end().get() <= outer.end().get()
}

pub(crate) fn map_source_range(
    range: TextRange,
    changes: &ChangeSet,
) -> Result<TextRange, LayoutError> {
    let start = changes
        .map_anchor(TextAnchor::new(
            changes.before(),
            range.start(),
            Affinity::Before,
        ))
        .map_err(upstream)?
        .offset();
    let end = changes
        .map_anchor(TextAnchor::new(
            changes.before(),
            range.end(),
            Affinity::After,
        ))
        .map_err(upstream)?
        .offset();
    TextRange::new(start, end).ok_or(LayoutError::OffsetOverflow)
}
