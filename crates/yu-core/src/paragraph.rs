//! Platform paragraph layout. Glyphs and editing boundaries are independent:
//! a grapheme may produce several glyphs and a ligature may cover several
//! graphemes. All ranges below refer to the input's UTF-8 visual text.

use crate::{
    FontFaceId, GlyphId, StyleId, TextAttrs, VisualOffset, VisualRange, WidgetId, WidgetSide,
};

/// Paragraph embedding direction; does not override the direction of individual runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BaseDirection {
    /// Infer from the first strong directional character (UAX #9 P2/P3).
    #[default]
    Auto,
    Ltr,
    Rtl,
}

/// Logical font boxes used to position a paragraph's baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontStrutMode {
    #[default]
    RunMetrics,
    /// Keep the paragraph's base font baseline without enlarging the line.
    BaseFont,
    /// Union the base and styled fonts' inherited absolute-height boxes.
    AllFonts,
    /// Union declared run fonts without the unstyled paragraph base or fallback glyph fonts.
    DeclaredFonts,
    /// Fixed code-font boxes, including the local Courier ascent normalization.
    /// Fallback glyph fonts do not move the logical baseline.
    CodeFont,
    /// Union base and declared fonts with line height relative to each font's
    /// size. Local Courier uses its normalized logical ascent.
    RelativeFonts,
}

#[derive(Clone, Copy, Debug)]
pub struct ParagraphRun {
    pub range: VisualRange,
    pub style: StyleId,
    pub attrs: TextAttrs,
}

#[derive(Clone, Copy, Debug)]
pub struct ParagraphObject {
    pub offset: VisualOffset,
    pub id: WidgetId,
    pub side: WidgetSide,
    pub size: crate::Size<crate::Block>,
    pub baseline: f32,
    pub ready: bool,
}

#[derive(Clone, Debug)]
pub struct ParagraphInput<'a> {
    pub text: &'a str,
    pub base_direction: BaseDirection,
    /// Expand soft-wrapped lines to the available width; final/hard-break lines stay natural.
    pub justify: bool,
    pub runs: Vec<ParagraphRun>,
    pub objects: Vec<ParagraphObject>,
    pub width: f32,
    pub indent: f32,
    /// Absolute logical line height, before inline-object or requested font-strut expansion.
    pub line_height: f32,
    /// Align the primary fonts' inherited absolute-height struts on one
    /// baseline. Fallback glyph ink does not change these logical boxes.
    pub font_strut_mode: FontStrutMode,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphLine {
    pub range: VisualRange,
    pub bounds: crate::Rect<crate::Block>,
    pub baseline: f32,
    /// Actual maximum font ascent, excluding half-leading and inline objects.
    pub font_ascent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphCluster {
    pub range: VisualRange,
    pub style: StyleId,
    pub line: usize,
    pub leading: f32,
    pub trailing: f32,
    pub line_break: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphGlyph {
    pub range: VisualRange,
    pub style: StyleId,
    pub line: usize,
    pub face: FontFaceId,
    pub glyph: GlyphId,
    pub x: f32,
    pub y: f32,
    pub size_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphObjectBox {
    pub index: usize,
    pub line: usize,
    pub x: f32,
    pub y: f32,
}

/// A source-neutral inline box fragment, measured with its paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineBoxFragment {
    pub id: u64,
    pub range: VisualRange,
    pub bounds: crate::Rect<crate::BlockInk>,
    pub left_edge: bool,
    pub right_edge: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphLayout {
    pub inline_boxes: Vec<InlineBoxFragment>,
    pub lines: Vec<ParagraphLine>,
    pub clusters: Vec<ParagraphCluster>,
    pub glyphs: Vec<ParagraphGlyph>,
    pub objects: Vec<ParagraphObjectBox>,
}

/// Produces final paragraph geometry, including native bidi and line breaking.
/// Native layout objects must not escape through this owned result.
pub trait ParagraphLayoutProvider {
    fn layout(&self, input: &ParagraphInput<'_>) -> Result<ParagraphLayout, String>;
}
