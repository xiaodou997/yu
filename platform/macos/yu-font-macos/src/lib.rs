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
    CFArray, CFAttributedString, CFDictionary, CFRange, CFRetained, CFString, CGAffineTransform,
    CGPoint, CGRect, CGSize,
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
    /// 重建 face 得到的字体与 shaping 时选中的不是同一个。宁可失败也不能
    /// 用错误的字体解释 glyph id——那会画出无关字形而不报任何错。
    FaceMismatch {
        expected: Arc<str>,
        actual: Arc<str>,
    },
}

impl fmt::Display for CoreTextRasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("CoreText is only available on macOS"),
            Self::FaceMismatch { expected, actual } => {
                write!(
                    formatter,
                    "CoreText face rebuild mismatch: expected {expected}, got {actual}"
                )
            }
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

/// 一个 face 的身份，以及重建它所需的信息。
///
/// 只记 PostScript 名是不够的：CoreText 为系统 UI 字体做 cascade fallback 时
/// 会选中私有字体（`.SFNS-Regular`、`.PingFangUITextSC-Regular`、
/// `.AppleColorEmojiUI`），这些名字**无法**通过 `CTFontCreateWithName` 重建
/// ——该函数在名字不可解析时不返回 null，而是静默回退到默认字体。用回退后的
/// 字体去解释原字体的 glyph id，画出来就是完全无关的字形。
///
/// 因此这里额外记住触发该 face 的样本文本，栅格化时用与 shaping 完全相同的
/// fallback 机制（`CTFontCreateForString`）重新选中同一个字体。
#[cfg(target_os = "macos")]
#[derive(Clone, Debug)]
struct FaceEntry {
    postscript_name: String,
    /// 触发该 face 的样本文本。base font 自身对应空串。
    sample: String,
    /// base font 的字重与斜体。face 身份也取决于它们：同一个样本字符在
    /// Bold 与 Regular 的 base 下会 cascade 到不同的 face
    /// （`.PingFangUIDisplaySC-Bold` 与 `-Regular`）。
    weight: FontWeight,
    slant: FontSlant,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct FaceTable {
    next: u32,
    ids: BTreeMap<String, FontFaceId>,
    entries: Vec<FaceEntry>,
}

#[cfg(target_os = "macos")]
impl FaceTable {
    fn id_for(
        &mut self,
        postscript_name: &str,
        sample: &str,
        weight: FontWeight,
        slant: FontSlant,
    ) -> Result<FontFaceId, CoreTextShapeError> {
        if let Some(face) = self.ids.get(postscript_name) {
            return Ok(*face);
        }
        let face = FontFaceId::from_raw(self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or(CoreTextShapeError::FaceIdOverflow)?;
        self.entries.push(FaceEntry {
            postscript_name: postscript_name.to_owned(),
            sample: sample.to_owned(),
            weight,
            slant,
        });
        self.ids.insert(postscript_name.to_owned(), face);
        Ok(face)
    }

    fn entry_for(&self, face: FontFaceId) -> Option<&FaceEntry> {
        self.entries.get(usize::try_from(face.get()).ok()?)
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
            CoreTextGlyphRasterizer::with_faces(
                Arc::clone(&self.faces),
                self.font_source,
                Arc::from(self.request.family()),
            )
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = self;
            CoreTextGlyphRasterizer::unsupported()
        }
    }

    #[cfg(target_os = "macos")]
    fn face_id(
        &self,
        postscript_name: &str,
        sample: &str,
        weight: FontWeight,
        slant: FontSlant,
    ) -> Result<FontFaceId, CoreTextShapeError> {
        self.faces
            .lock()
            .map_err(|_| CoreTextShapeError::FaceTablePoisoned)?
            .id_for(postscript_name, sample, weight, slant)
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
    /// base font 的 family，用于重放 shaping 时的 fallback 选择。
    requested_family: Arc<str>,
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
            requested_family: Arc::clone(&self.requested_family),
            #[cfg(target_os = "macos")]
            metrics: Arc::clone(&self.metrics),
            #[cfg(target_os = "macos")]
            atlas: Arc::clone(&self.atlas),
        }
    }
}

impl CoreTextGlyphRasterizer {
    #[cfg(target_os = "macos")]
    fn with_faces(
        faces: Arc<Mutex<FaceTable>>,
        font_source: CoreTextFontSource,
        requested_family: Arc<str>,
    ) -> Self {
        Self {
            faces,
            font_source,
            requested_family,
            metrics: Arc::new(Mutex::new(FontMetricsCache::new())),
            atlas: Arc::new(Mutex::new(GlyphAtlas::new(GlyphAtlasConfig::default()))),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn unsupported() -> Self {
        Self {
            font_source: CoreTextFontSource::RequestedFamily,
            requested_family: Arc::from(""),
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

        // 度量是逻辑量，与栅格分辨率无关。
        let font = self.font_for_face(key.face(), key.size(), 1.0)?;
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
        let font = self.font_for_face(key.face(), key.size(), key.raster_scale())?;
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
    /// 重建某个 face 对应的 `CTFont`。
    ///
    /// 关键在于**不能按 PostScript 名重建**：CoreText 为系统 UI 字体做 cascade
    /// fallback 时选中的是私有字体（`.PingFangUITextSC-Regular`、
    /// `.AppleColorEmojiUI` 等），`CTFontCreateWithName` 对这些名字既不成功也
    /// 不失败，而是静默返回默认字体。用它去解释原字体的 glyph id，画出来就是
    /// 完全无关的字形——中文和 emoji 会变成拉丁/西里尔符号。
    ///
    /// 因此这里重放 shaping 时的选择过程：先取 base font，再用记录下来的样本
    /// 字符触发同一次 `CTFontCreateForString` fallback，最后校验结果确实是同
    /// 一个 face。校验失败宁可报错，也不画出错误字形。
    fn font_for_face(
        &self,
        face: FontFaceId,
        size: f32,
        raster_scale: f32,
    ) -> Result<CFRetained<CTFont>, CoreTextRasterError> {
        let entry = self
            .faces
            .lock()
            .map_err(|_| CoreTextRasterError::FaceTablePoisoned)?
            .entry_for(face)
            .cloned()
            .ok_or(CoreTextRasterError::UnknownFace(face))?;

        // 复用 shaping 侧构造 base font 的同一个函数：两条路径各写一遍迟早
        // 会分叉，而分叉的表现就是画出错误字形。
        //
        // size 必须是**逻辑**尺寸，栅格倍率只进变换矩阵——否则会选到另一个
        // optical size 变体（PingFang UI Text ↔ Display），glyph id 随之失配。
        let request = FontRequest::new(&*self.requested_family, size)
            .map_err(|_| CoreTextRasterError::FontUnavailable)?
            .with_weight(entry.weight)
            .with_slant(entry.slant);
        let base = create_core_text_font_scaled(&request, self.font_source, raster_scale)
            .map_err(|_| CoreTextRasterError::FontUnavailable)?;

        let font = if entry.sample.is_empty() {
            base
        } else {
            let sample = CFString::from_str(&entry.sample);
            let range = CFRange {
                location: 0,
                length: sample.length(),
            };
            unsafe { base.for_string(&sample, range) }
        };

        let actual = unsafe { font.post_script_name() }.to_string();
        if actual != entry.postscript_name {
            return Err(CoreTextRasterError::FaceMismatch {
                expected: Arc::from(entry.postscript_name.as_str()),
                actual: Arc::from(actual.as_str()),
            });
        }
        Ok(font)
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
        // CGBitmapContext 的绘制坐标原点在左下，但内存布局是 top-down：
        // 扫描线 0 就是图像顶部。因此这里直接按行拷贝——额外翻转会让每个
        // 字形上下颠倒，而拉丁字母颠倒后不易察觉，中文一眼可见。
        let target_start = row
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
/// 构造 base font。
///
/// `raster_scale` 只作用于绘制变换矩阵，**不能**乘进 size：系统字体有 optical
/// size 变体（macOS 上 16pt 选 PingFang UI Text、32pt 选 Display），改 size 会
/// 选到另一个字体，其 glyph id 与 shaping 时的不再对应。
fn create_core_text_font_scaled(
    request: &FontRequest,
    source: CoreTextFontSource,
    raster_scale: f32,
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
    let matrix = CGAffineTransform {
        a: f64::from(raster_scale),
        b: 0.0,
        c: 0.0,
        d: f64::from(raster_scale),
        tx: 0.0,
        ty: 0.0,
    };
    let matrix_ptr = if (raster_scale - 1.0).abs() < f32::EPSILON {
        std::ptr::null()
    } else {
        std::ptr::from_ref(&matrix)
    };
    Ok(unsafe {
        base.copy_with_symbolic_traits(request.size() as _, matrix_ptr, value, mask)
            .unwrap_or(base)
    })
}

#[cfg(target_os = "macos")]
fn create_core_text_font(
    request: &FontRequest,
    source: CoreTextFontSource,
) -> Result<CFRetained<CTFont>, CoreTextShapeError> {
    create_core_text_font_scaled(request, source, 1.0)
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
/// 该 run 的首个字符，用作重建其 fallback 字体的样本。
///
/// CoreText 的 cascade 是按字符决定的，首字符足以重新选中同一个 face。
#[cfg(target_os = "macos")]
fn run_sample(request: &ShapeRequest<'_>, source: TextRange) -> String {
    let base = request.source().start().get();
    let Some(start) = source.start().get().checked_sub(base) else {
        return String::new();
    };
    let Ok(start) = usize::try_from(start) else {
        return String::new();
    };
    request
        .text()
        .get(start..)
        .and_then(|tail| tail.chars().next())
        .map(String::from)
        .unwrap_or_default()
}

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
    // 记住触发这个 face 的字符：栅格化时要用同样的 fallback 机制重建它，
    // 私有 UI 字体的名字无法反过来创建字体。
    let sample = run_sample(request, source);
    let styled = style_font_request(request.font(), request.style());
    let face_id = shaper.face_id(&postscript_name, &sample, styled.weight(), styled.slant())?;

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

    // CTRun 的 positions 是相对整条 CTLine 的绝对坐标，而 `Glyph::x_offset`
    // 的契约是「相对按 advance 累加出的笔位的微调」（kerning、mark
    // positioning）。布局层会自己累加 advance 再叠加 x_offset，因此这里必须
    // 先减去本 run 在行内的起点，否则从第二个 run 起，run 的起始位置会被计入
    // 两次——混合中英文时每段文字被越推越远，最终溢出可视宽度。
    let run_origin_x = positions
        .first()
        .map(|position| position.x as f32)
        .unwrap_or(0.0);
    if !run_origin_x.is_finite() {
        return Err(CoreTextShapeError::InvalidGlyphRun);
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
        let x_offset = position_x - run_origin_x - pen_x;
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

    /// shaping 选中的 face 与栅格化实际使用的字体必须是同一个。
    ///
    /// 这是 v1 一直缺失的断言，因而下述缺陷长期存在且不报任何错误：栅格化
    /// 曾按 PostScript 名重建字体，而 CoreText 为系统 UI 字体 cascade 出的
    /// 私有字体（`.PingFangUITextSC-Regular`、`.AppleColorEmojiUI`）无法被
    /// `CTFontCreateWithName` 重建——它静默返回默认字体。于是中文和 emoji 的
    /// glyph id 被拉丁字体解释，屏幕上是一片无关字形，而每个 API 都「成功」。
    ///
    /// 断言分两层：字体身份一致（由 font_for_face 的自校验保证），以及
    /// CJK/emoji 的字形尺寸确实大于拉丁字母——身份校验万一被绕过，尺寸也能
    /// 暴露出「用拉丁字体画中文」。
    #[cfg(target_os = "macos")]
    #[test]
    fn rasterized_font_matches_shaped_face_across_scripts() {
        let size = 16.0_f32;
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", size).expect("request"))
                .expect("shaper");
        let rasterizer = shaper.rasterizer();

        // 比较字形高度而不是宽度：CJK 字形填满 em box，拉丁字母只到 cap
        // height，因此高度差异是稳定的。宽度不行——「日」本身就是窄字形，
        // 正确渲染时也只有 12px 宽，和 "H" 一样。
        let mut heights: Vec<(&str, u32)> = Vec::new();
        for text in ["H", "\u{5f00}", "\u{65e5}", "\u{1f642}"] {
            let source = TextRange::new(
                ByteOffset::ZERO,
                ByteOffset::new(u64::try_from(text.len()).expect("len fits")),
            )
            .expect("source range");
            let shaped = ShapingProvider::shape(&shaper, text, source, VisualRunStyle::Plain)
                .expect("shape should succeed");
            let mut max_height = 0;
            for run in shaped.runs() {
                for glyph in run.glyphs() {
                    let key =
                        GlyphRasterKey::new(run.face(), glyph.id(), size).expect("raster key");
                    // font_for_face 的自校验在这里生效：字体身份不一致会返回
                    // FaceMismatch，而不是默默画错。
                    let raster = rasterizer
                        .rasterize(key)
                        .unwrap_or_else(|error| panic!("rasterize {text:?} failed: {error}"));
                    max_height = max_height.max(raster.bitmap().height());
                }
            }
            assert!(max_height > 0, "{text:?} produced an empty bitmap");
            heights.push((text, max_height));
        }

        let latin = heights[0].1;
        for (text, height) in &heights[1..] {
            assert!(
                *height > latin,
                "{text:?} rasterized {height}px tall, not taller than Latin {latin}px — \
                 CJK/emoji 很可能被拉丁字体解释了"
            );
        }
    }

    /// `Glyph::x_offset` 必须是相对笔位的微调，不能是 CTLine 的绝对坐标。
    ///
    /// CTRun 的 positions 相对整条 CTLine，而布局层会自己按 advance 累加笔位
    /// 再叠加 x_offset。若不减去 run 在行内的起点，从第二个 run 起该起点会被
    /// 计入两次：混合中英文时每段文字被越推越远，最终溢出可视宽度——真实窗口
    /// 里表现为词与词之间出现大段空隙、行尾内容被截断。
    #[cfg(target_os = "macos")]
    #[test]
    fn glyph_x_offset_is_relative_to_the_run_pen() {
        let size = 16.0_f32;
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", size).expect("request"))
                .expect("shaper");
        // 混合脚本才会被 CoreText 切成多个 run。
        let text = "capability mask\u{3001}block kind";
        let source = TextRange::new(
            ByteOffset::ZERO,
            ByteOffset::new(u64::try_from(text.len()).expect("len fits")),
        )
        .expect("source range");
        let shaped =
            ShapingProvider::shape(&shaper, text, source, VisualRunStyle::Plain).expect("shape");
        assert!(
            shaped.runs().len() > 1,
            "fixture 未产生多个 run，测不到 run 起点被重复计入的问题"
        );

        for (index, run) in shaped.runs().iter().enumerate() {
            let first = run.glyphs().first().expect("run has glyphs");
            assert!(
                first.x_offset().abs() < 1.0,
                "run {index} 的首个 glyph x_offset = {}，看起来是 CTLine 绝对坐标而非笔位微调",
                first.x_offset()
            );
            // 其余 glyph 的偏移也只应是微调，量级远小于自身 advance。
            for glyph in run.glyphs() {
                assert!(
                    glyph.x_offset().abs() <= glyph.advance().max(1.0),
                    "glyph x_offset = {} 超过自身 advance = {}",
                    glyph.x_offset(),
                    glyph.advance()
                );
            }
        }
    }

    /// 字形位图的方向必须正确。
    ///
    /// `CGBitmapContext` 的绘制坐标原点在左下，但内存布局是 top-down——
    /// 扫描线 0 就是图像顶部。v1 的拷贝循环额外做了一次 `height - 1 - row`
    /// 翻转，于是每个字形上下颠倒。拉丁字母颠倒后不易察觉（配合当时的字体
    /// 错位问题更看不出来），中文则一眼可见。
    ///
    /// 用 "F" 做探针：它在水平与垂直两个方向都不对称，一次断言可以同时抓住
    /// 上下颠倒与左右镜像。
    #[cfg(target_os = "macos")]
    #[test]
    fn rasterized_glyph_orientation_is_upright() {
        let size = 24.0_f32;
        let shaper =
            CoreTextShaper::from_system_ui(FontRequest::new("System UI", size).expect("request"))
                .expect("shaper");
        let rasterizer = shaper.rasterizer();
        let text = "F";
        let source = TextRange::new(ByteOffset::ZERO, ByteOffset::new(1)).expect("range");
        let shaped =
            ShapingProvider::shape(&shaper, text, source, VisualRunStyle::Plain).expect("shape");
        let run = shaped.runs().first().expect("one run");
        let glyph = run.glyphs().first().expect("one glyph");
        let raster = rasterizer
            .rasterize(GlyphRasterKey::new(run.face(), glyph.id(), size).expect("key"))
            .expect("rasterize");
        let bitmap = raster.bitmap();
        let (width, height) = (bitmap.width() as usize, bitmap.height() as usize);
        assert!(
            width >= 4 && height >= 4,
            "glyph bitmap is too small to probe"
        );

        let ink = |rows: std::ops::Range<usize>, cols: std::ops::Range<usize>| -> u64 {
            rows.flat_map(|row| {
                cols.clone().map(move |col| {
                    u64::from(bitmap.pixels()[row * bitmap.stride() as usize + col])
                })
            })
            .sum()
        };

        // "F" 的两条横都在上半部：上半墨迹必须明显多于下半。
        let top = ink(0..height / 2, 0..width);
        let bottom = ink(height / 2..height, 0..width);
        assert!(
            top > bottom,
            "字形上下颠倒：上半墨迹 {top} 不多于下半 {bottom}"
        );

        // "F" 的竖笔在左侧：左半墨迹必须明显多于右半。
        let left = ink(0..height, 0..width / 2);
        let right = ink(0..height, width / 2..width);
        assert!(
            left > right,
            "字形左右镜像：左半墨迹 {left} 不多于右半 {right}"
        );
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
