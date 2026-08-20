//! macOS CoreText font discovery and fallback selection.
//!
//! This crate intentionally returns owned, platform-neutral metadata rather
//! than exposing `CTFontRef` to the editor core. `CoreTextShaper` keeps the
//! CoreText objects on the platform side while exporting owned glyph runs.

#[cfg(target_os = "macos")]
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use std::sync::OnceLock;

#[cfg(target_os = "macos")]
use unicode_segmentation::UnicodeSegmentation;
#[cfg(target_os = "macos")]
use yu_core::ByteOffset;
use yu_core::TextRange;
use yu_font::FontFaceId;
use yu_font::{
    AtlasEntry, FontMetricKey, FontMetricsSnapshot, GlyphRasterKey, GlyphRasterizer,
    RasterizedGlyph,
};
#[cfg(target_os = "macos")]
use yu_font::{
    AtlasError, FontMetricsCache, FontSlant, FontWeight, Glyph, GlyphAtlas, GlyphAtlasConfig,
    GlyphBitmap, GlyphId, GlyphMetrics, GlyphRun,
};
use yu_font::{
    FontRequest, ShapeError, ShapeRequest, ShapedText, ShapingProvider, TextDirection, TextShaper,
};
use yu_projection::VisualRunStyle;

#[cfg(target_os = "macos")]
use objc2_core_foundation::{
    CFArray, CFAttributedString, CFDictionary, CFRange, CFRetained, CFString, CGPoint, CGRect,
    CGSize,
};
#[cfg(target_os = "macos")]
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData, CGContext,
    CGGlyph, CGImageAlphaInfo,
};
#[cfg(target_os = "macos")]
use objc2_core_text::{
    CTFont, CTFontManagerCopyAvailableFontFamilyNames, CTFontOrientation, CTFontSymbolicTraits,
    CTFontUIFontType, CTLine, CTRun, CTRunStatus, kCTFontAttributeName,
};

/// Errors raised by the CoreText adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreTextFontError {
    UnsupportedPlatform,
    EmptyCatalog,
    InvalidTextRange,
    FontNameUnavailable,
}

impl fmt::Display for CoreTextFontError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("CoreText is only available on macOS"),
            Self::EmptyCatalog => formatter.write_str("CoreText returned no font families"),
            Self::InvalidTextRange => formatter.write_str("text is too large for a CoreText range"),
            Self::FontNameUnavailable => formatter.write_str("CoreText did not return a font name"),
        }
    }
}

impl Error for CoreTextFontError {}

/// Errors raised while converting CoreText output into Yu's source-backed
/// glyph contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreTextShapeError {
    UnsupportedPlatform,
    UnsupportedDirection(TextDirection),
    AttributedStringUnavailable,
    InvalidCoreTextRange,
    InvalidGlyphRun,
    FontUnavailable,
    MissingRunFont,
    NonMonotonicGlyphIndices,
    FaceIdOverflow,
    FaceTablePoisoned,
    InvalidViewportMetrics,
}

impl fmt::Display for CoreTextShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("CoreText is only available on macOS"),
            Self::UnsupportedDirection(direction) => {
                write!(
                    formatter,
                    "CoreText shaping currently requires Ltr direction, got {direction:?}"
                )
            }
            Self::AttributedStringUnavailable => {
                formatter.write_str("CoreText could not create an attributed string")
            }
            Self::InvalidCoreTextRange => {
                formatter.write_str("CoreText returned an invalid UTF-16 range")
            }
            Self::InvalidGlyphRun => formatter.write_str("CoreText returned an invalid glyph run"),
            Self::FontUnavailable => {
                formatter.write_str("CoreText could not create the requested font")
            }
            Self::MissingRunFont => formatter.write_str("CoreText glyph run did not expose a font"),
            Self::NonMonotonicGlyphIndices => {
                formatter.write_str("CoreText returned non-monotonic glyph string indices")
            }
            Self::FaceIdOverflow => formatter.write_str("CoreText face id table overflowed"),
            Self::FaceTablePoisoned => formatter.write_str("CoreText face id table was poisoned"),
            Self::InvalidViewportMetrics => {
                formatter.write_str("CoreText returned invalid viewport metrics")
            }
        }
    }
}

impl Error for CoreTextShapeError {}

/// Errors raised while copying CoreText metrics and glyph pixels into owned
/// Yu data. Native CoreText/CoreGraphics objects never cross this boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreTextRasterError {
    UnsupportedPlatform,
    UnknownFace(FontFaceId),
    InvalidGlyphId(u32),
    FaceTablePoisoned,
    FontUnavailable,
    MetricsCachePoisoned,
    AtlasPoisoned,
    BitmapUnavailable,
    InvalidNativeMetrics,
    InvalidNativeBitmap,
    InvalidRasterData(Arc<str>),
    Atlas(Arc<str>),
}

impl fmt::Display for CoreTextRasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("CoreText is only available on macOS"),
            Self::UnknownFace(face) => {
                write!(formatter, "CoreText face id {} is unknown", face.get())
            }
            Self::InvalidGlyphId(glyph) => {
                write!(formatter, "glyph id {glyph} does not fit CGGlyph")
            }
            Self::FaceTablePoisoned => formatter.write_str("CoreText face id table was poisoned"),
            Self::FontUnavailable => {
                formatter.write_str("CoreText could not recreate a glyph font")
            }
            Self::MetricsCachePoisoned => {
                formatter.write_str("CoreText metrics cache was poisoned")
            }
            Self::AtlasPoisoned => formatter.write_str("CoreText glyph atlas was poisoned"),
            Self::BitmapUnavailable => {
                formatter.write_str("CoreGraphics could not create a bitmap context")
            }
            Self::InvalidNativeMetrics => {
                formatter.write_str("CoreText returned invalid glyph metrics")
            }
            Self::InvalidNativeBitmap => {
                formatter.write_str("CoreGraphics returned invalid bitmap data")
            }
            Self::InvalidRasterData(message) => write!(formatter, "invalid raster data: {message}"),
            Self::Atlas(message) => {
                write!(formatter, "glyph atlas rejected raster data: {message}")
            }
        }
    }
}

impl Error for CoreTextRasterError {}

/// A retained, sorted snapshot of the font family names visible to CoreText.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTextFontCatalog {
    families: Arc<[Arc<str>]>,
}

impl CoreTextFontCatalog {
    /// Reads the current system font family catalog from CoreText.
    pub fn system() -> Result<Self, CoreTextFontError> {
        #[cfg(target_os = "macos")]
        {
            let families = unsafe { CTFontManagerCopyAvailableFontFamilyNames() };
            // CoreText returns an array whose elements are CFStringRef. The
            // binding intentionally erases the element type at the C ABI, so
            // this checked-at-the-call-site cast restores it for iteration.
            let families: CFRetained<CFArray<CFString>> =
                unsafe { CFRetained::cast_unchecked(families) };
            let mut names = families
                .iter()
                .map(|family| Arc::<str>::from(family.to_string()))
                // Names beginning with `.` are private system UI aliases.
                // They must be created through CTFontCreateUIFontForLanguage,
                // not passed back to CTFontCreateWithName as user families.
                .filter(|family| !family.trim().is_empty() && !family.trim_start().starts_with('.'))
                .collect::<Vec<_>>();
            names.sort_unstable_by(|left, right| left.as_ref().cmp(right.as_ref()));
            names.dedup_by(|left, right| left.as_ref() == right.as_ref());
            if names.is_empty() {
                return Err(CoreTextFontError::EmptyCatalog);
            }
            Ok(Self {
                families: Arc::from(names.into_boxed_slice()),
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(CoreTextFontError::UnsupportedPlatform)
        }
    }

    /// Builds a catalog from deterministic names in tests or a platform
    /// bootstrap layer.
    #[must_use]
    pub fn from_families(families: impl IntoIterator<Item = impl Into<Arc<str>>>) -> Self {
        let mut names = families
            .into_iter()
            .map(Into::into)
            .filter(|family: &Arc<str>| !family.trim().is_empty())
            .collect::<Vec<_>>();
        names.sort_unstable_by(|left, right| left.as_ref().cmp(right.as_ref()));
        names.dedup_by(|left, right| left.as_ref() == right.as_ref());
        Self {
            families: Arc::from(names.into_boxed_slice()),
        }
    }

    #[must_use]
    pub fn families(&self) -> &[Arc<str>] {
        &self.families
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.families.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    #[must_use]
    pub fn contains_family(&self, family: &str) -> bool {
        self.families
            .iter()
            .any(|candidate| candidate.as_ref() == family)
    }

    #[must_use]
    pub fn resolver(&self) -> CoreTextFontResolver {
        CoreTextFontResolver {
            catalog: self.clone(),
        }
    }
}

/// Metadata for one CoreText-selected face. The underlying `CTFontRef` is
/// intentionally not stored here, so this value is safe to move across the
/// editor's platform-independent layers.
#[derive(Clone, Debug, PartialEq)]
pub struct CoreTextResolvedFont {
    requested_family: Arc<str>,
    family: Arc<str>,
    postscript_name: Arc<str>,
    size: f32,
    fallback: bool,
}

impl CoreTextResolvedFont {
    #[must_use]
    pub fn requested_family(&self) -> &str {
        &self.requested_family
    }

    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }

    #[must_use]
    pub fn postscript_name(&self) -> &str {
        &self.postscript_name
    }

    #[must_use]
    pub const fn size(&self) -> f32 {
        self.size
    }

    #[must_use]
    pub const fn used_fallback(&self) -> bool {
        self.fallback
    }
}

/// A resolver scoped to one catalog snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTextFontResolver {
    catalog: CoreTextFontCatalog,
}

impl CoreTextFontResolver {
    #[must_use]
    pub fn catalog(&self) -> &CoreTextFontCatalog {
        &self.catalog
    }

    /// Resolves a family and lets CoreText choose a cascade fallback for the
    /// supplied UTF-8 string. This performs no shaping or glyph rasterization.
    pub fn resolve(
        &self,
        request: &FontRequest,
        text: &str,
    ) -> Result<CoreTextResolvedFont, CoreTextFontError> {
        #[cfg(target_os = "macos")]
        {
            let family = CFString::from_str(request.family());
            let base = unsafe { CTFont::with_name(&family, request.size() as _, std::ptr::null()) };
            let selected = if text.is_empty() {
                base
            } else {
                let text = CFString::from_str(text);
                let length = text.length();
                let range = CFRange {
                    location: 0,
                    length,
                };
                unsafe { base.for_string(&text, range) }
            };
            let selected_family = unsafe { selected.family_name() }.to_string();
            let postscript_name = unsafe { selected.post_script_name() }.to_string();
            if selected_family.trim().is_empty() || postscript_name.trim().is_empty() {
                return Err(CoreTextFontError::FontNameUnavailable);
            }
            Ok(CoreTextResolvedFont {
                requested_family: Arc::from(request.family()),
                fallback: selected_family != request.family(),
                family: Arc::from(selected_family),
                postscript_name: Arc::from(postscript_name),
                size: request.size(),
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (request, text);
            Err(CoreTextFontError::UnsupportedPlatform)
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct FaceTable {
    next: u32,
    ids: BTreeMap<String, FontFaceId>,
    names: Vec<String>,
}

#[cfg(target_os = "macos")]
impl FaceTable {
    fn id_for(&mut self, postscript_name: &str) -> Result<FontFaceId, CoreTextShapeError> {
        if let Some(face) = self.ids.get(postscript_name) {
            return Ok(*face);
        }
        let face = FontFaceId::from_raw(self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or(CoreTextShapeError::FaceIdOverflow)?;
        self.names.push(postscript_name.to_owned());
        self.ids.insert(postscript_name.to_owned(), face);
        Ok(face)
    }

    fn name_for(&self, face: FontFaceId) -> Option<&str> {
        self.names
            .get(usize::try_from(face.get()).ok()?)
            .map(String::as_str)
    }
}

#[cfg(target_os = "macos")]
fn shared_face_table() -> Arc<Mutex<FaceTable>> {
    static TABLE: OnceLock<Arc<Mutex<FaceTable>>> = OnceLock::new();

    Arc::clone(TABLE.get_or_init(|| Arc::new(Mutex::new(FaceTable::default()))))
}

/// A CoreText-backed implementation of both the platform-independent shaping
/// contract and the layout-facing `ShapingProvider` adapter.
///
/// CoreText objects are created and discarded during `shape`; only owned
/// `ShapedText`/`GlyphRun` data and stable numeric face ids leave this crate.
#[derive(Debug)]
pub struct CoreTextShaper {
    catalog: CoreTextFontCatalog,
    request: FontRequest,
    font_source: CoreTextFontSource,
    #[cfg(target_os = "macos")]
    faces: Arc<Mutex<FaceTable>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoreTextFontSource {
    RequestedFamily,
    SystemUi,
}

/// Owned font metrics used to configure the native viewport before a full
/// shaped layout is attached to the editor document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoreTextViewportMetrics {
    line_height: f32,
    default_advance: f32,
}

impl CoreTextViewportMetrics {
    #[must_use]
    pub const fn line_height(self) -> f32 {
        self.line_height
    }

    #[must_use]
    pub const fn default_advance(self) -> f32 {
        self.default_advance
    }
}

impl Clone for CoreTextShaper {
    fn clone(&self) -> Self {
        Self {
            catalog: self.catalog.clone(),
            request: self.request.clone(),
            font_source: self.font_source,
            #[cfg(target_os = "macos")]
            faces: Arc::clone(&self.faces),
        }
    }
}

impl CoreTextShaper {
    #[must_use]
    pub fn new(catalog: CoreTextFontCatalog, request: FontRequest) -> Self {
        Self {
            catalog,
            request,
            font_source: CoreTextFontSource::RequestedFamily,
            #[cfg(target_os = "macos")]
            faces: shared_face_table(),
        }
    }

    pub fn from_system(request: FontRequest) -> Result<Self, CoreTextFontError> {
        Ok(Self::new(CoreTextFontCatalog::system()?, request))
    }

    /// Creates a shaper backed by the AppKit/CoreText system UI font. The
    /// request's family is retained as metadata, but is not passed to
    /// `CTFontCreateWithName`; private names such as `.SFNS-Regular` must be
    /// created through `CTFontCreateUIFontForLanguage` instead.
    pub fn from_system_ui(request: FontRequest) -> Result<Self, CoreTextFontError> {
        let mut shaper = Self::new(CoreTextFontCatalog::system()?, request);
        shaper.font_source = CoreTextFontSource::SystemUi;
        Ok(shaper)
    }

    #[must_use]
    pub fn catalog(&self) -> &CoreTextFontCatalog {
        &self.catalog
    }

    #[must_use]
    pub fn request(&self) -> &FontRequest {
        &self.request
    }

    /// Measures a mixed grapheme sample with CoreText and returns owned
    /// point-based metrics for the metrics-only viewport backend. The sample
    /// is shaped rather than measured with a guessed single-character width,
    /// so fallback faces and combining/emoji clusters contribute naturally.
    pub fn viewport_metrics(
        &self,
        sample: &str,
    ) -> Result<CoreTextViewportMetrics, CoreTextShapeError> {
        #[cfg(target_os = "macos")]
        {
            if sample.is_empty() {
                return Err(CoreTextShapeError::InvalidViewportMetrics);
            }
            let source = TextRange::new(
                ByteOffset::ZERO,
                ByteOffset::new(
                    u64::try_from(sample.len())
                        .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?,
                ),
            )
            .ok_or(CoreTextShapeError::InvalidViewportMetrics)?;
            let request =
                ShapeRequest::new(sample, source, VisualRunStyle::Plain, self.request.clone())
                    .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?;
            let shaped = self.shape_request(&request)?;
            let grapheme_count = sample.graphemes(true).count();
            let Some(run) = shaped.runs().first() else {
                return Err(CoreTextShapeError::InvalidViewportMetrics);
            };
            if grapheme_count == 0 {
                return Err(CoreTextShapeError::InvalidViewportMetrics);
            }
            let rasterizer = self.rasterizer();
            let key = FontMetricKey::new(run.face(), self.request.size())
                .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?;
            let font_metrics = rasterizer
                .font_metrics(key)
                .map_err(|_| CoreTextShapeError::InvalidViewportMetrics)?;
            let line_height = font_metrics.line_height();
            let default_advance = shaped.advance() / grapheme_count as f32;
            if !line_height.is_finite()
                || line_height <= 0.0
                || !default_advance.is_finite()
                || default_advance <= 0.0
            {
                return Err(CoreTextShapeError::InvalidViewportMetrics);
            }
            Ok(CoreTextViewportMetrics {
                line_height,
                default_advance,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = sample;
            Err(CoreTextShapeError::UnsupportedPlatform)
        }
    }

    /// Creates a glyph rasterizer that shares this shaper's stable face table.
    /// The returned object owns only caches and can be used by layout/render
    /// preparation without exposing any CoreText handle.
    #[must_use]
    pub fn rasterizer(&self) -> CoreTextGlyphRasterizer {
        #[cfg(target_os = "macos")]
        {
            CoreTextGlyphRasterizer::with_faces(Arc::clone(&self.faces), self.font_source)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = self;
            CoreTextGlyphRasterizer::unsupported()
        }
    }

    #[cfg(target_os = "macos")]
    fn face_id(&self, postscript_name: &str) -> Result<FontFaceId, CoreTextShapeError> {
        self.faces
            .lock()
            .map_err(|_| CoreTextShapeError::FaceTablePoisoned)?
            .id_for(postscript_name)
    }

    fn shape_request(&self, request: &ShapeRequest<'_>) -> Result<ShapedText, CoreTextShapeError> {
        #[cfg(target_os = "macos")]
        {
            shape_with_core_text(self, request)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = request;
            Err(CoreTextShapeError::UnsupportedPlatform)
        }
    }
}

/// CoreText-backed metrics and CPU glyph rasterization.
///
/// This is deliberately a preparation-layer object: it returns owned
/// single-channel pixels and atlas placements, not a platform texture or a
/// `CTFontRef`. The future renderer can upload atlas pages without changing
/// the editor/source model.
#[derive(Debug)]
pub struct CoreTextGlyphRasterizer {
    #[cfg(target_os = "macos")]
    faces: Arc<Mutex<FaceTable>>,
    font_source: CoreTextFontSource,
    #[cfg(target_os = "macos")]
    metrics: Arc<Mutex<FontMetricsCache>>,
    #[cfg(target_os = "macos")]
    atlas: Arc<Mutex<GlyphAtlas>>,
}

impl Clone for CoreTextGlyphRasterizer {
    fn clone(&self) -> Self {
        Self {
            #[cfg(target_os = "macos")]
            faces: Arc::clone(&self.faces),
            font_source: self.font_source,
            #[cfg(target_os = "macos")]
            metrics: Arc::clone(&self.metrics),
            #[cfg(target_os = "macos")]
            atlas: Arc::clone(&self.atlas),
        }
    }
}

impl CoreTextGlyphRasterizer {
    #[cfg(target_os = "macos")]
    fn with_faces(faces: Arc<Mutex<FaceTable>>, font_source: CoreTextFontSource) -> Self {
        Self {
            faces,
            font_source,
            metrics: Arc::new(Mutex::new(FontMetricsCache::new())),
            atlas: Arc::new(Mutex::new(GlyphAtlas::new(GlyphAtlasConfig::default()))),
        }
    }

    #[cfg(not(target_os = "macos"))]
    const fn unsupported() -> Self {
        Self {
            font_source: CoreTextFontSource::RequestedFamily,
        }
    }

    /// Returns the cached atlas placement, if this glyph has already been
    /// rasterized by this provider.
    #[cfg(target_os = "macos")]
    pub fn atlas_entry(
        &self,
        key: GlyphRasterKey,
    ) -> Result<Option<AtlasEntry>, CoreTextRasterError> {
        self.atlas
            .lock()
            .map_err(|_| CoreTextRasterError::AtlasPoisoned)
            .map(|atlas| atlas.entry(key))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn atlas_entry(
        &self,
        _key: GlyphRasterKey,
    ) -> Result<Option<AtlasEntry>, CoreTextRasterError> {
        Err(CoreTextRasterError::UnsupportedPlatform)
    }

    #[cfg(target_os = "macos")]
    pub fn metrics_cache_len(&self) -> Result<usize, CoreTextRasterError> {
        self.metrics
            .lock()
            .map_err(|_| CoreTextRasterError::MetricsCachePoisoned)
            .map(|cache| cache.len())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn metrics_cache_len(&self) -> Result<usize, CoreTextRasterError> {
        Err(CoreTextRasterError::UnsupportedPlatform)
    }

    #[cfg(target_os = "macos")]
    pub fn atlas_page_count(&self) -> Result<usize, CoreTextRasterError> {
        self.atlas
            .lock()
            .map_err(|_| CoreTextRasterError::AtlasPoisoned)
            .map(|atlas| atlas.page_count())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn atlas_page_count(&self) -> Result<usize, CoreTextRasterError> {
        Err(CoreTextRasterError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "macos")]
impl GlyphRasterizer for CoreTextGlyphRasterizer {
    type Error = CoreTextRasterError;

    fn font_metrics(&self, key: FontMetricKey) -> Result<FontMetricsSnapshot, Self::Error> {
        if let Some(metrics) = self
            .metrics
            .lock()
            .map_err(|_| CoreTextRasterError::MetricsCachePoisoned)?
            .get(key)
        {
            return Ok(metrics);
        }

        let font = self.font_for_face(key.face(), key.size())?;
        let metrics = FontMetricsSnapshot::new(
            unsafe { font.ascent() } as f32,
            unsafe { font.descent() } as f32,
            unsafe { font.leading() } as f32,
            unsafe { font.units_per_em() },
        )
        .map_err(|_| CoreTextRasterError::InvalidNativeMetrics)?;
        self.metrics
            .lock()
            .map_err(|_| CoreTextRasterError::MetricsCachePoisoned)?
            .insert(key, metrics);
        Ok(metrics)
    }

    fn rasterize(&self, key: GlyphRasterKey) -> Result<RasterizedGlyph, Self::Error> {
        if let Some(glyph) = self
            .atlas
            .lock()
            .map_err(|_| CoreTextRasterError::AtlasPoisoned)?
            .get(key)
        {
            return Ok(glyph.as_ref().clone());
        }

        let glyph = u16::try_from(key.glyph().get())
            .map_err(|_| CoreTextRasterError::InvalidGlyphId(key.glyph().get()))?;
        let font = self.font_for_face(key.face(), key.size())?;
        let (bounds, advance) = native_glyph_geometry(&font, glyph)?;
        let (bitmap, bearing_x, bearing_y) = rasterize_glyph(&font, glyph, bounds)?;
        let metrics = GlyphMetrics::new(bearing_x, bearing_y, advance)
            .map_err(|_| CoreTextRasterError::InvalidNativeMetrics)?;
        let rasterized = RasterizedGlyph::new(key, metrics, bitmap);
        self.atlas
            .lock()
            .map_err(|_| CoreTextRasterError::AtlasPoisoned)?
            .insert(rasterized.clone())
            .map_err(|error| match error {
                AtlasError::InvalidBitmap(error) => {
                    CoreTextRasterError::InvalidRasterData(Arc::from(error.to_string()))
                }
                other => CoreTextRasterError::Atlas(Arc::from(other.to_string())),
            })?;
        Ok(rasterized)
    }
}

#[cfg(not(target_os = "macos"))]
impl GlyphRasterizer for CoreTextGlyphRasterizer {
    type Error = CoreTextRasterError;

    fn font_metrics(&self, _key: FontMetricKey) -> Result<FontMetricsSnapshot, Self::Error> {
        Err(CoreTextRasterError::UnsupportedPlatform)
    }

    fn rasterize(&self, _key: GlyphRasterKey) -> Result<RasterizedGlyph, Self::Error> {
        Err(CoreTextRasterError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "macos")]
impl CoreTextGlyphRasterizer {
    fn font_for_face(
        &self,
        face: FontFaceId,
        size: f32,
    ) -> Result<CFRetained<CTFont>, CoreTextRasterError> {
        let postscript_name = self
            .faces
            .lock()
            .map_err(|_| CoreTextRasterError::FaceTablePoisoned)?
            .name_for(face)
            .map(str::to_owned)
            .ok_or(CoreTextRasterError::UnknownFace(face))?;
        if self.font_source == CoreTextFontSource::SystemUi
            && postscript_name.trim_start().starts_with('.')
        {
            return unsafe {
                CTFont::new_ui_font_for_language(CTFontUIFontType::System, size as _, None)
            }
            .ok_or(CoreTextRasterError::FontUnavailable);
        }
        let name = CFString::from_str(&postscript_name);
        Ok(unsafe { CTFont::with_name(&name, size as _, std::ptr::null()) })
    }
}

#[cfg(target_os = "macos")]
fn native_glyph_geometry(
    font: &CTFont,
    glyph: CGGlyph,
) -> Result<(CGRect, f32), CoreTextRasterError> {
    let mut glyph = glyph;
    let pointer = NonNull::from(&mut glyph);
    let bounds = unsafe {
        font.bounding_rects_for_glyphs(
            CTFontOrientation::Horizontal,
            pointer,
            std::ptr::null_mut(),
            1,
        )
    };
    let mut advance = CGSize::ZERO;
    unsafe {
        font.advances_for_glyphs(
            CTFontOrientation::Horizontal,
            pointer,
            std::ptr::addr_of_mut!(advance),
            1,
        );
    }
    let values = [
        bounds.origin.x as f64,
        bounds.origin.y as f64,
        bounds.size.width as f64,
        bounds.size.height as f64,
        advance.width,
    ];
    if values.iter().any(|value| !value.is_finite())
        || bounds.size.width < 0.0
        || bounds.size.height < 0.0
        || advance.width < 0.0
    {
        return Err(CoreTextRasterError::InvalidNativeMetrics);
    }
    Ok((bounds, advance.width as f32))
}

#[cfg(target_os = "macos")]
fn rasterize_glyph(
    font: &CTFont,
    glyph: CGGlyph,
    bounds: CGRect,
) -> Result<(GlyphBitmap, f32, f32), CoreTextRasterError> {
    let min_x = bounds.origin.x;
    let min_y = bounds.origin.y;
    let max_x = min_x + bounds.size.width;
    let max_y = min_y + bounds.size.height;
    if bounds.size.width == 0.0 || bounds.size.height == 0.0 {
        let bitmap = GlyphBitmap::new(0, 0, 0, Vec::<u8>::new()).map_err(|error| {
            CoreTextRasterError::InvalidRasterData(Arc::from(error.to_string()))
        })?;
        return Ok((bitmap, min_x.floor() as f32, max_y.ceil() as f32));
    }

    let left = min_x.floor() - 1.0;
    let bottom = min_y.floor() - 1.0;
    let right = max_x.ceil() + 1.0;
    let top = max_y.ceil() + 1.0;
    let width = raster_dimension(right - left)?;
    let height = raster_dimension(top - bottom)?;
    let width_usize =
        usize::try_from(width).map_err(|_| CoreTextRasterError::InvalidNativeBitmap)?;
    let height_usize =
        usize::try_from(height).map_err(|_| CoreTextRasterError::InvalidNativeBitmap)?;
    let context = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            width_usize,
            height_usize,
            8,
            width_usize,
            None,
            CGImageAlphaInfo::Only.0,
        )
    }
    .ok_or(CoreTextRasterError::BitmapUnavailable)?;
    CGContext::set_gray_fill_color(Some(context.as_ref()), 0.0, 1.0);

    let mut glyph = glyph;
    let mut position = CGPoint {
        x: (-left) as _,
        y: (-bottom) as _,
    };
    unsafe {
        font.draw_glyphs(
            NonNull::from(&mut glyph),
            NonNull::from(&mut position),
            1,
            context.as_ref(),
        );
    }
    let stride = CGBitmapContextGetBytesPerRow(Some(context.as_ref()));
    let data = CGBitmapContextGetData(Some(context.as_ref()));
    if data.is_null() || stride < width_usize {
        return Err(CoreTextRasterError::InvalidNativeBitmap);
    }
    let source_len = stride
        .checked_mul(height_usize)
        .ok_or(CoreTextRasterError::InvalidNativeBitmap)?;
    let source = unsafe { std::slice::from_raw_parts(data.cast_const().cast::<u8>(), source_len) };
    let pixel_len = width_usize
        .checked_mul(height_usize)
        .ok_or(CoreTextRasterError::InvalidNativeBitmap)?;
    let mut pixels = vec![0_u8; pixel_len];
    for row in 0..height_usize {
        let source_start = row
            .checked_mul(stride)
            .ok_or(CoreTextRasterError::InvalidNativeBitmap)?;
        let target_row = height_usize - 1 - row;
        let target_start = target_row
            .checked_mul(width_usize)
            .ok_or(CoreTextRasterError::InvalidNativeBitmap)?;
        let source_row = source
            .get(source_start..source_start + width_usize)
            .ok_or(CoreTextRasterError::InvalidNativeBitmap)?;
        let target_row = pixels
            .get_mut(target_start..target_start + width_usize)
            .ok_or(CoreTextRasterError::InvalidNativeBitmap)?;
        target_row.copy_from_slice(source_row);
    }
    let bitmap = GlyphBitmap::new(width, height, width, pixels)
        .map_err(|error| CoreTextRasterError::InvalidRasterData(Arc::from(error.to_string())))?;
    Ok((bitmap, left as f32, top as f32))
}

#[cfg(target_os = "macos")]
fn raster_dimension(value: f64) -> Result<u32, CoreTextRasterError> {
    if !value.is_finite() || value <= 0.0 || value > f64::from(u32::MAX) {
        return Err(CoreTextRasterError::InvalidNativeBitmap);
    }
    Ok(value.ceil() as u32)
}

impl TextShaper for CoreTextShaper {
    fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedText, ShapeError> {
        self.shape_request(request)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))
    }
}

impl ShapingProvider for CoreTextShaper {
    type Error = ShapeError;

    fn shape(
        &self,
        text: &str,
        source: TextRange,
        style: VisualRunStyle,
    ) -> Result<ShapedText, Self::Error> {
        let request = ShapeRequest::new(text, source, style, self.request.clone())?;
        <Self as TextShaper>::shape(self, &request)
    }

    fn shape_scaled(
        &self,
        text: &str,
        source: TextRange,
        style: VisualRunStyle,
        scale: f32,
    ) -> Result<ShapedText, Self::Error> {
        let size = self.request.size() * scale;
        let font = FontRequest::new(self.request.family(), size)
            .map_err(|error| ShapeError::Backend(Arc::from(error.to_string())))?
            .with_weight(self.request.weight())
            .with_slant(self.request.slant());
        let request = ShapeRequest::new(text, source, style, font)?;
        <Self as TextShaper>::shape(self, &request)
    }
}

#[cfg(target_os = "macos")]
fn style_font_request(request: &FontRequest, style: VisualRunStyle) -> FontRequest {
    match style {
        VisualRunStyle::Strong => request.clone().with_weight(FontWeight::Bold),
        VisualRunStyle::Emphasis => request.clone().with_slant(FontSlant::Italic),
        VisualRunStyle::Plain | VisualRunStyle::Code => request.clone(),
    }
}

#[cfg(target_os = "macos")]
fn create_core_text_font(
    request: &FontRequest,
    source: CoreTextFontSource,
) -> Result<CFRetained<CTFont>, CoreTextShapeError> {
    let base = match source {
        CoreTextFontSource::RequestedFamily => {
            let family = CFString::from_str(request.family());
            unsafe { CTFont::with_name(&family, request.size() as _, std::ptr::null()) }
        }
        CoreTextFontSource::SystemUi => unsafe {
            CTFont::new_ui_font_for_language(CTFontUIFontType::System, request.size() as _, None)
        }
        .ok_or(CoreTextShapeError::FontUnavailable)?,
    };
    let mut value = CTFontSymbolicTraits::empty();
    let mask = CTFontSymbolicTraits::TraitBold | CTFontSymbolicTraits::TraitItalic;
    if request.weight() == FontWeight::Bold {
        value.insert(CTFontSymbolicTraits::TraitBold);
    }
    if request.slant() != FontSlant::Upright {
        value.insert(CTFontSymbolicTraits::TraitItalic);
    }
    Ok(unsafe {
        base.copy_with_symbolic_traits(request.size() as _, std::ptr::null(), value, mask)
            .unwrap_or(base)
    })
}

#[cfg(target_os = "macos")]
fn shape_with_core_text(
    shaper: &CoreTextShaper,
    request: &ShapeRequest<'_>,
) -> Result<ShapedText, CoreTextShapeError> {
    if request.text().is_empty() {
        return Ok(ShapedText::new(request.source(), Vec::new()));
    }
    if request.direction() != TextDirection::Ltr {
        return Err(CoreTextShapeError::UnsupportedDirection(
            request.direction(),
        ));
    }

    let font_request = style_font_request(request.font(), request.style());
    let font = create_core_text_font(&font_request, shaper.font_source)?;
    let string = CFString::from_str(request.text());
    let font_attribute_name = unsafe { kCTFontAttributeName };
    let keys: [&CFString; 1] = [font_attribute_name];
    let values: [&CTFont; 1] = [font.as_ref()];
    let attributes = CFDictionary::from_slices(&keys, &values);
    let attributed =
        unsafe { CFAttributedString::new(None, Some(&string), Some(attributes.as_ref())) }
            .ok_or(CoreTextShapeError::AttributedStringUnavailable)?;
    let line = unsafe { CTLine::with_attributed_string(&attributed) };
    let runs = unsafe { line.glyph_runs() };
    let runs: CFRetained<CFArray<CTRun>> = unsafe { CFRetained::cast_unchecked(runs) };
    let map = Utf16Map::new(request.text());
    let glyph_runs = runs
        .iter()
        .map(|run| shape_run(shaper, request, &map, &run))
        .collect::<Result<Vec<_>, _>>()?;
    if glyph_runs.is_empty() {
        return Err(CoreTextShapeError::InvalidGlyphRun);
    }
    Ok(ShapedText::new(request.source(), glyph_runs))
}

#[cfg(target_os = "macos")]
fn shape_run(
    shaper: &CoreTextShaper,
    request: &ShapeRequest<'_>,
    map: &Utf16Map,
    run: &CTRun,
) -> Result<GlyphRun, CoreTextShapeError> {
    let glyph_count = unsafe { run.glyph_count() };
    let glyph_count =
        usize::try_from(glyph_count).map_err(|_| CoreTextShapeError::InvalidGlyphRun)?;
    if glyph_count == 0 {
        return Err(CoreTextShapeError::InvalidGlyphRun);
    }

    let run_range = unsafe { run.string_range() };
    let run_start = cf_index_to_usize(run_range.location)?;
    let run_length = cf_index_to_usize(run_range.length)?;
    let run_end = run_start
        .checked_add(run_length)
        .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
    let source = map.range(run_start, run_end, request.source())?;

    let attributes = unsafe { run.attributes() };
    let attributes: &CFDictionary<CFString, CTFont> = unsafe { attributes.cast_unchecked() };
    let font_attribute_name = unsafe { kCTFontAttributeName };
    let face = attributes
        .get(font_attribute_name)
        .ok_or(CoreTextShapeError::MissingRunFont)?;
    let postscript_name = unsafe { face.post_script_name() }.to_string();
    if postscript_name.trim().is_empty() {
        return Err(CoreTextShapeError::MissingRunFont);
    }
    let face_id = shaper.face_id(&postscript_name)?;

    let cf_range = CFRange {
        location: 0,
        length: glyph_count as _,
    };
    let mut raw_glyphs = vec![0 as CGGlyph; glyph_count];
    let mut positions = vec![CGPoint::ZERO; glyph_count];
    let mut advances = vec![CGSize::ZERO; glyph_count];
    let mut indices = vec![0; glyph_count];
    unsafe {
        run.glyphs(
            cf_range,
            std::ptr::NonNull::new(raw_glyphs.as_mut_ptr())
                .ok_or(CoreTextShapeError::InvalidGlyphRun)?,
        );
        run.positions(
            cf_range,
            std::ptr::NonNull::new(positions.as_mut_ptr())
                .ok_or(CoreTextShapeError::InvalidGlyphRun)?,
        );
        run.advances(
            cf_range,
            std::ptr::NonNull::new(advances.as_mut_ptr())
                .ok_or(CoreTextShapeError::InvalidGlyphRun)?,
        );
        run.string_indices(
            cf_range,
            std::ptr::NonNull::new(indices.as_mut_ptr())
                .ok_or(CoreTextShapeError::InvalidGlyphRun)?,
        );
    }

    let status = unsafe { run.status() };
    if status.intersects(CTRunStatus::RightToLeft | CTRunStatus::NonMonotonic) {
        return Err(CoreTextShapeError::NonMonotonicGlyphIndices);
    }

    let mut previous_index = None;
    let mut starts = Vec::with_capacity(glyph_count);
    for index in indices {
        let index = cf_index_to_usize(index)?;
        if index < run_start || index >= run_end {
            return Err(CoreTextShapeError::InvalidCoreTextRange);
        }
        if previous_index.is_some_and(|previous| index < previous) {
            return Err(CoreTextShapeError::NonMonotonicGlyphIndices);
        }
        previous_index = Some(index);
        starts.push(index);
    }

    let mut glyphs = Vec::with_capacity(glyph_count);
    let mut pen_x = 0.0_f32;
    for (index, raw_glyph) in raw_glyphs.into_iter().enumerate() {
        let start = starts[index];
        let end = starts[index + 1..]
            .iter()
            .copied()
            .find(|candidate| *candidate > start)
            .unwrap_or(run_end);
        let glyph_source = map.range(start, end, request.source())?;
        let advance = advances[index].width as f32;
        let position_x = positions[index].x as f32;
        let position_y = positions[index].y as f32;
        if !advance.is_finite()
            || advance < 0.0
            || !position_x.is_finite()
            || !position_y.is_finite()
        {
            return Err(CoreTextShapeError::InvalidGlyphRun);
        }
        let x_offset = position_x - pen_x;
        if !x_offset.is_finite() {
            return Err(CoreTextShapeError::InvalidGlyphRun);
        }
        glyphs.push(Glyph::new(
            GlyphId::from_raw(u32::from(raw_glyph)),
            glyph_source,
            advance,
            x_offset,
            position_y,
        ));
        pen_x += advance;
    }

    Ok(GlyphRun::new(
        face_id,
        source,
        request.style(),
        request.direction(),
        request.script(),
        glyphs,
    ))
}

#[cfg(target_os = "macos")]
fn cf_index_to_usize(index: isize) -> Result<usize, CoreTextShapeError> {
    usize::try_from(index).map_err(|_| CoreTextShapeError::InvalidCoreTextRange)
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct Utf16Map {
    boundaries: Vec<Option<usize>>,
}

#[cfg(target_os = "macos")]
impl Utf16Map {
    fn new(text: &str) -> Self {
        let mut boundaries = vec![Some(0)];
        for character in text.chars() {
            let byte_end = boundaries
                .last()
                .and_then(|boundary| *boundary)
                .unwrap_or(0_usize)
                .saturating_add(character.len_utf8());
            if character.len_utf16() == 2 {
                boundaries.push(None);
            }
            boundaries.push(Some(byte_end));
        }
        Self { boundaries }
    }

    fn range(
        &self,
        start: usize,
        end: usize,
        source: TextRange,
    ) -> Result<TextRange, CoreTextShapeError> {
        let start = self
            .boundaries
            .get(start)
            .and_then(|boundary| *boundary)
            .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
        let end = self
            .boundaries
            .get(end)
            .and_then(|boundary| *boundary)
            .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
        if start > end || end > usize::try_from(source.len()).unwrap_or(usize::MAX) {
            return Err(CoreTextShapeError::InvalidCoreTextRange);
        }
        let source_start = source
            .start()
            .checked_add(
                u64::try_from(start).map_err(|_| CoreTextShapeError::InvalidCoreTextRange)?,
            )
            .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
        let source_end = source
            .start()
            .checked_add(u64::try_from(end).map_err(|_| CoreTextShapeError::InvalidCoreTextRange)?)
            .ok_or(CoreTextShapeError::InvalidCoreTextRange)?;
        TextRange::new(source_start, source_end).ok_or(CoreTextShapeError::InvalidCoreTextRange)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange};

    #[test]
    fn catalog_normalizes_names() {
        let catalog = CoreTextFontCatalog::from_families(["Zed", "Yu", "Yu", ""]);
        assert_eq!(
            catalog.families(),
            &[Arc::<str>::from("Yu"), Arc::from("Zed")]
        );
        assert!(catalog.contains_family("Yu"));
        assert!(!catalog.contains_family("Missing"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_catalog_and_resolver_are_live() {
        let catalog = CoreTextFontCatalog::system().expect("CoreText should expose families");
        assert!(!catalog.is_empty());
        let family = catalog.families()[0].clone();
        assert!(!family.trim_start().starts_with('.'));
        let request = FontRequest::new(family.as_ref(), 13.0).expect("request should be valid");
        let resolved = catalog
            .resolver()
            .resolve(&request, "羽🙂")
            .expect("CoreText should resolve a fallback font");
        assert_eq!(resolved.requested_family(), family.as_ref());
        assert!(!resolved.family().is_empty());
        assert!(!resolved.postscript_name().is_empty());
        assert_eq!(resolved.size(), 13.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_text_shaper_returns_real_glyph_clusters_for_unicode() {
        let catalog = CoreTextFontCatalog::system().expect("CoreText should expose families");
        let family = catalog.families()[0].clone();
        let request = FontRequest::new(family.as_ref(), 16.0).expect("request should be valid");
        let shaper = CoreTextShaper::new(catalog, request.clone());
        let text = "office 羽🙂 e\u{301}";
        let source = TextRange::new(
            ByteOffset::ZERO,
            ByteOffset::new(u64::try_from(text.len()).expect("test text should fit")),
        )
        .expect("source range should be valid");
        let shape_request =
            ShapeRequest::new(text, source, VisualRunStyle::Plain, request).expect("request");
        let shaped = TextShaper::shape(&shaper, &shape_request).expect("CoreText should shape");

        assert_eq!(shaped.source(), source);
        assert!(shaped.advance().is_finite());
        assert!(shaped.advance() > 0.0);
        assert!(!shaped.runs().is_empty());
        assert!(shaped.runs().iter().all(|run| {
            run.source().start() >= source.start()
                && run.source().end() <= source.end()
                && run.glyphs().iter().all(|glyph| {
                    glyph.source().start() >= run.source().start()
                        && glyph.source().end() <= run.source().end()
                        && glyph.advance().is_finite()
                        && glyph.advance() >= 0.0
                })
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_text_viewport_metrics_use_shaped_grapheme_advance() {
        let catalog = CoreTextFontCatalog::system().expect("CoreText should expose families");
        let family = catalog.families()[0].clone();
        let request = FontRequest::new(family.as_ref(), 16.0).expect("request should be valid");
        let shaper = CoreTextShaper::new(catalog, request.clone());
        let sample = "M中🙂e\u{301}";
        let metrics = shaper
            .viewport_metrics(sample)
            .expect("CoreText viewport metrics should be available");
        let source = TextRange::new(
            ByteOffset::ZERO,
            ByteOffset::new(u64::try_from(sample.len()).expect("sample should fit")),
        )
        .expect("source range should be valid");
        let request = ShapeRequest::new(sample, source, VisualRunStyle::Plain, request)
            .expect("request should be valid");
        let shaped = TextShaper::shape(&shaper, &request).expect("CoreText should shape");
        let expected = shaped.advance() / sample.graphemes(true).count() as f32;
        assert!((metrics.default_advance() - expected).abs() < 0.001);
        assert!(metrics.line_height() > 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_text_system_ui_font_uses_ui_creation_path() {
        let request = FontRequest::new(".SFNS-Regular", 16.0).expect("request should be valid");
        let shaper = CoreTextShaper::from_system_ui(request).expect("CoreText should initialize");
        let metrics = shaper
            .viewport_metrics("M中🙂e\u{301}")
            .expect("system UI metrics should be available");
        assert!(metrics.line_height().is_finite() && metrics.line_height() > 0.0);
        assert!(metrics.default_advance().is_finite() && metrics.default_advance() > 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn utf16_map_only_accepts_scalar_boundaries() {
        let text = "a🙂e";
        let source = TextRange::new(
            ByteOffset::ZERO,
            ByteOffset::new(u64::try_from(text.len()).expect("test text should fit")),
        )
        .expect("source range should be valid");
        let map = Utf16Map::new(text);

        assert_eq!(
            map.range(1, 3, source).expect("emoji range"),
            TextRange::new(ByteOffset::new(1), ByteOffset::new(5),).expect("expected source range")
        );
        assert_eq!(
            map.range(2, 3, source),
            Err(CoreTextShapeError::InvalidCoreTextRange)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn layout_consumes_core_text_advances_and_source_clusters() {
        let text = "office 羽🙂";
        let buffer = yu_text::TextBuffer::new(text);
        let snapshot = buffer.snapshot();
        let source = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("snapshot range should be valid");
        let projection = yu_projection::Projection::inline(&snapshot, source)
            .expect("inline projection should be valid");
        let family = CoreTextFontCatalog::system()
            .expect("CoreText should expose families")
            .families()[0]
            .clone();
        let request = FontRequest::new(family.as_ref(), 16.0).expect("request should be valid");
        let shaper = CoreTextShaper::from_system(request).expect("CoreText should initialize");
        let layout = yu_layout::LayoutSnapshot::from_projection_with_shaper(
            &projection,
            yu_layout::LayoutConfig::new(10_000.0, 20.0),
            &shaper,
        )
        .expect("layout should consume CoreText glyph runs");

        assert_eq!(layout.source_range(), source);
        assert!(layout.lines()[0].width() > 0.0);
        assert!(layout.clusters().iter().all(|cluster| {
            cluster.source().start() >= source.start() && cluster.source().end() <= source.end()
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_text_rasterizer_returns_owned_pixels_and_reuses_caches() {
        let catalog = CoreTextFontCatalog::system().expect("CoreText should expose families");
        let family = catalog.families()[0].clone();
        let request = FontRequest::new(family.as_ref(), 18.0).expect("request should be valid");
        let shaper = CoreTextShaper::new(catalog, request.clone());
        let source = TextRange::new(ByteOffset::ZERO, ByteOffset::new(1))
            .expect("source range should be valid");
        let shape_request =
            ShapeRequest::new("A", source, VisualRunStyle::Plain, request).expect("request");
        let shaped = TextShaper::shape(&shaper, &shape_request).expect("CoreText should shape");
        let run = shaped.runs().first().expect("shaped run");
        let glyph = run.glyphs().first().expect("shaped glyph");
        let rasterizer = shaper.rasterizer();
        let metric_key = yu_font::FontMetricKey::new(run.face(), 18.0).expect("metric key");
        let metrics = rasterizer
            .font_metrics(metric_key)
            .expect("CoreText metrics should be available");
        assert!(metrics.ascent() > 0.0);
        assert!(metrics.descent() >= 0.0);
        assert_eq!(rasterizer.metrics_cache_len().expect("metrics cache"), 1);

        let key = yu_font::GlyphRasterKey::new(run.face(), glyph.id(), 18.0)
            .expect("glyph key should be valid");
        let first = rasterizer
            .rasterize(key)
            .expect("CoreText should rasterize a visible glyph");
        assert!(first.bitmap().width() > 0);
        assert!(first.bitmap().height() > 0);
        assert!(first.bitmap().pixels().iter().any(|pixel| *pixel > 0));
        assert!(first.metrics().advance_x() > 0.0);
        let second = rasterizer
            .rasterize(key)
            .expect("cached glyph should rasterize");
        assert_eq!(first, second);
        let entry = rasterizer
            .atlas_entry(key)
            .expect("atlas query")
            .expect("glyph should have an atlas entry");
        assert!(entry.page().is_some());
        assert_eq!(entry.rect().width(), first.bitmap().width());
        assert_eq!(rasterizer.atlas_page_count().expect("atlas"), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_text_face_ids_survive_shaper_recreation() {
        let catalog = CoreTextFontCatalog::system().expect("CoreText should expose families");
        let family = catalog.families()[0].clone();
        let request = FontRequest::new(family.as_ref(), 18.0).expect("request should be valid");
        let first_shaper = CoreTextShaper::new(catalog.clone(), request.clone());
        let second_shaper = CoreTextShaper::new(catalog, request.clone());
        let source = TextRange::new(ByteOffset::ZERO, ByteOffset::new(1))
            .expect("source range should be valid");
        let shape_request =
            ShapeRequest::new("A", source, VisualRunStyle::Plain, request).expect("request");
        let shaped =
            TextShaper::shape(&first_shaper, &shape_request).expect("CoreText should shape");
        let glyph = shaped
            .runs()
            .first()
            .and_then(|run| run.glyphs().first().map(|glyph| (run.face(), glyph.id())))
            .expect("shaped glyph");
        let key = GlyphRasterKey::new(glyph.0, glyph.1, 18.0).expect("glyph key should be valid");

        second_shaper
            .rasterizer()
            .rasterize(key)
            .expect("a recreated shaper must resolve the shared face id");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn system_catalog_is_explicitly_unsupported_off_macos() {
        assert_eq!(
            CoreTextFontCatalog::system(),
            Err(CoreTextFontError::UnsupportedPlatform)
        );
    }
}
