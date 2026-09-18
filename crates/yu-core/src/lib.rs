#![forbid(unsafe_code)]

//! Stable foundational types shared by Yu Editor's core crates.

mod geometry;
mod paragraph;
mod position;
mod shaping;
pub use paragraph::*;
pub mod shaping_conformance;
mod style;

pub use geometry::{
    Block, BlockInk, CoordinateSpace, Device, Document, GeometryError, Point, Rect, Scale, Size,
};
pub use position::{
    Affinity, ByteOffset, CaretAffinity, LineIndex, NativeCaretPosition, Revision,
    SourceCaretPosition, TextAnchor, TextRange, Utf16Offset, Utf16Range, VisualOffset, VisualRange,
};
pub use shaping::{
    ClusterMetrics, FontFaceId, Glyph, GlyphId, GlyphRun, Script, ShapedText, ShapingProvider,
    TextDirection,
};
pub use style::{LineStyleId, StyleId, TextAttrs, TextRole, TextStyle, WidgetId, WidgetSide};

mod theme;
pub use theme::{ReadingGeometry, TaskCheckboxStyle, ThemeFont, ThemeId, ThemeSpec};
