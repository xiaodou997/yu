//! 跨层的两个小工具。
//!
//! 它们跟着表格几何一起从 `yu-layout` 搬过来，都不认识 Markdown，但只有
//! 认识 Markdown 的那些几何还需要它们。

use std::fmt;

use yu_core::TextRange;
use yu_layout::LayoutError;

/// 把上一层的错误按 `Display` 搬进 [`LayoutError`]。
///
/// `yu-layout` 在依赖图的下面（不变量 E2），它不认识 `VisualTextError`
/// 这种类型。跨层的错误只能带走说明文字——与 `LayoutError::Shaping` 是
/// 同一个理由，够定位，不够反解。
pub(crate) fn upstream(error: impl fmt::Display) -> LayoutError {
    LayoutError::Upstream(error.to_string())
}

pub(crate) const fn source_range_contains(outer: TextRange, inner: TextRange) -> bool {
    outer.start().get() <= inner.start().get() && inner.end().get() <= outer.end().get()
}
