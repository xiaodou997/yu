#![forbid(unsafe_code)]

//! Backend-neutral render preparation for Yu Editor.
//!
//! `yu-render` turns a revision-bound [`yu_scene::Scene`] and a CPU
//! [`yu_font::GlyphAtlas`] into an owned [`RenderPlan`]. It does not create a
//! window, GPU device, texture, command encoder or event loop. A future
//! Metal/wgpu backend can consume [`AtlasPageUpload`] through
//! [`RenderUploader`] without changing scene or editor contracts.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::Revision;
use yu_font::{AtlasError, GlyphAtlas, GlyphRasterKey};
use yu_scene::{Point, Primitive, Rect, Rgba8, Scene};

/// A page upload containing owned alpha pixels ready for a backend texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtlasPageUpload {
    page: u32,
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
    fingerprint: u64,
}

impl AtlasPageUpload {
    #[must_use]
    pub const fn page(&self) -> u32 {
        self.page
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
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

/// Backend-independent draw commands in painter order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RenderCommand {
    FillRect {
        bounds: Rect,
        color: Rgba8,
    },
    Glyph {
        page: Option<u32>,
        rect: yu_font::AtlasRect,
        origin: Point,
        metrics: yu_font::GlyphMetrics,
        color: Rgba8,
    },
}

/// A complete render preparation result for one source revision.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderPlan {
    revision: Revision,
    viewport: Rect,
    damage: Vec<Rect>,
    uploads: Vec<AtlasPageUpload>,
    commands: Vec<RenderCommand>,
}

impl RenderPlan {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn viewport(&self) -> Rect {
        self.viewport
    }

    #[must_use]
    pub fn damage(&self) -> &[Rect] {
        &self.damage
    }

    #[must_use]
    pub fn uploads(&self) -> &[AtlasPageUpload] {
        &self.uploads
    }

    #[must_use]
    pub fn commands(&self) -> &[RenderCommand] {
        &self.commands
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.uploads.is_empty()
    }
}

/// Errors raised while converting a scene and atlas into a render plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    Atlas(AtlasError),
    MissingAtlasEntry(GlyphRasterKey),
    StaleAtlasEntry(GlyphRasterKey),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Atlas(error) => write!(formatter, "glyph atlas query failed: {error}"),
            Self::MissingAtlasEntry(key) => {
                write!(
                    formatter,
                    "scene references missing atlas glyph {}",
                    key.glyph().get()
                )
            }
            Self::StaleAtlasEntry(key) => {
                write!(
                    formatter,
                    "scene references stale atlas glyph {}",
                    key.glyph().get()
                )
            }
        }
    }
}

impl Error for RenderError {}

/// A renderer-side cache key for an uploaded page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PageFingerprint {
    width: u32,
    height: u32,
    hash: u64,
}

/// Builds plans and suppresses duplicate atlas page uploads until page bytes
/// change. The cache contains no GPU handles and can be reset on device loss.
#[derive(Clone, Debug, Default)]
pub struct RenderPlanBuilder {
    uploaded_pages: HashMap<u32, PageFingerprint>,
}

impl RenderPlanBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(&mut self, scene: &Scene, atlas: &GlyphAtlas) -> Result<RenderPlan, RenderError> {
        let mut uploads = Vec::new();
        let mut commands = Vec::with_capacity(scene.primitives().len());
        let mut next_pages = self.uploaded_pages.clone();

        for primitive in scene.primitives().iter().copied() {
            match primitive {
                Primitive::FillRect { bounds, color } => {
                    commands.push(RenderCommand::FillRect { bounds, color });
                }
                Primitive::Glyph(glyph) => {
                    let key = glyph.key();
                    let entry = atlas
                        .entry(key)
                        .ok_or(RenderError::MissingAtlasEntry(key))?;
                    if entry != glyph.atlas() {
                        return Err(RenderError::StaleAtlasEntry(key));
                    }
                    let page = entry.page();
                    if let Some(page) = page {
                        let width = atlas.config().page_width();
                        let height = atlas.config().page_height();
                        let pixels = atlas.page_pixels(page).map_err(RenderError::Atlas)?;
                        let fingerprint = PageFingerprint {
                            width,
                            height,
                            hash: hash_page(page, width, height, pixels),
                        };
                        if next_pages.get(&page).copied() != Some(fingerprint) {
                            uploads.push(AtlasPageUpload {
                                page,
                                width,
                                height,
                                pixels: Arc::from(pixels.to_vec().into_boxed_slice()),
                                fingerprint: fingerprint.hash,
                            });
                            next_pages.insert(page, fingerprint);
                        }
                    }
                    commands.push(RenderCommand::Glyph {
                        page,
                        rect: entry.rect(),
                        origin: glyph.origin(),
                        metrics: entry.metrics(),
                        color: glyph.color(),
                    });
                }
            }
        }

        self.uploaded_pages = next_pages;
        Ok(RenderPlan {
            revision: scene.revision(),
            viewport: scene.viewport(),
            damage: scene.damage().rects().to_vec(),
            uploads,
            commands,
        })
    }

    pub fn invalidate_page(&mut self, page: u32) {
        self.uploaded_pages.remove(&page);
    }

    pub fn reset(&mut self) {
        self.uploaded_pages.clear();
    }

    #[must_use]
    pub fn uploaded_page_count(&self) -> usize {
        self.uploaded_pages.len()
    }
}

/// The only texture operation a concrete backend needs to implement for the
/// atlas stage. The returned handle is owned by the backend, not by the scene.
pub trait RenderUploader {
    type Texture;
    type Error: fmt::Display;

    fn upload_alpha_page(&mut self, page: &AtlasPageUpload) -> Result<Self::Texture, Self::Error>;
}

fn hash_page(page: u32, width: u32, height: u32, pixels: &[u8]) -> u64 {
    let mut hash = 1_469_598_103_934_665_603_u64;
    hash = hash.wrapping_mul(1_099_511_628_211_u64) ^ u64::from(page);
    hash = hash
        .wrapping_mul(1_099_511_628_211_u64)
        .wrapping_add(u64::from(width));
    hash = hash
        .wrapping_mul(1_099_511_628_211_u64)
        .wrapping_add(u64::from(height));
    for byte in pixels {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211_u64);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_font::{
        AtlasEntry, FontFaceId, GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics,
        GlyphRasterKey, RasterizedGlyph,
    };
    use yu_scene::{GlyphPrimitive, SceneBuilder, SceneError};

    fn make_glyph(atlas: &mut GlyphAtlas, glyph: u32, width: u32, height: u32) -> AtlasEntry {
        let key = GlyphRasterKey::new(FontFaceId::from_raw(2), GlyphId::from_raw(glyph), 14.0)
            .expect("key");
        let bitmap = GlyphBitmap::new(
            width,
            height,
            width,
            vec![
                u8::try_from(glyph).expect("test glyph fits");
                usize::try_from(width * height).expect("pixels fit")
            ],
        )
        .expect("bitmap");
        let metrics = GlyphMetrics::new(0.0, height as f32, width as f32).expect("metrics");
        atlas
            .insert(RasterizedGlyph::new(key, metrics, bitmap))
            .expect("atlas entry")
    }

    fn scene_with_glyph(revision: Revision, entry: AtlasEntry) -> yu_scene::Scene {
        let viewport = Rect::new(0.0, 0.0, 80.0, 40.0).expect("viewport");
        let mut builder = SceneBuilder::new(revision, viewport).expect("builder");
        builder
            .glyph(GlyphPrimitive::new(
                entry,
                Point::new(4.0, 20.0),
                Rgba8::white(),
            ))
            .expect("glyph primitive");
        builder.finish()
    }

    #[test]
    fn first_plan_uploads_one_shared_page_and_second_plan_reuses_it() {
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        let first = make_glyph(&mut atlas, 1, 2, 3);
        let second = make_glyph(&mut atlas, 2, 2, 3);
        let viewport = Rect::new(0.0, 0.0, 80.0, 40.0).expect("viewport");
        let mut builder = SceneBuilder::new(Revision::new(4), viewport).expect("builder");
        builder
            .glyph(GlyphPrimitive::new(
                first,
                Point::new(4.0, 20.0),
                Rgba8::white(),
            ))
            .expect("first glyph");
        builder
            .glyph(GlyphPrimitive::new(
                second,
                Point::new(12.0, 20.0),
                Rgba8::white(),
            ))
            .expect("second glyph");
        let scene = builder.finish();
        let mut plans = RenderPlanBuilder::new();
        let first_plan = plans.build(&scene, &atlas).expect("first plan");
        assert_eq!(first_plan.revision(), Revision::new(4));
        assert_eq!(first_plan.viewport(), viewport);
        assert_eq!(first_plan.uploads().len(), 1);
        assert_eq!(first_plan.commands().len(), 2);
        assert_eq!(plans.uploaded_page_count(), 1);
        let second_plan = plans.build(&scene, &atlas).expect("second plan");
        assert!(second_plan.uploads().is_empty());
        assert_eq!(second_plan.commands().len(), 2);
    }

    #[test]
    fn page_mutation_invalidates_fingerprint_and_empty_glyph_needs_no_upload() {
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        let first = make_glyph(&mut atlas, 1, 2, 3);
        let mut plans = RenderPlanBuilder::new();
        let first_scene = scene_with_glyph(Revision::new(1), first);
        assert_eq!(
            plans
                .build(&first_scene, &atlas)
                .expect("plan")
                .uploads()
                .len(),
            1
        );

        let second = make_glyph(&mut atlas, 2, 2, 3);
        let second_scene = scene_with_glyph(Revision::new(2), second);
        assert_eq!(
            plans
                .build(&second_scene, &atlas)
                .expect("plan")
                .uploads()
                .len(),
            1
        );

        let empty_key = GlyphRasterKey::new(FontFaceId::from_raw(2), GlyphId::from_raw(3), 14.0)
            .expect("empty key");
        let empty = atlas
            .insert(RasterizedGlyph::new(
                empty_key,
                GlyphMetrics::new(0.0, 0.0, 4.0).expect("metrics"),
                GlyphBitmap::new(0, 0, 0, Vec::<u8>::new()).expect("empty bitmap"),
            ))
            .expect("empty entry");
        let empty_scene = scene_with_glyph(Revision::new(3), empty);
        let empty_plan = plans.build(&empty_scene, &atlas).expect("empty plan");
        assert!(empty_plan.uploads().is_empty());
        assert_eq!(empty_plan.commands().len(), 1);
        assert!(matches!(
            empty_plan.commands()[0],
            RenderCommand::Glyph { page: None, .. }
        ));
    }

    #[test]
    fn stale_scene_entry_is_rejected() {
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        let entry = make_glyph(&mut atlas, 1, 2, 3);
        let scene = scene_with_glyph(Revision::INITIAL, entry);
        let empty_atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        let error = RenderPlanBuilder::new()
            .build(&scene, &empty_atlas)
            .expect_err("missing atlas entry");
        assert_eq!(error, RenderError::MissingAtlasEntry(entry.key()));
    }

    #[test]
    fn invalid_scene_budget_is_still_a_scene_layer_error() {
        assert_eq!(
            SceneBuilder::new(
                Revision::INITIAL,
                Rect::new(0.0, 0.0, 1.0, 1.0).expect("viewport"),
            )
            .expect("builder")
            .with_damage_budget(0)
            .expect_err("invalid budget"),
            SceneError::InvalidDamageBudget
        );
    }
}
