//! macOS CoreText font discovery and fallback selection.
//!
//! This crate intentionally returns owned, platform-neutral metadata rather
//! than exposing `CTFontRef` to the editor core. `CoreTextShaper` keeps the
//! CoreText objects on the platform side while exporting owned glyph runs.

#[cfg(target_os = "macos")]
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::Mutex;

use yu_core::TextRange;
#[cfg(target_os = "macos")]
use yu_font::{FontFaceId, FontSlant, FontWeight, Glyph, GlyphId, GlyphRun};
use yu_font::{
    FontRequest, ShapeError, ShapeRequest, ShapedText, ShapingProvider, TextDirection, TextShaper,
};
use yu_projection::VisualRunStyle;

#[cfg(target_os = "macos")]
use objc2_core_foundation::{
    CFArray, CFAttributedString, CFDictionary, CFRange, CFRetained, CFString, CGPoint, CGSize,
};
#[cfg(target_os = "macos")]
use objc2_core_graphics::CGGlyph;
#[cfg(target_os = "macos")]
use objc2_core_text::{
    CTFont, CTFontManagerCopyAvailableFontFamilyNames, CTFontSymbolicTraits, CTLine, CTRun,
    CTRunStatus, kCTFontAttributeName,
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
    MissingRunFont,
    NonMonotonicGlyphIndices,
    FaceIdOverflow,
    FaceTablePoisoned,
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
            Self::MissingRunFont => formatter.write_str("CoreText glyph run did not expose a font"),
            Self::NonMonotonicGlyphIndices => {
                formatter.write_str("CoreText returned non-monotonic glyph string indices")
            }
            Self::FaceIdOverflow => formatter.write_str("CoreText face id table overflowed"),
            Self::FaceTablePoisoned => formatter.write_str("CoreText face id table was poisoned"),
        }
    }
}

impl Error for CoreTextShapeError {}

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
                .filter(|family| !family.trim().is_empty())
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
        self.ids.insert(postscript_name.to_owned(), face);
        Ok(face)
    }
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
    #[cfg(target_os = "macos")]
    faces: Arc<Mutex<FaceTable>>,
}

impl Clone for CoreTextShaper {
    fn clone(&self) -> Self {
        Self {
            catalog: self.catalog.clone(),
            request: self.request.clone(),
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
            #[cfg(target_os = "macos")]
            faces: Arc::new(Mutex::new(FaceTable::default())),
        }
    }

    pub fn from_system(request: FontRequest) -> Result<Self, CoreTextFontError> {
        Ok(Self::new(CoreTextFontCatalog::system()?, request))
    }

    #[must_use]
    pub fn catalog(&self) -> &CoreTextFontCatalog {
        &self.catalog
    }

    #[must_use]
    pub fn request(&self) -> &FontRequest {
        &self.request
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
fn create_core_text_font(request: &FontRequest) -> CFRetained<CTFont> {
    let family = CFString::from_str(request.family());
    let base = unsafe { CTFont::with_name(&family, request.size() as _, std::ptr::null()) };
    let mut value = CTFontSymbolicTraits::empty();
    let mask = CTFontSymbolicTraits::TraitBold | CTFontSymbolicTraits::TraitItalic;
    if request.weight() == FontWeight::Bold {
        value.insert(CTFontSymbolicTraits::TraitBold);
    }
    if request.slant() != FontSlant::Upright {
        value.insert(CTFontSymbolicTraits::TraitItalic);
    }
    unsafe { base.copy_with_symbolic_traits(request.size() as _, std::ptr::null(), value, mask) }
        .unwrap_or(base)
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
    let font = create_core_text_font(&font_request);
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn system_catalog_is_explicitly_unsupported_off_macos() {
        assert_eq!(
            CoreTextFontCatalog::system(),
            Err(CoreTextFontError::UnsupportedPlatform)
        );
    }
}
