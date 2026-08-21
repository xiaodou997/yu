#![forbid(unsafe_code)]

//! Stable foundational types shared by Yu Editor's core crates.

mod geometry;
mod position;
mod shaping;
mod style;

pub use geometry::{
    Block, CoordinateSpace, Device, Document, GeometryError, Point, Rect, Scale, Size,
};
pub use position::{
    Affinity, ByteOffset, LineIndex, Revision, TextAnchor, TextRange, Utf16Offset, Utf16Range,
    VisualOffset, VisualRange,
};
pub use shaping::{
    ClusterMetrics, FontFaceId, Glyph, GlyphId, GlyphRun, Script, ShapedText, ShapingProvider,
    TextDirection,
};
pub use style::TextStyle;
