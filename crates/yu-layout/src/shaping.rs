use std::fmt;

use yu_core::TextRange;
use yu_projection::VisualRunStyle;

/// Stable face identity carried by shaped glyph runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFaceId(u32);

impl FontFaceId {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }
}

/// Stable glyph identity within a font face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(u32);

impl GlyphId {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }
}

/// Direction passed through the shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextDirection {
    Ltr,
    Rtl,
}

/// Script hint passed through the shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    Common,
    Latin,
    Han,
    Japanese,
    Arabic,
    Devanagari,
    Unknown,
}

/// One positioned glyph with a source cluster range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    id: GlyphId,
    source: TextRange,
    advance: f32,
    x_offset: f32,
    y_offset: f32,
}

impl Glyph {
    #[must_use]
    pub const fn new(
        id: GlyphId,
        source: TextRange,
        advance: f32,
        x_offset: f32,
        y_offset: f32,
    ) -> Self {
        Self {
            id,
            source,
            advance,
            x_offset,
            y_offset,
        }
    }

    #[must_use]
    pub const fn id(self) -> GlyphId {
        self.id
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn advance(self) -> f32 {
        self.advance
    }

    #[must_use]
    pub const fn x_offset(self) -> f32 {
        self.x_offset
    }

    #[must_use]
    pub const fn y_offset(self) -> f32 {
        self.y_offset
    }
}

/// A same-face shaped run. Source ranges are ordered and may span multiple
/// Unicode code points when a shaping engine forms a ligature or cluster.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    face: FontFaceId,
    source: TextRange,
    style: VisualRunStyle,
    direction: TextDirection,
    script: Script,
    glyphs: Vec<Glyph>,
    advance: f32,
}

impl GlyphRun {
    #[must_use]
    pub fn new(
        face: FontFaceId,
        source: TextRange,
        style: VisualRunStyle,
        direction: TextDirection,
        script: Script,
        glyphs: Vec<Glyph>,
    ) -> Self {
        let advance = glyphs.iter().map(|glyph| glyph.advance()).sum();
        Self {
            face,
            source,
            style,
            direction,
            script,
            glyphs,
            advance,
        }
    }

    #[must_use]
    pub const fn face(&self) -> FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn style(&self) -> VisualRunStyle {
        self.style
    }

    #[must_use]
    pub const fn direction(&self) -> TextDirection {
        self.direction
    }

    #[must_use]
    pub const fn script(&self) -> Script {
        self.script
    }

    #[must_use]
    pub fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    #[must_use]
    pub const fn advance(&self) -> f32 {
        self.advance
    }
}

/// Shaped output potentially split into fallback-face runs.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedText {
    source: TextRange,
    runs: Vec<GlyphRun>,
    advance: f32,
}

impl ShapedText {
    #[must_use]
    pub fn new(source: TextRange, runs: Vec<GlyphRun>) -> Self {
        let advance = runs.iter().map(GlyphRun::advance).sum();
        Self {
            source,
            runs,
            advance,
        }
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub fn runs(&self) -> &[GlyphRun] {
        &self.runs
    }

    #[must_use]
    pub const fn advance(&self) -> f32 {
        self.advance
    }
}

/// A source-backed shaping provider consumed by `LayoutSnapshot`.
pub trait ShapingProvider {
    type Error: fmt::Display;

    fn shape(
        &self,
        text: &str,
        source: TextRange,
        style: VisualRunStyle,
    ) -> Result<ShapedText, Self::Error>;
}
