use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use super::{FontFaceId, GlyphId};

/// A validated cache key for metrics belonging to one face at one point size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontMetricKey {
    face: FontFaceId,
    size_bits: u32,
}

impl FontMetricKey {
    pub fn new(face: FontFaceId, size: f32) -> Result<Self, RasterDataError> {
        if !size.is_finite() || size <= 0.0 {
            return Err(RasterDataError::InvalidSize(size.to_bits()));
        }
        Ok(Self {
            face,
            size_bits: size.to_bits(),
        })
    }

    #[must_use]
    pub const fn face(self) -> FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        f32::from_bits(self.size_bits)
    }
}

/// A validated cache key for one glyph at one point size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphRasterKey {
    face: FontFaceId,
    glyph: GlyphId,
    size_bits: u32,
}

impl GlyphRasterKey {
    pub fn new(face: FontFaceId, glyph: GlyphId, size: f32) -> Result<Self, RasterDataError> {
        if !size.is_finite() || size <= 0.0 {
            return Err(RasterDataError::InvalidSize(size.to_bits()));
        }
        Ok(Self {
            face,
            glyph,
            size_bits: size.to_bits(),
        })
    }

    #[must_use]
    pub const fn face(self) -> FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        f32::from_bits(self.size_bits)
    }
}

/// Font-wide metrics copied out of a native font object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetricsSnapshot {
    ascent: f32,
    descent: f32,
    leading: f32,
    units_per_em: u32,
}

impl FontMetricsSnapshot {
    pub fn new(
        ascent: f32,
        descent: f32,
        leading: f32,
        units_per_em: u32,
    ) -> Result<Self, RasterDataError> {
        if !ascent.is_finite() || ascent < 0.0 {
            return Err(RasterDataError::InvalidMetric("ascent"));
        }
        if !descent.is_finite() || descent < 0.0 {
            return Err(RasterDataError::InvalidMetric("descent"));
        }
        if !leading.is_finite() {
            return Err(RasterDataError::InvalidMetric("leading"));
        }
        if units_per_em == 0 {
            return Err(RasterDataError::InvalidMetric("units_per_em"));
        }
        Ok(Self {
            ascent,
            descent,
            leading,
            units_per_em,
        })
    }

    #[must_use]
    pub const fn ascent(self) -> f32 {
        self.ascent
    }

    #[must_use]
    pub const fn descent(self) -> f32 {
        self.descent
    }

    #[must_use]
    pub const fn leading(self) -> f32 {
        self.leading
    }

    #[must_use]
    pub const fn units_per_em(self) -> u32 {
        self.units_per_em
    }

    #[must_use]
    pub fn line_height(self) -> f32 {
        self.ascent + self.descent + self.leading
    }
}

/// Per-glyph placement metrics copied out of a native rasterizer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphMetrics {
    bearing_x: f32,
    bearing_y: f32,
    advance_x: f32,
}

impl GlyphMetrics {
    pub fn new(bearing_x: f32, bearing_y: f32, advance_x: f32) -> Result<Self, RasterDataError> {
        if !bearing_x.is_finite() {
            return Err(RasterDataError::InvalidMetric("bearing_x"));
        }
        if !bearing_y.is_finite() {
            return Err(RasterDataError::InvalidMetric("bearing_y"));
        }
        if !advance_x.is_finite() || advance_x < 0.0 {
            return Err(RasterDataError::InvalidMetric("advance_x"));
        }
        Ok(Self {
            bearing_x,
            bearing_y,
            advance_x,
        })
    }

    #[must_use]
    pub const fn bearing_x(self) -> f32 {
        self.bearing_x
    }

    #[must_use]
    pub const fn bearing_y(self) -> f32 {
        self.bearing_y
    }

    #[must_use]
    pub const fn advance_x(self) -> f32 {
        self.advance_x
    }
}

/// Owned, single-channel glyph coverage pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphBitmap {
    width: u32,
    height: u32,
    stride: u32,
    pixels: Arc<[u8]>,
}

impl GlyphBitmap {
    pub fn new(
        width: u32,
        height: u32,
        stride: u32,
        pixels: impl Into<Arc<[u8]>>,
    ) -> Result<Self, RasterDataError> {
        if stride < width {
            return Err(RasterDataError::InvalidBitmap(
                "stride is smaller than width",
            ));
        }
        let pixels = pixels.into();
        let expected = usize::try_from(stride)
            .ok()
            .and_then(|stride| usize::try_from(height).ok()?.checked_mul(stride))
            .ok_or(RasterDataError::PixelLengthOverflow)?;
        if pixels.len() != expected {
            return Err(RasterDataError::InvalidBitmap(
                "pixel length does not match dimensions",
            ));
        }
        Ok(Self {
            width,
            height,
            stride,
            pixels,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[must_use]
    pub fn shared_pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Complete native rasterization result retained by the atlas.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterizedGlyph {
    key: GlyphRasterKey,
    metrics: GlyphMetrics,
    bitmap: GlyphBitmap,
}

impl RasterizedGlyph {
    pub fn new(key: GlyphRasterKey, metrics: GlyphMetrics, bitmap: GlyphBitmap) -> Self {
        Self {
            key,
            metrics,
            bitmap,
        }
    }

    #[must_use]
    pub const fn key(&self) -> GlyphRasterKey {
        self.key
    }

    #[must_use]
    pub const fn metrics(&self) -> GlyphMetrics {
        self.metrics
    }

    #[must_use]
    pub const fn bitmap(&self) -> &GlyphBitmap {
        &self.bitmap
    }
}

/// Errors raised while constructing owned raster data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RasterDataError {
    InvalidSize(u32),
    InvalidMetric(&'static str),
    InvalidBitmap(&'static str),
    PixelLengthOverflow,
}

impl fmt::Display for RasterDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize(size) => {
                write!(formatter, "invalid raster size {}", f32::from_bits(*size))
            }
            Self::InvalidMetric(metric) => write!(formatter, "invalid glyph metric {metric}"),
            Self::InvalidBitmap(message) => formatter.write_str(message),
            Self::PixelLengthOverflow => formatter.write_str("glyph pixel length overflowed"),
        }
    }
}

impl Error for RasterDataError {}

/// Native glyph metrics/rasterization boundary. Implementations must return
/// owned values; native font/context handles stay inside the platform crate.
pub trait GlyphRasterizer: Send + Sync {
    type Error: fmt::Display;

    fn font_metrics(&self, key: FontMetricKey) -> Result<FontMetricsSnapshot, Self::Error>;

    fn rasterize(&self, key: GlyphRasterKey) -> Result<RasterizedGlyph, Self::Error>;
}

/// A small metrics cache keyed by stable face id and point size.
#[derive(Clone, Debug, Default)]
pub struct FontMetricsCache {
    entries: HashMap<FontMetricKey, FontMetricsSnapshot>,
}

impl FontMetricsCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get(&self, key: FontMetricKey) -> Option<FontMetricsSnapshot> {
        self.entries.get(&key).copied()
    }

    pub fn insert(&mut self, key: FontMetricKey, metrics: FontMetricsSnapshot) {
        self.entries.insert(key, metrics);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A rectangle in a single-channel atlas page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl AtlasRect {
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Placement and metrics for one atlas entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasEntry {
    key: GlyphRasterKey,
    page: Option<u32>,
    rect: AtlasRect,
    metrics: GlyphMetrics,
}

impl AtlasEntry {
    #[must_use]
    pub const fn key(self) -> GlyphRasterKey {
        self.key
    }

    #[must_use]
    pub const fn page(self) -> Option<u32> {
        self.page
    }

    #[must_use]
    pub const fn rect(self) -> AtlasRect {
        self.rect
    }

    #[must_use]
    pub const fn metrics(self) -> GlyphMetrics {
        self.metrics
    }
}

/// Atlas dimensions and page policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphAtlasConfig {
    page_width: u32,
    page_height: u32,
    max_pages: usize,
    padding: u32,
}

impl GlyphAtlasConfig {
    pub fn new(page_width: u32, page_height: u32, max_pages: usize) -> Result<Self, AtlasError> {
        if page_width == 0 || page_height == 0 || max_pages == 0 {
            return Err(AtlasError::InvalidConfig);
        }
        Ok(Self {
            page_width,
            page_height,
            max_pages,
            padding: 1,
        })
    }

    #[must_use]
    pub const fn page_width(self) -> u32 {
        self.page_width
    }

    #[must_use]
    pub const fn page_height(self) -> u32 {
        self.page_height
    }

    #[must_use]
    pub const fn max_pages(self) -> usize {
        self.max_pages
    }

    #[must_use]
    pub const fn padding(self) -> u32 {
        self.padding
    }
}

impl Default for GlyphAtlasConfig {
    fn default() -> Self {
        Self {
            page_width: 1024,
            page_height: 1024,
            max_pages: 8,
            padding: 1,
        }
    }
}

/// Errors raised while packing glyphs into the CPU-side atlas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtlasError {
    InvalidConfig,
    GlyphTooLarge { width: u32, height: u32 },
    CapacityExceeded,
    PageIndexOutOfBounds,
    InvalidBitmap(RasterDataError),
}

impl fmt::Display for AtlasError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid glyph atlas configuration"),
            Self::GlyphTooLarge { width, height } => {
                write!(
                    formatter,
                    "glyph {width}x{height} does not fit an atlas page"
                )
            }
            Self::CapacityExceeded => formatter.write_str("glyph atlas page capacity exceeded"),
            Self::PageIndexOutOfBounds => formatter.write_str("glyph atlas page index is invalid"),
            Self::InvalidBitmap(error) => write!(formatter, "invalid atlas bitmap: {error}"),
        }
    }
}

impl Error for AtlasError {}

#[derive(Clone, Debug)]
struct AtlasPage {
    pixels: Vec<u8>,
    next_x: u32,
    next_y: u32,
    row_height: u32,
}

impl AtlasPage {
    fn new(config: GlyphAtlasConfig) -> Result<Self, AtlasError> {
        let len = usize::try_from(config.page_width)
            .ok()
            .and_then(|width| {
                usize::try_from(config.page_height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(AtlasError::CapacityExceeded)?;
        Ok(Self {
            pixels: vec![0; len],
            next_x: 0,
            next_y: 0,
            row_height: 0,
        })
    }

    fn place(&mut self, config: GlyphAtlasConfig, width: u32, height: u32) -> Option<AtlasRect> {
        let padded_width = width.checked_add(config.padding)?;
        let padded_height = height.checked_add(config.padding)?;
        if self.next_x.checked_add(padded_width)? > config.page_width {
            self.next_x = 0;
            self.next_y = self.next_y.checked_add(self.row_height)?;
            self.row_height = 0;
        }
        if self.next_y.checked_add(padded_height)? > config.page_height {
            return None;
        }
        let rect = AtlasRect {
            x: self.next_x,
            y: self.next_y,
            width,
            height,
        };
        self.next_x = self.next_x.checked_add(padded_width)?;
        self.row_height = self.row_height.max(padded_height);
        Some(rect)
    }

    fn copy_bitmap(
        &mut self,
        config: GlyphAtlasConfig,
        rect: AtlasRect,
        bitmap: &GlyphBitmap,
    ) -> Result<(), AtlasError> {
        let page_width =
            usize::try_from(config.page_width).map_err(|_| AtlasError::CapacityExceeded)?;
        let rect_x = usize::try_from(rect.x).map_err(|_| AtlasError::CapacityExceeded)?;
        let rect_y = usize::try_from(rect.y).map_err(|_| AtlasError::CapacityExceeded)?;
        let width = usize::try_from(rect.width).map_err(|_| AtlasError::CapacityExceeded)?;
        let height = usize::try_from(rect.height).map_err(|_| AtlasError::CapacityExceeded)?;
        let stride = usize::try_from(bitmap.stride()).map_err(|_| AtlasError::CapacityExceeded)?;
        for row in 0..height {
            let source_start = row
                .checked_mul(stride)
                .ok_or(AtlasError::CapacityExceeded)?;
            let target_start = rect_y
                .checked_add(row)
                .and_then(|y| y.checked_mul(page_width))
                .and_then(|row_start| row_start.checked_add(rect_x))
                .ok_or(AtlasError::CapacityExceeded)?;
            let source = bitmap
                .pixels()
                .get(
                    source_start
                        ..source_start
                            .checked_add(width)
                            .ok_or(AtlasError::CapacityExceeded)?,
                )
                .ok_or(AtlasError::InvalidBitmap(RasterDataError::InvalidBitmap(
                    "source row is out of bounds",
                )))?;
            let target = self
                .pixels
                .get_mut(
                    target_start
                        ..target_start
                            .checked_add(width)
                            .ok_or(AtlasError::CapacityExceeded)?,
                )
                .ok_or(AtlasError::CapacityExceeded)?;
            target.copy_from_slice(source);
        }
        Ok(())
    }
}

/// A single-channel CPU atlas. The renderer can upload page pixels to GPU
/// textures later without making this cache part of canonical editor state.
#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    config: GlyphAtlasConfig,
    pages: Vec<AtlasPage>,
    entries: HashMap<GlyphRasterKey, AtlasEntry>,
    glyphs: HashMap<GlyphRasterKey, Arc<RasterizedGlyph>>,
    insertion_order: VecDeque<GlyphRasterKey>,
    bytes: usize,
}

impl GlyphAtlas {
    #[must_use]
    pub fn new(config: GlyphAtlasConfig) -> Self {
        Self {
            config,
            pages: Vec::new(),
            entries: HashMap::new(),
            glyphs: HashMap::new(),
            insertion_order: VecDeque::new(),
            bytes: 0,
        }
    }

    #[must_use]
    pub fn config(&self) -> GlyphAtlasConfig {
        self.config
    }

    #[must_use]
    pub fn get(&self, key: GlyphRasterKey) -> Option<Arc<RasterizedGlyph>> {
        self.glyphs.get(&key).cloned()
    }

    #[must_use]
    pub fn entry(&self, key: GlyphRasterKey) -> Option<AtlasEntry> {
        self.entries.get(&key).copied()
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn page_pixels(&self, page: u32) -> Result<&[u8], AtlasError> {
        self.pages
            .get(usize::try_from(page).map_err(|_| AtlasError::PageIndexOutOfBounds)?)
            .map(|page| page.pixels.as_slice())
            .ok_or(AtlasError::PageIndexOutOfBounds)
    }

    pub fn insert(&mut self, glyph: RasterizedGlyph) -> Result<AtlasEntry, AtlasError> {
        if let Some(entry) = self.entries.get(&glyph.key()).copied() {
            return Ok(entry);
        }
        let bitmap = glyph.bitmap();
        if bitmap.width() > self.config.page_width || bitmap.height() > self.config.page_height {
            return Err(AtlasError::GlyphTooLarge {
                width: bitmap.width(),
                height: bitmap.height(),
            });
        }
        let rect = if bitmap.is_empty() {
            AtlasEntry {
                key: glyph.key(),
                page: None,
                rect: AtlasRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                metrics: glyph.metrics(),
            }
        } else {
            let mut placement = None;
            for (index, page) in self.pages.iter_mut().enumerate() {
                if let Some(rect) = page.place(self.config, bitmap.width(), bitmap.height()) {
                    page.copy_bitmap(self.config, rect, bitmap)?;
                    placement = Some((
                        u32::try_from(index).map_err(|_| AtlasError::CapacityExceeded)?,
                        rect,
                    ));
                    break;
                }
            }
            if placement.is_none() {
                if self.pages.len() >= self.config.max_pages {
                    return Err(AtlasError::CapacityExceeded);
                }
                let mut page = AtlasPage::new(self.config)?;
                let rect = page
                    .place(self.config, bitmap.width(), bitmap.height())
                    .ok_or(AtlasError::GlyphTooLarge {
                        width: bitmap.width(),
                        height: bitmap.height(),
                    })?;
                page.copy_bitmap(self.config, rect, bitmap)?;
                self.pages.push(page);
                placement = Some((
                    u32::try_from(self.pages.len() - 1)
                        .map_err(|_| AtlasError::CapacityExceeded)?,
                    rect,
                ));
            }
            let (page, rect) = placement.ok_or(AtlasError::CapacityExceeded)?;
            AtlasEntry {
                key: glyph.key(),
                page: Some(page),
                rect,
                metrics: glyph.metrics(),
            }
        };
        self.bytes = self
            .bytes
            .checked_add(bitmap.pixels().len())
            .ok_or(AtlasError::CapacityExceeded)?;
        let key = glyph.key();
        self.insertion_order.push_back(key);
        self.glyphs.insert(key, Arc::new(glyph));
        self.entries.insert(key, rect);
        Ok(rect)
    }

    /// Removes the oldest glyph metadata. Pages are intentionally retained so
    /// the future GPU uploader can keep stable texture ids; a later page epoch
    /// will provide actual texture reclamation.
    pub fn remove_oldest(&mut self) -> Option<GlyphRasterKey> {
        let key = self.insertion_order.pop_front()?;
        let glyph = self.glyphs.remove(&key)?;
        self.entries.remove(&key);
        self.bytes = self.bytes.saturating_sub(glyph.bitmap().pixels().len());
        Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(glyph: u32, size: f32) -> GlyphRasterKey {
        GlyphRasterKey::new(FontFaceId::from_raw(1), GlyphId::from_raw(glyph), size)
            .expect("test key should be valid")
    }

    fn glyph(glyph: u32, width: u32, height: u32) -> RasterizedGlyph {
        let key = key(glyph, 12.0);
        let bitmap = GlyphBitmap::new(
            width,
            height,
            width,
            vec![
                u8::try_from(glyph).expect("test glyph fits");
                usize::try_from(width * height).expect("bitmap fits")
            ],
        )
        .expect("bitmap should be valid");
        let metrics = GlyphMetrics::new(0.0, 10.0, 8.0).expect("metrics should be valid");
        RasterizedGlyph::new(key, metrics, bitmap)
    }

    #[test]
    fn metric_and_bitmap_keys_reject_invalid_sizes() {
        assert_eq!(
            FontMetricKey::new(FontFaceId::from_raw(1), 0.0),
            Err(RasterDataError::InvalidSize(0.0_f32.to_bits()))
        );
        assert!(GlyphBitmap::new(2, 2, 1, vec![0; 4]).is_err());
    }

    #[test]
    fn metrics_cache_is_keyed_by_face_and_size() {
        let key = FontMetricKey::new(FontFaceId::from_raw(1), 12.0).expect("key");
        let metrics = FontMetricsSnapshot::new(9.0, 3.0, 1.0, 1000).expect("metrics");
        let mut cache = FontMetricsCache::new();
        assert!(cache.get(key).is_none());
        cache.insert(key, metrics);
        assert_eq!(cache.get(key), Some(metrics));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn atlas_packs_entries_and_keeps_zero_area_glyphs() {
        let config = GlyphAtlasConfig::new(5, 4, 2).expect("config");
        let mut atlas = GlyphAtlas::new(config);
        let first = atlas.insert(glyph(1, 2, 2)).expect("first glyph");
        let second = atlas.insert(glyph(2, 1, 2)).expect("second glyph");
        assert_eq!(first.page(), Some(0));
        assert_eq!(second.page(), Some(0));
        assert_eq!(atlas.page_count(), 1);
        assert_eq!(atlas.len(), 2);
        assert!(atlas.page_pixels(0).expect("page").contains(&1));

        let empty_key = key(3, 12.0);
        let empty = RasterizedGlyph::new(
            empty_key,
            GlyphMetrics::new(0.0, 0.0, 4.0).expect("metrics"),
            GlyphBitmap::new(0, 0, 0, Vec::<u8>::new()).expect("empty bitmap"),
        );
        let empty_entry = atlas.insert(empty).expect("empty glyph");
        assert_eq!(empty_entry.page(), None);
        assert_eq!(atlas.get(empty_key).expect("cached glyph").key(), empty_key);
    }

    #[test]
    fn atlas_rejects_glyphs_that_need_more_pages() {
        let config = GlyphAtlasConfig::new(2, 2, 1).expect("config");
        let mut atlas = GlyphAtlas::new(config);
        atlas.insert(glyph(1, 2, 1)).expect("first glyph");
        assert_eq!(
            atlas.insert(glyph(2, 2, 2)),
            Err(AtlasError::CapacityExceeded)
        );
        assert_eq!(atlas.remove_oldest(), Some(key(1, 12.0)));
        assert!(atlas.get(key(1, 12.0)).is_none());
    }
}
