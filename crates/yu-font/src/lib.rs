#![forbid(unsafe_code)]

//! Platform-independent font selection and shaping contracts.
//!
//! This crate intentionally does not open font files or call CoreText,
//! DirectWrite, or Fontconfig. It defines the data that those platform
//! backends must produce, including owned font metrics, glyph bitmaps and CPU
//! atlas placements, and ships a deterministic shaper for contract tests.
//! [`FontMetrics`] implements `yu-layout::ClusterMetrics`, while [`FontShaper`]
//! implements `yu-layout::ShapingProvider`, so the layout engine can consume
//! the same fallback policy before a native glyph backend exists.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;
use yu_core::{ByteOffset, TextRange};
use yu_layout::ClusterMetrics;
use yu_projection::VisualRunStyle;

mod raster;

pub use raster::{
    AtlasEntry, AtlasError, AtlasRect, FontMetricKey, FontMetricsCache, FontMetricsSnapshot,
    GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphMetrics, GlyphRasterKey, GlyphRasterizer,
    RasterDataError, RasterizedGlyph,
};
pub use yu_layout::{
    FontFaceId, Glyph, GlyphId, GlyphRun, Script, ShapedText, ShapingProvider, TextDirection,
};

/// Coarse weight requests used by fallback selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontWeight {
    Normal,
    Medium,
    Bold,
}

impl FontWeight {
    fn distance(self, other: Self) -> u32 {
        let left: u32 = match self {
            Self::Normal => 400,
            Self::Medium => 500,
            Self::Bold => 700,
        };
        let right: u32 = match other {
            Self::Normal => 400,
            Self::Medium => 500,
            Self::Bold => 700,
        };
        left.abs_diff(right)
    }
}

/// Font slant request used by fallback selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontSlant {
    Upright,
    Italic,
    Oblique,
}

/// An inclusive Unicode scalar range supported by a face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnicodeRange {
    start: u32,
    end: u32,
}

impl UnicodeRange {
    #[must_use]
    pub const fn new(start: char, end: char) -> Option<Self> {
        let start = start as u32;
        let end = end as u32;
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn start(self) -> char {
        // The constructor only accepts valid scalar values, so these values
        // are guaranteed to round-trip through char.
        match char::from_u32(self.start) {
            Some(value) => value,
            None => '\u{FFFD}',
        }
    }

    #[must_use]
    pub const fn end(self) -> char {
        match char::from_u32(self.end) {
            Some(value) => value,
            None => '\u{FFFD}',
        }
    }

    #[must_use]
    pub const fn contains(self, character: char) -> bool {
        let value = character as u32;
        self.start <= value && value <= self.end
    }
}

/// Declarative coverage used by the contract backend and future font loaders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontCoverage {
    All,
    Ranges(Vec<UnicodeRange>),
}

impl FontCoverage {
    #[must_use]
    pub fn supports(&self, text: &str) -> bool {
        match self {
            Self::All => true,
            Self::Ranges(ranges) => text
                .chars()
                .all(|character| ranges.iter().any(|range| range.contains(character))),
        }
    }
}

/// Input specification for one registered face.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFaceSpec {
    family: Arc<str>,
    postscript_name: Arc<str>,
    weight: FontWeight,
    slant: FontSlant,
    coverage: FontCoverage,
    /// Advance measured as an em fraction; `0.5` is half the requested size.
    nominal_advance: f32,
}

impl FontFaceSpec {
    pub fn new(family: impl Into<Arc<str>>, nominal_advance: f32) -> Self {
        let family = family.into();
        Self {
            postscript_name: Arc::clone(&family),
            family,
            weight: FontWeight::Normal,
            slant: FontSlant::Upright,
            coverage: FontCoverage::All,
            nominal_advance,
        }
    }

    #[must_use]
    pub fn with_postscript_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.postscript_name = name.into();
        self
    }

    #[must_use]
    pub const fn with_weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    #[must_use]
    pub const fn with_slant(mut self, slant: FontSlant) -> Self {
        self.slant = slant;
        self
    }

    #[must_use]
    pub fn with_coverage(mut self, coverage: FontCoverage) -> Self {
        self.coverage = coverage;
        self
    }
}

/// A validated face in a [`FontDatabase`].
#[derive(Clone, Debug, PartialEq)]
pub struct FontFace {
    id: FontFaceId,
    spec: FontFaceSpec,
}

impl FontFace {
    #[must_use]
    pub const fn id(&self) -> FontFaceId {
        self.id
    }

    #[must_use]
    pub fn family(&self) -> &str {
        &self.spec.family
    }

    #[must_use]
    pub fn postscript_name(&self) -> &str {
        &self.spec.postscript_name
    }

    #[must_use]
    pub const fn weight(&self) -> FontWeight {
        self.spec.weight
    }

    #[must_use]
    pub const fn slant(&self) -> FontSlant {
        self.spec.slant
    }

    #[must_use]
    pub const fn nominal_advance(&self) -> f32 {
        self.spec.nominal_advance
    }

    #[must_use]
    pub fn coverage(&self) -> &FontCoverage {
        &self.spec.coverage
    }
}

/// Registered faces and their deterministic fallback order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontDatabase {
    faces: Vec<FontFace>,
}

impl FontDatabase {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, spec: FontFaceSpec) -> Result<FontFaceId, FontError> {
        if spec.family.trim().is_empty() {
            return Err(FontError::InvalidFamily);
        }
        if !spec.nominal_advance.is_finite() || spec.nominal_advance <= 0.0 {
            return Err(FontError::InvalidAdvance(spec.nominal_advance.to_bits()));
        }
        let id = FontFaceId::from_raw(
            u32::try_from(self.faces.len()).map_err(|_| FontError::FaceIdOverflow)?,
        );
        self.faces.push(FontFace { id, spec });
        Ok(id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    #[must_use]
    pub fn face(&self, id: FontFaceId) -> Option<&FontFace> {
        self.faces.get(usize::try_from(id.get()).ok()?)
    }

    /// Selects the closest family/weight/slant face that covers `text`.
    #[must_use]
    pub fn resolve(&self, request: &FontRequest, text: &str) -> Option<FontFaceId> {
        self.faces
            .iter()
            .filter(|face| face.coverage().supports(text))
            .min_by_key(|face| {
                let family_penalty = u8::from(face.family() != request.family());
                let slant_penalty = u8::from(face.slant() != request.slant());
                (
                    family_penalty,
                    face.weight().distance(request.weight()),
                    slant_penalty,
                    face.id().get(),
                )
            })
            .map(FontFace::id)
    }
}

/// Font request passed to fallback and shaping.
#[derive(Clone, Debug, PartialEq)]
pub struct FontRequest {
    family: Arc<str>,
    size: f32,
    weight: FontWeight,
    slant: FontSlant,
}

impl FontRequest {
    pub fn new(family: impl Into<Arc<str>>, size: f32) -> Result<Self, FontError> {
        let family = family.into();
        if family.trim().is_empty() {
            return Err(FontError::InvalidFamily);
        }
        if !size.is_finite() || size <= 0.0 {
            return Err(FontError::InvalidSize(size.to_bits()));
        }
        Ok(Self {
            family,
            size,
            weight: FontWeight::Normal,
            slant: FontSlant::Upright,
        })
    }

    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }

    #[must_use]
    pub const fn size(&self) -> f32 {
        self.size
    }

    #[must_use]
    pub const fn weight(&self) -> FontWeight {
        self.weight
    }

    #[must_use]
    pub const fn slant(&self) -> FontSlant {
        self.slant
    }

    #[must_use]
    pub const fn with_weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    #[must_use]
    pub const fn with_slant(mut self, slant: FontSlant) -> Self {
        self.slant = slant;
        self
    }

    fn for_style(&self, style: VisualRunStyle) -> Self {
        match style {
            VisualRunStyle::Strong => self.clone().with_weight(FontWeight::Bold),
            VisualRunStyle::Emphasis => self.clone().with_slant(FontSlant::Italic),
            VisualRunStyle::Plain | VisualRunStyle::Code => self.clone(),
        }
    }
}

/// Input to a shaping backend.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeRequest<'a> {
    text: &'a str,
    source: TextRange,
    style: VisualRunStyle,
    font: FontRequest,
    direction: TextDirection,
    script: Script,
}

impl<'a> ShapeRequest<'a> {
    pub fn new(
        text: &'a str,
        source: TextRange,
        style: VisualRunStyle,
        font: FontRequest,
    ) -> Result<Self, ShapeError> {
        if source.len() != u64::try_from(text.len()).map_err(|_| ShapeError::OffsetOverflow)? {
            return Err(ShapeError::SourceLengthMismatch {
                source: source.len(),
                text: text.len(),
            });
        }
        Ok(Self {
            text,
            source,
            style,
            font,
            direction: TextDirection::Ltr,
            script: Script::Unknown,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        self.text
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
    pub fn font(&self) -> &FontRequest {
        &self.font
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
    pub const fn with_direction(mut self, direction: TextDirection) -> Self {
        self.direction = direction;
        self
    }

    #[must_use]
    pub const fn with_script(mut self, script: Script) -> Self {
        self.script = script;
        self
    }
}

/// Errors that a real shaping backend must report rather than fabricating data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeError {
    EmptyFontDatabase,
    MissingGlyph(Arc<str>),
    Backend(Arc<str>),
    OffsetOverflow,
    SourceLengthMismatch { source: u64, text: usize },
}

impl fmt::Display for ShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFontDatabase => formatter.write_str("font database is empty"),
            Self::MissingGlyph(cluster) => write!(formatter, "no fallback face covers {cluster:?}"),
            Self::Backend(message) => write!(formatter, "native shaping backend failed: {message}"),
            Self::OffsetOverflow => formatter.write_str("shaping source offset overflowed"),
            Self::SourceLengthMismatch { source, text } => {
                write!(
                    formatter,
                    "source range is {source} bytes but text is {text} bytes"
                )
            }
        }
    }
}

impl Error for ShapeError {}

/// Errors while registering or requesting a face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontError {
    FaceIdOverflow,
    InvalidAdvance(u32),
    InvalidFamily,
    InvalidSize(u32),
}

impl fmt::Display for FontError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FaceIdOverflow => formatter.write_str("font face id overflowed"),
            Self::InvalidAdvance(advance) => {
                write!(
                    formatter,
                    "invalid nominal advance {}",
                    f32::from_bits(*advance)
                )
            }
            Self::InvalidFamily => formatter.write_str("font family must not be empty"),
            Self::InvalidSize(size) => {
                write!(formatter, "invalid font size {}", f32::from_bits(*size))
            }
        }
    }
}

impl Error for FontError {}

/// A shaping backend. Native CoreText/DirectWrite/Fontconfig implementations
/// can replace [`MockShaper`] without changing layout-facing data types.
pub trait TextShaper: Send + Sync {
    fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedText, ShapeError>;
}

/// Deterministic one-glyph-per-grapheme shaper used before native shaping.
#[derive(Clone, Debug)]
pub struct MockShaper {
    database: Arc<FontDatabase>,
}

impl MockShaper {
    pub fn new(database: Arc<FontDatabase>) -> Result<Self, ShapeError> {
        if database.is_empty() {
            return Err(ShapeError::EmptyFontDatabase);
        }
        Ok(Self { database })
    }

    #[must_use]
    pub fn database(&self) -> &Arc<FontDatabase> {
        &self.database
    }
}

impl TextShaper for MockShaper {
    fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedText, ShapeError> {
        if request.text.is_empty() {
            return Ok(ShapedText::new(request.source, Vec::new()));
        }
        let styled_font = request.font.for_style(request.style);
        let mut runs = Vec::new();
        let mut current_face = None;
        let mut current_glyphs = Vec::new();
        let mut current_source_start = request.source.start();
        let mut current_source_end = current_source_start;
        let mut current_advance = 0.0_f32;
        let mut total_advance = 0.0_f32;

        for (local_start, cluster) in request.text.grapheme_indices(true) {
            let local_end = local_start
                .checked_add(cluster.len())
                .ok_or(ShapeError::OffsetOverflow)?;
            let source_start = add_offset(request.source.start(), local_start)?;
            let source_end = add_offset(request.source.start(), local_end)?;
            let face_id = self
                .database
                .resolve(&styled_font, cluster)
                .ok_or_else(|| ShapeError::MissingGlyph(Arc::from(cluster)))?;
            let face = self
                .database
                .face(face_id)
                .ok_or(ShapeError::MissingGlyph(Arc::from(cluster)))?;
            if current_face != Some(face_id) && !current_glyphs.is_empty() {
                let face = current_face.ok_or(ShapeError::OffsetOverflow)?;
                runs.push(GlyphRun::new(
                    face,
                    TextRange::new(current_source_start, current_source_end)
                        .ok_or(ShapeError::OffsetOverflow)?,
                    request.style,
                    request.direction,
                    request.script,
                    std::mem::take(&mut current_glyphs),
                ));
                total_advance += current_advance;
                current_advance = 0.0;
                current_source_start = source_start;
            }
            if current_glyphs.is_empty() {
                current_source_start = source_start;
            }
            current_face = Some(face_id);
            let advance = face.nominal_advance() * styled_font.size();
            current_glyphs.push(Glyph::new(
                GlyphId::from_raw(hash_cluster(cluster)),
                TextRange::new(source_start, source_end).ok_or(ShapeError::OffsetOverflow)?,
                advance,
                0.0,
                0.0,
            ));
            current_source_end = source_end;
            current_advance += advance;
        }

        if !current_glyphs.is_empty() {
            let face = current_face.ok_or(ShapeError::OffsetOverflow)?;
            runs.push(GlyphRun::new(
                face,
                TextRange::new(current_source_start, current_source_end)
                    .ok_or(ShapeError::OffsetOverflow)?,
                request.style,
                request.direction,
                request.script,
                current_glyphs,
            ));
            total_advance += current_advance;
        }

        debug_assert_eq!(
            total_advance,
            runs.iter().map(GlyphRun::advance).sum::<f32>()
        );
        Ok(ShapedText::new(request.source, runs))
    }
}

/// A layout-facing adapter that turns a font request into shaped glyph runs.
#[derive(Clone, Debug)]
pub struct FontShaper {
    backend: MockShaper,
    request: FontRequest,
}

impl FontShaper {
    pub fn new(database: Arc<FontDatabase>, request: FontRequest) -> Result<Self, ShapeError> {
        Ok(Self {
            backend: MockShaper::new(database)?,
            request,
        })
    }

    #[must_use]
    pub fn backend(&self) -> &MockShaper {
        &self.backend
    }

    #[must_use]
    pub fn request(&self) -> &FontRequest {
        &self.request
    }
}

impl TextShaper for FontShaper {
    fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedText, ShapeError> {
        self.backend.shape(request)
    }
}

impl ShapingProvider for FontShaper {
    type Error = ShapeError;

    fn shape(
        &self,
        text: &str,
        source: TextRange,
        style: VisualRunStyle,
    ) -> Result<ShapedText, Self::Error> {
        let request = ShapeRequest::new(text, source, style, self.request.clone())?;
        self.backend.shape(&request)
    }

    fn shape_scaled(
        &self,
        text: &str,
        source: TextRange,
        style: VisualRunStyle,
        scale: f32,
    ) -> Result<ShapedText, Self::Error> {
        let size = self.request.size * scale;
        let font = FontRequest::new(self.request.family.clone(), size)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))?
            .with_weight(self.request.weight)
            .with_slant(self.request.slant);
        let request = ShapeRequest::new(text, source, style, font)?;
        self.backend.shape(&request)
    }
}

/// A layout-facing advance provider backed by the same fallback database.
#[derive(Clone, Debug)]
pub struct FontMetrics {
    database: Arc<FontDatabase>,
    request: FontRequest,
}

impl FontMetrics {
    pub fn new(database: Arc<FontDatabase>, request: FontRequest) -> Result<Self, ShapeError> {
        if database.is_empty() {
            return Err(ShapeError::EmptyFontDatabase);
        }
        Ok(Self { database, request })
    }

    #[must_use]
    pub fn request(&self) -> &FontRequest {
        &self.request
    }
}

impl ClusterMetrics for FontMetrics {
    fn advance(&self, cluster: &str, style: VisualRunStyle) -> f32 {
        let request = self.request.for_style(style);
        self.database
            .resolve(&request, cluster)
            .and_then(|id| self.database.face(id))
            .map_or(0.0, |face| face.nominal_advance() * request.size())
    }
}

fn add_offset(start: ByteOffset, local: usize) -> Result<ByteOffset, ShapeError> {
    start
        .checked_add(u64::try_from(local).map_err(|_| ShapeError::OffsetOverflow)?)
        .ok_or(ShapeError::OffsetOverflow)
}

fn hash_cluster(cluster: &str) -> u32 {
    let mut hash = 2_166_136_261_u32;
    for byte in cluster.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange};
    use yu_layout::{LayoutConfig, LayoutSnapshot};
    use yu_projection::Projection;
    use yu_text::TextBuffer;

    fn database() -> Arc<FontDatabase> {
        let mut database = FontDatabase::new();
        database
            .register(
                FontFaceSpec::new("Latin", 0.5).with_coverage(FontCoverage::Ranges(vec![
                    UnicodeRange::new('a', 'z').expect("range should be valid"),
                ])),
            )
            .expect("Latin face should register");
        database
            .register(FontFaceSpec::new("Fallback", 1.0))
            .expect("fallback face should register");
        Arc::new(database)
    }

    #[test]
    fn fallback_shaping_splits_runs_and_preserves_source_clusters() {
        let database = database();
        let latin = database
            .resolve(
                &FontRequest::new("Latin", 12.0).expect("request should be valid"),
                "a",
            )
            .expect("Latin should resolve");
        let fallback = database
            .resolve(
                &FontRequest::new("Latin", 12.0).expect("request should be valid"),
                "🙂",
            )
            .expect("fallback should resolve");
        assert_ne!(latin, fallback);

        let request = ShapeRequest::new(
            "a🙂",
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(5)).expect("range should be valid"),
            VisualRunStyle::Plain,
            FontRequest::new("Latin", 12.0).expect("request should be valid"),
        )
        .expect("shape request should be valid");
        let shaped = MockShaper::new(Arc::clone(&database))
            .expect("shaper should build")
            .shape(&request)
            .expect("text should shape");
        assert_eq!(shaped.runs().len(), 2);
        assert_eq!(shaped.runs()[0].face(), latin);
        assert_eq!(shaped.runs()[0].glyphs()[0].source().len(), 1);
        assert_eq!(shaped.runs()[1].face(), fallback);
        assert_eq!(shaped.runs()[1].glyphs()[0].source().start().get(), 1);
        assert_eq!(shaped.runs()[1].glyphs()[0].source().len(), 4);
    }

    #[test]
    fn font_metrics_feed_the_existing_layout_contract() {
        let source = "ab";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let projection = Projection::inline(
            &snapshot,
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("range should be valid"),
        )
        .expect("projection should build");
        let metrics = FontMetrics::new(
            database(),
            FontRequest::new("Latin", 2.0).expect("request should be valid"),
        )
        .expect("metrics should build");
        let layout = LayoutSnapshot::from_projection_with_metrics(
            &projection,
            LayoutConfig::new(2.0, 1.0),
            &metrics,
        )
        .expect("layout should consume font metrics");
        assert_eq!(layout.lines().len(), 1);
        assert_eq!(layout.lines()[0].width(), 2.0);
        assert_eq!(layout.clusters().len(), 2);
    }

    #[test]
    fn font_shaper_feeds_shaping_aware_layout() {
        let source = "ab";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let projection = Projection::inline(
            &snapshot,
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("range should be valid"),
        )
        .expect("projection should build");
        let shaper = FontShaper::new(
            database(),
            FontRequest::new("Latin", 2.0).expect("request should be valid"),
        )
        .expect("shaper should build");
        let layout = LayoutSnapshot::from_projection_with_shaper(
            &projection,
            LayoutConfig::new(2.0, 1.0),
            &shaper,
        )
        .expect("layout should consume shaped runs");

        assert_eq!(layout.lines().len(), 1);
        assert_eq!(layout.lines()[0].width(), 2.0);
        assert_eq!(layout.clusters().len(), 2);
    }

    #[test]
    fn font_shaper_shapes_scaled_requests_at_the_target_size() {
        let shaper = FontShaper::new(
            database(),
            FontRequest::new("Latin", 2.0).expect("request should be valid"),
        )
        .expect("shaper should build");
        let source = TextRange::new(ByteOffset::ZERO, ByteOffset::new(1)).expect("source range");
        let body = ShapingProvider::shape(&shaper, "a", source, VisualRunStyle::Plain)
            .expect("body shaping");
        let heading =
            ShapingProvider::shape_scaled(&shaper, "a", source, VisualRunStyle::Strong, 2.0)
                .expect("heading shaping");

        assert_eq!(body.advance(), 1.0);
        assert_eq!(heading.advance(), 2.0);
        assert_eq!(heading.runs()[0].style(), VisualRunStyle::Strong);
    }
}
