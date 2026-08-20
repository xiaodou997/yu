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

use yu_assets::{EmbeddedRenderPayload, EmbeddedRenderPublication};
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

/// An owned SVG payload that a concrete backend may upload or compile for a
/// single render plan. The source range and resource identity remain attached
/// so a platform can reject stale work without consulting the Markdown tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedSvgUpload {
    resource: u64,
    generation: u64,
    kind: u8,
    source: yu_core::TextRange,
    width: u32,
    height: u32,
    markup: Arc<str>,
}

impl EmbeddedSvgUpload {
    #[must_use]
    pub const fn resource(&self) -> u64 {
        self.resource
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn kind(&self) -> u8 {
        self.kind
    }

    #[must_use]
    pub const fn source(&self) -> yu_core::TextRange {
        self.source
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
    pub fn markup(&self) -> &str {
        &self.markup
    }
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
    /// An image resource reference. If the backend has not published the
    /// resource yet, it must draw `fallback` instead of blocking the render
    /// thread or failing the whole frame.
    Image {
        resource: u64,
        bounds: Rect,
        fallback: Rgba8,
    },
    /// A published SVG resource reference. The markup itself is carried by
    /// [`EmbeddedSvgUpload`], while this copyable command stays small enough
    /// for native command buffers and retains a safe fallback color.
    EmbeddedSvg {
        resource: u64,
        generation: u64,
        kind: u8,
        bounds: Rect,
        width: u32,
        height: u32,
        fallback: Rgba8,
    },
}

/// A complete render preparation result for one source revision.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderPlan {
    revision: Revision,
    viewport: Rect,
    /// glyph atlas 相对逻辑坐标的采样倍率。
    ///
    /// 字形按 `font_size × raster_scale` 栅格化，因此 atlas 中的矩形是物理
    /// 像素。后端必须除回这个倍率才能得到逻辑坐标的目标矩形——否则在 Retina
    /// 上要么尺寸翻倍，要么把 1x 纹理拉伸到 2x 而发虚。
    raster_scale: f32,
    damage: Vec<Rect>,
    uploads: Vec<AtlasPageUpload>,
    embedded_uploads: Vec<EmbeddedSvgUpload>,
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
    pub const fn raster_scale(&self) -> f32 {
        self.raster_scale
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
    pub fn embedded_uploads(&self) -> &[EmbeddedSvgUpload] {
        &self.embedded_uploads
    }

    #[must_use]
    pub fn commands(&self) -> &[RenderCommand] {
        &self.commands
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.uploads.is_empty() && self.embedded_uploads.is_empty()
    }
}

/// Errors raised while converting a scene and atlas into a render plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    Atlas(AtlasError),
    MissingAtlasEntry(GlyphRasterKey),
    StaleAtlasEntry(GlyphRasterKey),
    MissingEmbeddedPublication {
        resource: u64,
        generation: u64,
    },
    NonSvgEmbeddedPublication {
        resource: u64,
    },
    EmbeddedDimensionsMismatch {
        resource: u64,
        primitive_width: u32,
        primitive_height: u32,
        publication_width: u32,
        publication_height: u32,
    },
    /// atlas 采样倍率必须是有限正数。
    InvalidRasterScale,
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRasterScale => {
                formatter.write_str("atlas raster scale must be a finite positive number")
            }
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
            Self::MissingEmbeddedPublication {
                resource,
                generation,
            } => write!(
                formatter,
                "scene references missing embedded SVG resource {resource:#x} generation {generation}"
            ),
            Self::NonSvgEmbeddedPublication { resource } => write!(
                formatter,
                "embedded resource {resource:#x} is not an SVG publication"
            ),
            Self::EmbeddedDimensionsMismatch {
                resource,
                primitive_width,
                primitive_height,
                publication_width,
                publication_height,
            } => write!(
                formatter,
                "embedded resource {resource:#x} dimensions {primitive_width}x{primitive_height} do not match publication {publication_width}x{publication_height}"
            ),
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
    uploaded_embedded: HashMap<(u64, u64), u64>,
    raster_scale: Option<f32>,
}

impl RenderPlanBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 声明 glyph atlas 相对逻辑坐标的采样倍率。
    ///
    /// 未声明时为 `1.0`，即 atlas 按逻辑尺寸栅格化。在 Retina 上应声明为
    /// backing scale，并按同一倍率栅格化字形，后端才能把 atlas 矩形除回逻辑
    /// 坐标、让纹理与物理像素 1:1 对应。
    pub fn set_raster_scale(&mut self, scale: f32) -> Result<(), RenderError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(RenderError::InvalidRasterScale);
        }
        self.raster_scale = Some(scale);
        Ok(())
    }

    pub fn build(&mut self, scene: &Scene, atlas: &GlyphAtlas) -> Result<RenderPlan, RenderError> {
        self.build_with_embedded(scene, atlas, &[])
    }

    /// Builds a plan after validating and consuming revision-bound embedded
    /// SVG publications. The ordinary [`Self::build`] path deliberately
    /// supplies no publications, so an embedded primitive cannot become
    /// visible by accident before a renderer has crossed this boundary.
    pub fn build_with_embedded(
        &mut self,
        scene: &Scene,
        atlas: &GlyphAtlas,
        publications: &[EmbeddedRenderPublication],
    ) -> Result<RenderPlan, RenderError> {
        let mut uploads = Vec::new();
        let mut embedded_uploads = Vec::new();
        let mut commands = Vec::with_capacity(scene.primitives().len());
        let mut next_pages = self.uploaded_pages.clone();
        let mut next_embedded = self.uploaded_embedded.clone();

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
                Primitive::Image(image) => {
                    commands.push(RenderCommand::Image {
                        resource: image.resource(),
                        bounds: image.bounds(),
                        fallback: image.fallback(),
                    });
                }
                Primitive::EmbeddedSvg(svg) => {
                    let publication = publications.iter().find(|publication| {
                        publication.revision() == scene.revision()
                            && publication.key().fingerprint() == svg.resource()
                            && publication.generation() == svg.generation()
                            && publication.kind().tag() == svg.kind()
                            && publication.source_range() == svg.source()
                    });
                    let publication =
                        publication.ok_or(RenderError::MissingEmbeddedPublication {
                            resource: svg.resource(),
                            generation: svg.generation(),
                        })?;
                    let EmbeddedRenderPayload::Svg { dimensions, markup } = publication.payload()
                    else {
                        return Err(RenderError::NonSvgEmbeddedPublication {
                            resource: svg.resource(),
                        });
                    };
                    if dimensions.width() != svg.width() || dimensions.height() != svg.height() {
                        return Err(RenderError::EmbeddedDimensionsMismatch {
                            resource: svg.resource(),
                            primitive_width: svg.width(),
                            primitive_height: svg.height(),
                            publication_width: dimensions.width(),
                            publication_height: dimensions.height(),
                        });
                    }
                    let upload_key = (svg.resource(), svg.generation());
                    if next_embedded.get(&upload_key).copied()
                        != Some(publication.key().fingerprint())
                    {
                        embedded_uploads.push(EmbeddedSvgUpload {
                            resource: svg.resource(),
                            generation: svg.generation(),
                            kind: svg.kind(),
                            source: svg.source(),
                            width: dimensions.width(),
                            height: dimensions.height(),
                            markup: Arc::clone(markup),
                        });
                        next_embedded.insert(upload_key, publication.key().fingerprint());
                    }
                    commands.push(RenderCommand::EmbeddedSvg {
                        resource: svg.resource(),
                        generation: svg.generation(),
                        kind: svg.kind(),
                        bounds: svg.bounds(),
                        width: dimensions.width(),
                        height: dimensions.height(),
                        fallback: svg.fallback(),
                    });
                }
                Primitive::BlockQuote(quote) => {
                    commands.push(RenderCommand::FillRect {
                        bounds: quote.bounds(),
                        color: quote.color(),
                    });
                }
                Primitive::Table(table) => {
                    // Table roles remain available in the retained scene for
                    // native selection/accessibility consumers. The current
                    // backend-neutral renderer uses the existing solid-fill
                    // command until a dedicated table pipeline is needed.
                    commands.push(RenderCommand::FillRect {
                        bounds: table.bounds(),
                        color: table.color(),
                    });
                }
                Primitive::TaskCheckbox(task) => {
                    commands.push(RenderCommand::FillRect {
                        bounds: task.bounds(),
                        color: task.color(),
                    });
                }
                Primitive::EditorDecoration(decoration) => {
                    commands.push(RenderCommand::FillRect {
                        bounds: decoration.bounds(),
                        color: decoration.color(),
                    });
                }
            }
        }

        self.uploaded_pages = next_pages;
        self.uploaded_embedded = next_embedded;
        Ok(RenderPlan {
            revision: scene.revision(),
            viewport: scene.viewport(),
            raster_scale: self.raster_scale.unwrap_or(1.0),
            damage: scene.damage().rects().to_vec(),
            uploads,
            embedded_uploads,
            commands,
        })
    }

    pub fn invalidate_page(&mut self, page: u32) {
        self.uploaded_pages.remove(&page);
    }

    pub fn reset(&mut self) {
        self.uploaded_pages.clear();
        self.uploaded_embedded.clear();
    }

    #[must_use]
    pub fn uploaded_page_count(&self) -> usize {
        self.uploaded_pages.len()
    }

    #[must_use]
    pub fn uploaded_embedded_count(&self) -> usize {
        self.uploaded_embedded.len()
    }

    pub fn invalidate_embedded(&mut self, resource: u64, generation: u64) {
        self.uploaded_embedded.remove(&(resource, generation));
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
    use std::sync::Arc;
    use yu_assets::{EmbeddedRenderPayload, EmbeddedRenderRequest, EmbeddedResourceKind};
    use yu_core::{ByteOffset, TextRange};
    use yu_font::{
        AtlasEntry, FontDatabase, FontFaceId, FontFaceSpec, FontRequest, FontShaper,
        GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics, GlyphRasterKey, RasterizedGlyph,
    };
    use yu_layout::{LayoutConfig, LayoutSnapshot};
    use yu_projection::Projection;
    use yu_scene::{
        EmbeddedSvgPrimitive, GlyphPrimitive, ImagePrimitive, Point, SceneBuilder, SceneError,
        TablePrimitiveRole, TableSceneStyle, TaskCheckboxPrimitive, TaskCheckboxPrimitiveRole,
        ViewportBlockGeometry, ViewportSceneInput,
    };
    use yu_text::TextBuffer;

    #[derive(Debug)]
    struct FakeUploadError;

    impl fmt::Display for FakeUploadError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fake upload failed")
        }
    }

    #[derive(Default)]
    struct FakeUploader {
        pages: Vec<(u32, u64)>,
    }

    impl RenderUploader for FakeUploader {
        type Texture = u32;
        type Error = FakeUploadError;

        fn upload_alpha_page(
            &mut self,
            page: &AtlasPageUpload,
        ) -> Result<Self::Texture, Self::Error> {
            self.pages.push((page.page(), page.fingerprint()));
            Ok(page.page())
        }
    }

    fn shaped_layout(font_size: f32) -> LayoutSnapshot {
        let buffer = TextBuffer::new("ab");
        let snapshot = buffer.snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).expect("range");
        let projection = Projection::inline(&snapshot, range).expect("projection");
        let mut database = FontDatabase::new();
        database
            .register(FontFaceSpec::new("Test", 0.5))
            .expect("face");
        let shaper = FontShaper::new(
            Arc::new(database),
            FontRequest::new("Test", font_size).expect("font request"),
        )
        .expect("shaper");
        LayoutSnapshot::from_projection_with_shaper(
            &projection,
            LayoutConfig::new(200.0, 20.0),
            &shaper,
        )
        .expect("layout")
    }

    fn atlas_for_layout(layout: &LayoutSnapshot, font_size: f32) -> GlyphAtlas {
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("config"));
        insert_layout_glyphs(&mut atlas, &[layout], font_size);
        atlas
    }

    fn insert_layout_glyphs(atlas: &mut GlyphAtlas, layouts: &[&LayoutSnapshot], font_size: f32) {
        let mut index = 0_usize;
        for layout in layouts {
            for placement in layout.glyphs() {
                let key = GlyphRasterKey::new(
                    placement.face(),
                    placement.glyph(),
                    font_size * placement.font_scale(),
                )
                .expect("key");
                if atlas.entry(key).is_none() {
                    let value = u8::try_from(index + 1).expect("test glyph value");
                    let bitmap = GlyphBitmap::new(2, 3, 2, vec![value; 6]).expect("bitmap");
                    let metrics = GlyphMetrics::new(0.0, 10.0, 7.0).expect("metrics");
                    atlas
                        .insert(RasterizedGlyph::new(key, metrics, bitmap))
                        .expect("atlas entry");
                }
                index = index.saturating_add(1);
            }
        }
    }

    fn shaped_block_layouts(font_size: f32) -> (LayoutSnapshot, LayoutSnapshot) {
        let buffer = TextBuffer::new("ab\ncd");
        let snapshot = buffer.snapshot();
        let first_range = TextRange::new(ByteOffset::new(0), ByteOffset::new(2)).expect("range");
        let second_range = TextRange::new(ByteOffset::new(3), ByteOffset::new(5)).expect("range");
        let first_projection = Projection::inline(&snapshot, first_range).expect("projection");
        let second_projection = Projection::inline(&snapshot, second_range).expect("projection");
        let mut database = FontDatabase::new();
        database
            .register(FontFaceSpec::new("Test", 0.5))
            .expect("face");
        let shaper = FontShaper::new(
            Arc::new(database),
            FontRequest::new("Test", font_size).expect("font request"),
        )
        .expect("shaper");
        let config = LayoutConfig::new(200.0, 20.0);
        let first = LayoutSnapshot::from_projection_with_shaper(&first_projection, config, &shaper)
            .expect("first layout");
        let second =
            LayoutSnapshot::from_projection_with_shaper(&second_projection, config, &shaper)
                .expect("second layout");
        (first, second)
    }

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

    #[test]
    fn layout_scene_render_plan_and_fake_uploader_are_revision_bound() {
        let font_size = 14.0;
        let layout = shaped_layout(font_size);
        assert_eq!(layout.glyphs().len(), 2);
        let atlas = atlas_for_layout(&layout, font_size);
        let viewport = Rect::new(0.0, 0.0, 200.0, 80.0).expect("viewport");
        let mut scene_builder = SceneBuilder::new(layout.revision(), viewport).expect("scene");
        assert_eq!(
            scene_builder
                .append_layout(&layout, &atlas, font_size, Rgba8::black())
                .expect("append layout"),
            2
        );
        let scene = scene_builder.finish();
        assert_eq!(scene.primitives().len(), 2);

        let mut plans = RenderPlanBuilder::new();
        let first = plans.build(&scene, &atlas).expect("first plan");
        assert_eq!(first.revision(), layout.revision());
        assert_eq!(first.uploads().len(), 1);
        assert_eq!(first.commands().len(), 2);
        let mut uploader = FakeUploader::default();
        for upload in first.uploads() {
            uploader
                .upload_alpha_page(upload)
                .expect("fake page upload");
        }
        assert_eq!(uploader.pages.len(), 1);
        assert_eq!(uploader.pages[0].0, 0);

        let second = plans.build(&scene, &atlas).expect("second plan");
        assert!(second.uploads().is_empty());
        assert_eq!(second.commands().len(), 2);
        match first.commands()[0] {
            RenderCommand::Glyph { origin, .. } => {
                assert_eq!(origin.x(), layout.glyphs()[0].x());
                assert_eq!(origin.y(), layout.glyphs()[0].y());
            }
            RenderCommand::FillRect { .. }
            | RenderCommand::Image { .. }
            | RenderCommand::EmbeddedSvg { .. } => {
                panic!("expected glyph command")
            }
        }
    }

    #[test]
    fn image_primitives_become_revision_bound_resource_commands() {
        let revision = Revision::new(7);
        let viewport = Rect::new(0.0, 0.0, 320.0, 200.0).expect("viewport");
        let bounds = Rect::new(24.0, 32.0, 120.0, 80.0).expect("image bounds");
        let fallback = Rgba8::new(230, 232, 236, 255);
        let mut scene = SceneBuilder::new(revision, viewport).expect("scene");
        scene
            .image(ImagePrimitive::new(0xfeed_beef, bounds, fallback))
            .expect("image primitive");
        let scene = scene.finish();
        let mut plans = RenderPlanBuilder::new();
        let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas config"));
        let plan = plans.build(&scene, &atlas).expect("image plan");
        assert_eq!(plan.revision(), revision);
        assert_eq!(
            plan.commands(),
            &[RenderCommand::Image {
                resource: 0xfeed_beef,
                bounds,
                fallback,
            }]
        );
    }

    #[test]
    fn published_embedded_svg_becomes_upload_and_revision_bound_command() {
        let revision = Revision::new(7);
        let source = TextRange::new(ByteOffset::new(10), ByteOffset::new(18)).expect("source");
        let request =
            EmbeddedRenderRequest::new(revision, source, EmbeddedResourceKind::Math, "x^2 + y^2")
                .expect("request");
        let mut cache = yu_assets::EmbeddedResourceCache::new();
        let publication = cache
            .publish(
                request,
                revision,
                EmbeddedRenderPayload::svg(640, 320, "<svg viewBox=\"0 0 640 320\"/>")
                    .expect("SVG"),
            )
            .expect("publication");
        let bounds = Rect::new(24.0, 32.0, 160.0, 80.0).expect("bounds");
        let fallback = Rgba8::new(230, 232, 236, 255);
        let primitive = EmbeddedSvgPrimitive::new(
            publication.key().fingerprint(),
            publication.generation(),
            publication.kind().tag(),
            publication.source_range(),
            bounds,
            640,
            320,
            fallback,
        );
        let mut scene = SceneBuilder::new(
            revision,
            Rect::new(0.0, 0.0, 320.0, 200.0).expect("viewport"),
        )
        .expect("scene");
        scene.embedded_svg(primitive).expect("embedded primitive");
        let scene = scene.finish();
        let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas config"));
        let mut plans = RenderPlanBuilder::new();
        let first = plans
            .build_with_embedded(&scene, &atlas, std::slice::from_ref(&publication))
            .expect("first plan");
        assert_eq!(first.embedded_uploads().len(), 1);
        assert_eq!(first.embedded_uploads()[0].resource(), primitive.resource());
        assert_eq!(first.embedded_uploads()[0].source(), source);
        assert_eq!(
            first.embedded_uploads()[0].markup(),
            "<svg viewBox=\"0 0 640 320\"/>"
        );
        assert!(matches!(
            first.commands(),
            [RenderCommand::EmbeddedSvg {
                resource,
                generation,
                kind,
                width: 640,
                height: 320,
                ..
            }] if *resource == primitive.resource()
                && *generation == primitive.generation()
                && *kind == EmbeddedResourceKind::Math.tag()
        ));
        let second = plans
            .build_with_embedded(&scene, &atlas, std::slice::from_ref(&publication))
            .expect("second plan");
        assert!(second.embedded_uploads().is_empty());
        assert_eq!(plans.uploaded_embedded_count(), 1);
    }

    #[test]
    fn embedded_svg_requires_matching_publication_before_it_can_be_rendered() {
        let revision = Revision::new(8);
        let source = TextRange::new(ByteOffset::new(1), ByteOffset::new(3)).expect("source");
        let primitive = EmbeddedSvgPrimitive::new(
            42,
            1,
            EmbeddedResourceKind::Math.tag(),
            source,
            Rect::new(0.0, 0.0, 12.0, 12.0).expect("bounds"),
            12,
            12,
            Rgba8::black(),
        );
        let mut scene =
            SceneBuilder::new(revision, Rect::new(0.0, 0.0, 20.0, 20.0).expect("viewport"))
                .expect("scene");
        scene.embedded_svg(primitive).expect("embedded primitive");
        let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(8, 8, 1).expect("atlas config"));
        let error = RenderPlanBuilder::new()
            .build(&scene.finish(), &atlas)
            .expect_err("missing publication");
        assert_eq!(
            error,
            RenderError::MissingEmbeddedPublication {
                resource: 42,
                generation: 1,
            }
        );
    }

    #[test]
    fn embedded_svg_rejects_intrinsic_dimension_drift() {
        let revision = Revision::new(9);
        let source = TextRange::new(ByteOffset::new(0), ByteOffset::new(2)).expect("source");
        let request = EmbeddedRenderRequest::new(revision, source, EmbeddedResourceKind::Math, "x")
            .expect("request");
        let mut cache = yu_assets::EmbeddedResourceCache::new();
        let publication = cache
            .publish(
                request,
                revision,
                EmbeddedRenderPayload::svg(20, 10, "<svg/>").expect("SVG"),
            )
            .expect("publication");
        let primitive = EmbeddedSvgPrimitive::new(
            publication.key().fingerprint(),
            publication.generation(),
            publication.kind().tag(),
            source,
            Rect::new(0.0, 0.0, 20.0, 10.0).expect("bounds"),
            21,
            10,
            Rgba8::black(),
        );
        let mut builder =
            SceneBuilder::new(revision, Rect::new(0.0, 0.0, 30.0, 20.0).expect("viewport"))
                .expect("scene");
        builder.embedded_svg(primitive).expect("primitive");
        let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(8, 8, 1).expect("atlas config"));
        let error = RenderPlanBuilder::new()
            .build_with_embedded(
                &builder.finish(),
                &atlas,
                std::slice::from_ref(&publication),
            )
            .expect_err("dimension drift");
        assert_eq!(
            error,
            RenderError::EmbeddedDimensionsMismatch {
                resource: publication.key().fingerprint(),
                primitive_width: 21,
                primitive_height: 10,
                publication_width: 20,
                publication_height: 10,
            }
        );
    }

    #[test]
    fn table_scene_primitives_become_solid_fill_commands_without_losing_scene_roles() {
        let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("table block");
        let projection = yu_projection::BlockProjection::from_block_with_definitions(
            &snapshot,
            block,
            markdown.reference_definitions(),
        )
        .expect("table projection");
        let layout =
            LayoutSnapshot::from_block_projection(&projection, LayoutConfig::new(20.0, 2.0))
                .expect("table layout");
        let table = layout.table().expect("table layout");
        let selected = table.cells()[2].source();
        let style = TableSceneStyle::new(
            1.0,
            Rgba8::new(150, 155, 165, 255),
            Some(Rgba8::new(235, 238, 244, 255)),
            Some(Rgba8::new(210, 225, 255, 255)),
        );
        let mut builder = SceneBuilder::new(
            layout.revision(),
            Rect::new(0.0, 0.0, 40.0, 20.0).expect("viewport"),
        )
        .expect("scene");
        builder
            .append_table_with_selection(table, Point::new(3.0, 4.0), style, Some(selected))
            .expect("table scene");
        let scene = builder.finish();
        assert!(scene.primitives().iter().any(|primitive| matches!(
            primitive,
            yu_scene::Primitive::Table(table)
                if table.role() == TablePrimitiveRole::SelectionFill
        )));
        let mut plans = RenderPlanBuilder::new();
        let plan = plans
            .build(
                &scene,
                &GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas")),
            )
            .expect("table render plan");
        assert_eq!(plan.commands().len(), scene.primitives().len());
        assert!(
            plan.commands()
                .iter()
                .all(|command| matches!(command, RenderCommand::FillRect { .. }))
        );
    }

    #[test]
    fn task_checkbox_layers_lower_to_solid_render_commands_in_order() {
        let revision = Revision::new(13);
        let viewport = Rect::new(0.0, 0.0, 320.0, 200.0).expect("viewport");
        let source = TextRange::new(ByteOffset::new(2), ByteOffset::new(5)).expect("marker");
        let outer_bounds = Rect::new(12.0, 18.0, 14.0, 14.0).expect("outer");
        let check_bounds = Rect::new(16.0, 23.0, 3.0, 3.0).expect("check");
        let outer_color = Rgba8::new(38, 111, 219, 255);
        let mut scene = SceneBuilder::new(revision, viewport).expect("scene");
        scene
            .task_checkbox(TaskCheckboxPrimitive::new(
                source,
                outer_bounds,
                outer_color,
                TaskCheckboxPrimitiveRole::Border,
            ))
            .expect("outer");
        scene
            .task_checkbox(TaskCheckboxPrimitive::new(
                source,
                check_bounds,
                Rgba8::white(),
                TaskCheckboxPrimitiveRole::Check,
            ))
            .expect("check");
        let scene = scene.finish();
        let mut plans = RenderPlanBuilder::new();
        let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas"));
        let plan = plans.build(&scene, &atlas).expect("render plan");
        assert_eq!(
            plan.commands(),
            &[
                RenderCommand::FillRect {
                    bounds: outer_bounds,
                    color: outer_color,
                },
                RenderCommand::FillRect {
                    bounds: check_bounds,
                    color: Rgba8::white(),
                },
            ]
        );
    }

    #[test]
    fn viewport_block_layout_is_translated_to_document_space_atomically() {
        let font_size = 14.0;
        let layout = shaped_layout(font_size);
        let atlas = atlas_for_layout(&layout, font_size);
        let geometry = yu_scene::ViewportBlockGeometry::new(
            layout.revision(),
            3,
            layout.source_range(),
            40.0,
            20.0,
            true,
            2,
        )
        .expect("geometry");
        let viewport = Rect::new(0.0, 0.0, 200.0, 80.0).expect("viewport");
        let mut builder = SceneBuilder::new(layout.revision(), viewport).expect("scene");
        assert_eq!(
            builder
                .append_layout_at_block(geometry, &layout, &atlas, font_size, Rgba8::black())
                .expect("append block layout"),
            2
        );
        match builder.finish().primitives()[0] {
            Primitive::Glyph(glyph) => {
                assert_eq!(glyph.origin().x(), layout.glyphs()[0].x());
                assert_eq!(glyph.origin().y(), layout.glyphs()[0].y() + 40.0);
            }
            Primitive::FillRect { .. }
            | Primitive::Image(_)
            | Primitive::EmbeddedSvg(_)
            | Primitive::BlockQuote(_)
            | Primitive::Table(_)
            | Primitive::TaskCheckbox(_)
            | Primitive::EditorDecoration(_) => {
                panic!("expected glyph primitive")
            }
        }

        let mut stale_builder = SceneBuilder::new(Revision::new(9), viewport).expect("scene");
        assert!(matches!(
            stale_builder.append_layout_at_block(
                geometry,
                &layout,
                &atlas,
                font_size,
                Rgba8::black()
            ),
            Err(SceneError::ViewportRevisionMismatch { .. })
        ));
        assert!(stale_builder.finish().primitives().is_empty());

        let mismatched = yu_scene::ViewportBlockGeometry::new(
            layout.revision(),
            3,
            yu_core::TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(1))
                .expect("range"),
            40.0,
            20.0,
            true,
            2,
        )
        .expect("geometry");
        let mut mismatch_builder = SceneBuilder::new(layout.revision(), viewport).expect("scene");
        assert_eq!(
            mismatch_builder.append_layout_at_block(
                mismatched,
                &layout,
                &atlas,
                font_size,
                Rgba8::black()
            ),
            Err(SceneError::ViewportSourceMismatch)
        );
        assert!(mismatch_builder.finish().primitives().is_empty());
    }

    #[test]
    fn viewport_scene_batch_translates_all_blocks_before_publishing() {
        let font_size = 14.0;
        let (first, second) = shaped_block_layouts(font_size);
        assert_eq!(first.revision(), second.revision());
        assert_eq!(first.source_range().start().get(), 0);
        assert_eq!(second.source_range().start().get(), 3);

        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("config"));
        insert_layout_glyphs(&mut atlas, &[&first, &second], font_size);
        let first_geometry = ViewportBlockGeometry::new(
            first.revision(),
            0,
            first.source_range(),
            0.0,
            20.0,
            true,
            1,
        )
        .expect("first geometry");
        let second_geometry = ViewportBlockGeometry::new(
            second.revision(),
            1,
            second.source_range(),
            20.0,
            20.0,
            true,
            1,
        )
        .expect("second geometry");
        let input = ViewportSceneInput::new(
            first.revision(),
            0..2,
            40.0,
            vec![first_geometry, second_geometry],
        )
        .expect("viewport input");
        let viewport = Rect::new(0.0, 0.0, 200.0, 80.0).expect("viewport");
        let mut builder = SceneBuilder::new(first.revision(), viewport).expect("scene");
        assert_eq!(
            builder
                .append_viewport(
                    &input,
                    &[&first, &second],
                    &atlas,
                    font_size,
                    Rgba8::black(),
                )
                .expect("append viewport"),
            4
        );
        let scene = builder.finish();
        assert_eq!(scene.primitives().len(), 4);
        let origins = scene
            .primitives()
            .iter()
            .map(|primitive| match primitive {
                Primitive::Glyph(glyph) => glyph.origin(),
                Primitive::FillRect { .. }
                | Primitive::Image(_)
                | Primitive::EmbeddedSvg(_)
                | Primitive::BlockQuote(_)
                | Primitive::Table(_)
                | Primitive::TaskCheckbox(_)
                | Primitive::EditorDecoration(_) => {
                    panic!("expected glyph primitive")
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(origins[0].y(), first.glyphs()[0].y());
        assert_eq!(origins[1].y(), first.glyphs()[1].y());
        assert_eq!(origins[2].y(), second.glyphs()[0].y() + 20.0);
        assert_eq!(origins[3].y(), second.glyphs()[1].y() + 20.0);
    }

    #[test]
    fn viewport_scene_batch_rejects_any_bad_block_without_a_prefix() {
        let font_size = 14.0;
        let (first, second) = shaped_block_layouts(font_size);
        let viewport = Rect::new(0.0, 0.0, 200.0, 80.0).expect("viewport");
        let first_geometry = ViewportBlockGeometry::new(
            first.revision(),
            0,
            first.source_range(),
            0.0,
            20.0,
            true,
            1,
        )
        .expect("first geometry");
        let second_geometry = ViewportBlockGeometry::new(
            second.revision(),
            1,
            second.source_range(),
            20.0,
            20.0,
            true,
            1,
        )
        .expect("second geometry");
        let input = ViewportSceneInput::new(
            first.revision(),
            0..2,
            40.0,
            vec![first_geometry, second_geometry],
        )
        .expect("viewport input");

        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("config"));
        insert_layout_glyphs(&mut atlas, &[&first], font_size);
        let mut missing_builder = SceneBuilder::new(first.revision(), viewport).expect("scene");
        assert!(matches!(
            missing_builder.append_viewport(
                &input,
                &[&first, &second],
                &atlas,
                font_size,
                Rgba8::black(),
            ),
            Err(SceneError::MissingGlyphAtlas(_))
        ));
        assert!(missing_builder.finish().primitives().is_empty());

        let mut limited_builder = SceneBuilder::new(first.revision(), viewport)
            .expect("scene")
            .with_primitive_limit(3);
        let mut full_atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("config"));
        insert_layout_glyphs(&mut full_atlas, &[&first, &second], font_size);
        assert_eq!(
            limited_builder.append_viewport(
                &input,
                &[&first, &second],
                &full_atlas,
                font_size,
                Rgba8::black(),
            ),
            Err(SceneError::PrimitiveLimitExceeded)
        );
        assert!(limited_builder.finish().primitives().is_empty());

        let mut stale_builder = SceneBuilder::new(Revision::new(9), viewport).expect("scene");
        assert!(matches!(
            stale_builder.append_viewport(
                &input,
                &[&first, &second],
                &full_atlas,
                font_size,
                Rgba8::black(),
            ),
            Err(SceneError::ViewportRevisionMismatch { .. })
        ));
        assert!(stale_builder.finish().primitives().is_empty());

        let source_mismatch = ViewportBlockGeometry::new(
            first.revision(),
            0,
            TextRange::new(ByteOffset::new(0), ByteOffset::new(1)).expect("range"),
            0.0,
            20.0,
            true,
            1,
        )
        .expect("mismatch geometry");
        let mismatch_input = ViewportSceneInput::new(
            first.revision(),
            0..2,
            40.0,
            vec![source_mismatch, second_geometry],
        )
        .expect("mismatch input");
        let mut mismatch_builder = SceneBuilder::new(first.revision(), viewport).expect("scene");
        assert_eq!(
            mismatch_builder.append_viewport(
                &mismatch_input,
                &[&first, &second],
                &full_atlas,
                font_size,
                Rgba8::black(),
            ),
            Err(SceneError::ViewportSourceMismatch)
        );
        assert!(mismatch_builder.finish().primitives().is_empty());
    }

    #[test]
    fn missing_layout_atlas_is_atomic_and_revision_mismatch_is_rejected() {
        let font_size = 14.0;
        let layout = shaped_layout(font_size);
        let viewport = Rect::new(0.0, 0.0, 200.0, 80.0).expect("viewport");
        let empty_atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("config"));
        let mut missing_builder =
            SceneBuilder::new(layout.revision(), viewport).expect("scene builder");
        let error = missing_builder
            .append_layout(&layout, &empty_atlas, font_size, Rgba8::black())
            .expect_err("missing atlas entry");
        assert!(matches!(error, SceneError::MissingGlyphAtlas(_)));
        assert!(missing_builder.finish().primitives().is_empty());

        let mut stale_builder = SceneBuilder::new(
            Revision::new(layout.revision().get().saturating_add(1)),
            viewport,
        )
        .expect("stale scene builder");
        let atlas = atlas_for_layout(&layout, font_size);
        assert!(matches!(
            stale_builder.append_layout(&layout, &atlas, font_size, Rgba8::black()),
            Err(SceneError::RevisionMismatch { .. })
        ));
    }
}
