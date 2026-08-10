#![forbid(unsafe_code)]

//! Revision-bound retained drawing data for Yu Editor.
//!
//! A scene contains only owned geometry, colors and glyph-atlas placements. It
//! deliberately does not contain source text, layout caches, pixels, native
//! window objects or GPU handles. A renderer can therefore consume a scene on
//! its own thread and discard it when its source revision is stale.

use std::error::Error;
use std::fmt;

use yu_core::Revision;
use yu_font::{AtlasEntry, GlyphRasterKey};

/// A finite point in scene coordinates. The coordinate system is chosen by
/// the platform shell; Yu's current layout convention is x-right/y-down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    x: f32,
    y: f32,
}

impl Point {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    fn validate(self) -> Result<(), SceneError> {
        if self.x.is_finite() && self.y.is_finite() {
            Ok(())
        } else {
            Err(SceneError::InvalidGeometry("point must be finite"))
        }
    }
}

/// A finite, non-negative rectangle in scene coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, SceneError> {
        let rect = Self {
            x,
            y,
            width,
            height,
        };
        rect.validate()?;
        Ok(rect)
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0.0 || self.height == 0.0
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    #[must_use]
    pub fn intersects_or_touches(self, other: Self) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    fn validate(self) -> Result<(), SceneError> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !self.width.is_finite()
            || !self.height.is_finite()
            || self.width < 0.0
            || self.height < 0.0
        {
            return Err(SceneError::InvalidGeometry(
                "rectangle must be finite and non-negative",
            ));
        }
        if !self.right().is_finite() || !self.bottom().is_finite() {
            return Err(SceneError::InvalidGeometry(
                "rectangle bounds must be finite",
            ));
        }
        Ok(())
    }
}

/// Packed non-premultiplied sRGB color used by scene primitives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba8(u32);

impl Rgba8 {
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self(u32::from_be_bytes([red, green, blue, alpha]))
    }

    #[must_use]
    pub const fn red(self) -> u8 {
        self.0.to_be_bytes()[0]
    }

    #[must_use]
    pub const fn green(self) -> u8 {
        self.0.to_be_bytes()[1]
    }

    #[must_use]
    pub const fn blue(self) -> u8 {
        self.0.to_be_bytes()[2]
    }

    #[must_use]
    pub const fn alpha(self) -> u8 {
        self.0.to_be_bytes()[3]
    }

    #[must_use]
    pub const fn white() -> Self {
        Self::new(255, 255, 255, 255)
    }

    #[must_use]
    pub const fn black() -> Self {
        Self::new(0, 0, 0, 255)
    }
}

impl Default for Rgba8 {
    fn default() -> Self {
        Self::black()
    }
}

/// A glyph draw operation referencing an entry in a separate CPU/GPU atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPrimitive {
    atlas: AtlasEntry,
    origin: Point,
    color: Rgba8,
}

impl GlyphPrimitive {
    #[must_use]
    pub const fn new(atlas: AtlasEntry, origin: Point, color: Rgba8) -> Self {
        Self {
            atlas,
            origin,
            color,
        }
    }

    #[must_use]
    pub const fn atlas(self) -> AtlasEntry {
        self.atlas
    }

    #[must_use]
    pub const fn key(self) -> GlyphRasterKey {
        self.atlas.key()
    }

    #[must_use]
    pub const fn origin(self) -> Point {
        self.origin
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    /// Returns the visual bounds in baseline-oriented scene coordinates.
    #[must_use]
    pub fn bounds(self) -> Rect {
        let rect = self.atlas.rect();
        Rect {
            x: self.origin.x() + self.atlas.metrics().bearing_x(),
            y: self.origin.y() - self.atlas.metrics().bearing_y(),
            width: rect.width() as f32,
            height: rect.height() as f32,
        }
    }
}

/// One retained scene primitive. Insertion order is the painter's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    FillRect { bounds: Rect, color: Rgba8 },
    Glyph(GlyphPrimitive),
}

impl Primitive {
    #[must_use]
    pub fn bounds(self) -> Rect {
        match self {
            Self::FillRect { bounds, .. } => bounds,
            Self::Glyph(glyph) => glyph.bounds(),
        }
    }
}

/// Damage rectangles for one scene build. Adjacent or overlapping rectangles
/// are merged; when the budget is exceeded, all damage is collapsed to bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageSet {
    max_rects: usize,
    rects: Vec<Rect>,
}

impl DamageSet {
    pub fn new(max_rects: usize) -> Result<Self, SceneError> {
        if max_rects == 0 {
            return Err(SceneError::InvalidDamageBudget);
        }
        Ok(Self {
            max_rects,
            rects: Vec::new(),
        })
    }

    #[must_use]
    pub const fn max_rects(&self) -> usize {
        self.max_rects
    }

    #[must_use]
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(Rect::union)
    }

    pub fn add(&mut self, rect: Rect) -> Result<(), SceneError> {
        rect.validate()?;
        if rect.is_empty() {
            return Ok(());
        }
        let mut merged = rect;
        let mut index = 0;
        while index < self.rects.len() {
            if self.rects[index].intersects_or_touches(merged) {
                merged = merged.union(self.rects.remove(index));
            } else {
                index += 1;
            }
        }
        self.rects.push(merged);
        if self.rects.len() > self.max_rects {
            let bounds = self.bounds().ok_or(SceneError::InvalidDamageBudget)?;
            self.rects.clear();
            self.rects.push(bounds);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

impl Default for DamageSet {
    fn default() -> Self {
        Self {
            max_rects: 64,
            rects: Vec::new(),
        }
    }
}

/// Errors raised while constructing a retained scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    InvalidGeometry(&'static str),
    InvalidDamageBudget,
    PrimitiveLimitExceeded,
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry(message) => formatter.write_str(message),
            Self::InvalidDamageBudget => formatter.write_str("damage budget must be positive"),
            Self::PrimitiveLimitExceeded => formatter.write_str("scene primitive limit exceeded"),
        }
    }
}

impl Error for SceneError {}

/// A retained, source-revision-bound drawing scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    revision: Revision,
    viewport: Rect,
    primitives: Vec<Primitive>,
    damage: DamageSet,
}

impl Scene {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn viewport(&self) -> Rect {
        self.viewport
    }

    #[must_use]
    pub fn primitives(&self) -> &[Primitive] {
        &self.primitives
    }

    #[must_use]
    pub fn damage(&self) -> &DamageSet {
        &self.damage
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
    }
}

/// Builds a scene while keeping primitive and damage order deterministic.
#[derive(Clone, Debug)]
pub struct SceneBuilder {
    revision: Revision,
    viewport: Rect,
    primitives: Vec<Primitive>,
    damage: DamageSet,
    max_primitives: usize,
}

impl SceneBuilder {
    pub fn new(revision: Revision, viewport: Rect) -> Result<Self, SceneError> {
        viewport.validate()?;
        Ok(Self {
            revision,
            viewport,
            primitives: Vec::new(),
            damage: DamageSet::default(),
            max_primitives: 1_000_000,
        })
    }

    pub fn with_damage_budget(mut self, max_rects: usize) -> Result<Self, SceneError> {
        self.damage = DamageSet::new(max_rects)?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_primitive_limit(mut self, max_primitives: usize) -> Self {
        self.max_primitives = max_primitives;
        self
    }

    pub fn push(&mut self, primitive: Primitive) -> Result<u32, SceneError> {
        if self.primitives.len() >= self.max_primitives {
            return Err(SceneError::PrimitiveLimitExceeded);
        }
        if let Primitive::Glyph(glyph) = primitive {
            glyph.origin.validate()?;
        }
        primitive.bounds().validate()?;
        let index =
            u32::try_from(self.primitives.len()).map_err(|_| SceneError::PrimitiveLimitExceeded)?;
        self.primitives.push(primitive);
        self.damage.add(primitive.bounds())?;
        Ok(index)
    }

    pub fn fill_rect(&mut self, bounds: Rect, color: Rgba8) -> Result<u32, SceneError> {
        self.push(Primitive::FillRect { bounds, color })
    }

    pub fn glyph(&mut self, glyph: GlyphPrimitive) -> Result<u32, SceneError> {
        self.push(Primitive::Glyph(glyph))
    }

    #[must_use]
    pub fn finish(self) -> Scene {
        Scene {
            revision: self.revision,
            viewport: self.viewport,
            primitives: self.primitives,
            damage: self.damage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_font::{
        GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphMetrics, GlyphRasterKey, RasterizedGlyph,
    };

    fn atlas_entry(glyph: u32) -> AtlasEntry {
        let key = GlyphRasterKey::new(
            yu_font::FontFaceId::from_raw(1),
            yu_font::GlyphId::from_raw(glyph),
            12.0,
        )
        .expect("key");
        let bitmap = GlyphBitmap::new(2, 3, 2, vec![255; 6]).expect("bitmap");
        let metrics = GlyphMetrics::new(1.0, 9.0, 8.0).expect("metrics");
        let glyph = RasterizedGlyph::new(key, metrics, bitmap);
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        atlas.insert(glyph).expect("atlas entry")
    }

    #[test]
    fn damage_merges_touching_rectangles_and_collapses_over_budget() {
        let mut damage = DamageSet::new(2).expect("budget");
        damage
            .add(Rect::new(0.0, 0.0, 10.0, 10.0).expect("rect"))
            .expect("damage");
        damage
            .add(Rect::new(10.0, 0.0, 4.0, 4.0).expect("touching rect"))
            .expect("damage");
        assert_eq!(damage.rects().len(), 1);
        assert_eq!(damage.bounds().expect("bounds").width(), 14.0);

        damage
            .add(Rect::new(100.0, 100.0, 2.0, 2.0).expect("rect"))
            .expect("damage");
        damage
            .add(Rect::new(200.0, 200.0, 2.0, 2.0).expect("rect"))
            .expect("damage");
        assert_eq!(damage.rects().len(), 1);
        assert_eq!(damage.bounds().expect("bounds").right(), 202.0);
    }

    #[test]
    fn scene_keeps_revision_order_and_glyph_bounds() {
        let viewport = Rect::new(0.0, 0.0, 640.0, 480.0).expect("viewport");
        let mut builder = SceneBuilder::new(Revision::new(7), viewport)
            .expect("builder")
            .with_damage_budget(8)
            .expect("damage budget");
        let glyph = GlyphPrimitive::new(atlas_entry(1), Point::new(10.0, 20.0), Rgba8::white());
        builder
            .fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0).expect("rect"), Rgba8::black())
            .expect("rect primitive");
        builder.glyph(glyph).expect("glyph primitive");
        let scene = builder.finish();
        assert_eq!(scene.revision(), Revision::new(7));
        assert_eq!(scene.primitives().len(), 2);
        assert_eq!(scene.primitives()[1], Primitive::Glyph(glyph));
        let bounds = glyph.bounds();
        assert_eq!(bounds.x(), 11.0);
        assert_eq!(bounds.y(), 11.0);
        assert_eq!(bounds.width(), 2.0);
        assert_eq!(bounds.height(), 3.0);
    }

    #[test]
    fn invalid_geometry_is_rejected_before_scene_publication() {
        assert!(Rect::new(0.0, 0.0, f32::NAN, 1.0).is_err());
        assert!(Point::new(f32::INFINITY, 0.0).validate().is_err());
        assert_eq!(DamageSet::new(0), Err(SceneError::InvalidDamageBudget));
    }
}
