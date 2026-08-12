#![forbid(unsafe_code)]

//! Product-facing integration between the editor model and retained scenes.
//!
//! `yu-editor` owns canonical source, Markdown, viewport measurements and
//! block-local layout caches. This crate is the first layer allowed to combine
//! those results with `yu-scene`/`yu-render`; neither side needs to depend on
//! the other.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::Revision;
use yu_editor::{EditorDocument, EditorDocumentError, ShapingProvider, ViewportRect};
use yu_font::GlyphAtlas;
use yu_render::{RenderError, RenderPlan, RenderPlanBuilder};
use yu_scene::{
    Rect, Rgba8, Scene, SceneBuilder, SceneError, ViewportBlockGeometry, ViewportSceneInput,
};

/// A validated scene together with the viewport metadata that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportSceneFrame {
    input: ViewportSceneInput,
    scene: Scene,
}

impl ViewportSceneFrame {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.scene.revision()
    }

    #[must_use]
    pub fn input(&self) -> &ViewportSceneInput {
        &self.input
    }

    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    #[must_use]
    pub fn into_parts(self) -> (ViewportSceneInput, Scene) {
        (self.input, self.scene)
    }
}

/// A scene and its backend-neutral render plan published as one revision-bound
/// frame. Keeping them together prevents a new scene from being paired with an
/// older command list (or vice versa) at a platform boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportRenderFrame {
    scene: ViewportSceneFrame,
    plan: RenderPlan,
}

impl ViewportRenderFrame {
    pub fn new(scene: ViewportSceneFrame, plan: RenderPlan) -> Result<Self, ViewportFrameError> {
        if scene.revision() != plan.revision() {
            return Err(ViewportFrameError::RevisionMismatch {
                scene: scene.revision(),
                plan: plan.revision(),
            });
        }
        Ok(Self { scene, plan })
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.scene.revision()
    }

    #[must_use]
    pub fn scene(&self) -> &ViewportSceneFrame {
        &self.scene
    }

    #[must_use]
    pub fn plan(&self) -> &RenderPlan {
        &self.plan
    }

    #[must_use]
    pub fn into_parts(self) -> (ViewportSceneFrame, RenderPlan) {
        (self.scene, self.plan)
    }
}

/// Immutable inputs shared by one scene/render frame build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRenderConfig {
    viewport: ViewportRect,
    font_size: f32,
    scene_viewport: Rect,
    color: Rgba8,
}

impl ViewportRenderConfig {
    #[must_use]
    pub const fn new(
        viewport: ViewportRect,
        font_size: f32,
        scene_viewport: Rect,
        color: Rgba8,
    ) -> Self {
        Self {
            viewport,
            font_size,
            scene_viewport,
            color,
        }
    }

    #[must_use]
    pub const fn viewport(self) -> ViewportRect {
        self.viewport
    }

    #[must_use]
    pub const fn font_size(self) -> f32 {
        self.font_size
    }

    #[must_use]
    pub const fn scene_viewport(self) -> Rect {
        self.scene_viewport
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }
}

/// Errors raised when a scene and render plan are combined or published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportFrameError {
    RevisionMismatch {
        scene: Revision,
        plan: Revision,
    },
    Stale {
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for ViewportFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionMismatch { scene, plan } => {
                write!(
                    formatter,
                    "viewport scene revision {scene:?} does not match plan {plan:?}"
                )
            }
            Self::Stale { expected, actual } => {
                write!(
                    formatter,
                    "viewport frame {actual:?} is stale for {expected:?}"
                )
            }
        }
    }
}

impl Error for ViewportFrameError {}

/// Single-entry cache for the latest publishable viewport frame.
///
/// A lookup is revision-aware and never returns a frame for another source
/// revision. Hosts can call `invalidate_stale` after an edit to eagerly drop
/// the old scene and plan, while a stale publish is rejected even if the host
/// forgot to clear first.
#[derive(Clone, Debug, Default)]
pub struct ViewportFrameCache {
    current: Option<Arc<ViewportRenderFrame>>,
}

impl ViewportFrameCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn current_revision(&self) -> Option<Revision> {
        self.current.as_ref().map(|frame| frame.revision())
    }

    #[must_use]
    pub fn get(&self, revision: Revision) -> Option<&ViewportRenderFrame> {
        self.current
            .as_ref()
            .filter(|frame| frame.revision() == revision)
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn current_frame_handle(&self, revision: Revision) -> Option<Arc<ViewportRenderFrame>> {
        self.current
            .as_ref()
            .filter(|frame| frame.revision() == revision)
            .map(Arc::clone)
    }

    /// Drops the cached frame unless it belongs to `revision`.
    ///
    /// Returns `true` when an old frame was removed. An empty cache is already
    /// synchronized and returns `false`.
    pub fn invalidate_stale(&mut self, revision: Revision) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|frame| frame.revision() != revision)
        {
            self.current = None;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.current = None;
    }

    /// Publishes a complete frame only if it still belongs to the caller's
    /// current document revision. Replacement is atomic at the cache level.
    pub fn publish_if_current(
        &mut self,
        current_revision: Revision,
        frame: ViewportRenderFrame,
    ) -> Result<(), ViewportFrameError> {
        self.publish_shared_if_current(current_revision, Arc::new(frame))
    }

    /// Publishes a shared immutable frame handle without cloning the scene or
    /// render plan. The caller may retain another handle for a platform
    /// handoff; all handles still refer to the same revision-bound frame.
    pub fn publish_shared_if_current(
        &mut self,
        current_revision: Revision,
        frame: Arc<ViewportRenderFrame>,
    ) -> Result<(), ViewportFrameError> {
        if frame.revision() != current_revision {
            return Err(ViewportFrameError::Stale {
                expected: current_revision,
                actual: frame.revision(),
            });
        }
        if let Some(existing) = self.current.as_ref()
            && existing.revision() > frame.revision()
        {
            return Err(ViewportFrameError::Stale {
                expected: existing.revision(),
                actual: frame.revision(),
            });
        }
        self.current = Some(frame);
        Ok(())
    }
}

/// Errors raised while assembling an editor viewport into a retained scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportSceneError {
    Document(EditorDocumentError),
    Scene(SceneError),
    Render(RenderError),
    Frame(ViewportFrameError),
}

impl fmt::Display for ViewportSceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => error.fmt(formatter),
            Self::Scene(error) => error.fmt(formatter),
            Self::Render(error) => error.fmt(formatter),
            Self::Frame(error) => error.fmt(formatter),
        }
    }
}

impl Error for ViewportSceneError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Document(error) => Some(error),
            Self::Scene(error) => Some(error),
            Self::Render(error) => Some(error),
            Self::Frame(error) => Some(error),
        }
    }
}

impl From<EditorDocumentError> for ViewportSceneError {
    fn from(error: EditorDocumentError) -> Self {
        Self::Document(error)
    }
}

impl From<SceneError> for ViewportSceneError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

impl From<RenderError> for ViewportSceneError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

impl From<ViewportFrameError> for ViewportSceneError {
    fn from(error: ViewportFrameError) -> Self {
        Self::Frame(error)
    }
}

/// An owned viewport frame handed from the shared workspace publisher to a
/// platform host. The frame keeps the scene, render plan, source Revision and
/// publication serial together so a host cannot accidentally pair a frame
/// with metadata from another build.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportFramePublication {
    revision: Revision,
    serial: u64,
    frame: Arc<ViewportRenderFrame>,
}

impl ViewportFramePublication {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn serial(&self) -> u64 {
        self.serial
    }

    #[must_use]
    pub fn frame(&self) -> &ViewportRenderFrame {
        self.frame.as_ref()
    }

    #[must_use]
    pub fn frame_handle(&self) -> Arc<ViewportRenderFrame> {
        Arc::clone(&self.frame)
    }

    #[must_use]
    pub fn into_parts(self) -> (Revision, u64, Arc<ViewportRenderFrame>) {
        (self.revision, self.serial, self.frame)
    }
}

/// Errors raised while assembling and publishing a workspace viewport frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportPublishError {
    Scene(ViewportSceneError),
    SerialOverflow,
}

impl fmt::Display for ViewportPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(error) => error.fmt(formatter),
            Self::SerialOverflow => formatter.write_str("viewport publication serial overflowed"),
        }
    }
}

impl Error for ViewportPublishError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scene(error) => Some(error),
            Self::SerialOverflow => None,
        }
    }
}

impl From<ViewportSceneError> for ViewportPublishError {
    fn from(error: ViewportSceneError) -> Self {
        Self::Scene(error)
    }
}

/// Shared, platform-free owner of the latest assembled viewport frame.
///
/// The publisher reads the current `EditorDocument` Revision, assembles the
/// revision-bound scene and render plan, and returns an owned publication with
/// a monotonic serial. It does not own source text, an editor document, a
/// native surface or a GPU object. A host may retain the returned publication
/// while the publisher keeps a revision-aware cache for the latest frame.
#[derive(Clone, Debug, Default)]
pub struct ViewportFramePublisher {
    cache: ViewportFrameCache,
    next_serial: u64,
    last_publication: Option<ViewportFramePublication>,
}

impl ViewportFramePublisher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn current_revision(&self) -> Option<Revision> {
        self.cache.current_revision()
    }

    #[must_use]
    pub fn current_frame(&self, revision: Revision) -> Option<&ViewportRenderFrame> {
        self.cache.get(revision)
    }

    #[must_use]
    pub fn current_frame_handle(&self, revision: Revision) -> Option<Arc<ViewportRenderFrame>> {
        self.cache.current_frame_handle(revision)
    }

    #[must_use]
    pub fn last_publication(&self) -> Option<&ViewportFramePublication> {
        self.last_publication.as_ref()
    }

    /// Drops the cached frame and publication unless they belong to `revision`.
    pub fn invalidate_stale(&mut self, revision: Revision) -> bool {
        let cache_dropped = self.cache.invalidate_stale(revision);
        let publication_dropped = self
            .last_publication
            .as_ref()
            .is_some_and(|publication| publication.revision() != revision);
        if publication_dropped {
            self.last_publication = None;
        }
        cache_dropped || publication_dropped
    }

    /// Assembles and publishes the editor's current viewport as one owned
    /// publication. All mutable state is updated only after assembly and the
    /// next serial have both been validated.
    pub fn publish<S: ShapingProvider>(
        &mut self,
        document: &mut EditorDocument,
        config: ViewportRenderConfig,
        shaper: &S,
        atlas: &GlyphAtlas,
        render_plans: &mut RenderPlanBuilder,
    ) -> Result<ViewportFramePublication, ViewportPublishError> {
        let frame = assemble_viewport_render_frame(document, config, shaper, atlas, render_plans)?;
        let revision = document.revision();
        if frame.revision() != revision {
            return Err(ViewportPublishError::Scene(ViewportSceneError::Frame(
                ViewportFrameError::Stale {
                    expected: revision,
                    actual: frame.revision(),
                },
            )));
        }
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(ViewportPublishError::SerialOverflow)?;
        let frame = Arc::new(frame);
        self.cache
            .publish_shared_if_current(revision, Arc::clone(&frame))
            .map_err(|error| ViewportPublishError::Scene(ViewportSceneError::Frame(error)))?;

        let publication = ViewportFramePublication {
            revision,
            serial,
            frame,
        };
        self.next_serial = serial;
        self.last_publication = Some(publication.clone());
        Ok(publication)
    }
}

/// Measures the current editor viewport, converts its block metadata into the
/// scene boundary, materializes matching shaped block layouts, and appends
/// them atomically through `SceneBuilder::append_viewport`.
pub fn assemble_viewport_scene<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    let viewport_snapshot = document.visible_blocks_with_shaper(viewport, shaper)?;
    let revision = viewport_snapshot.revision();
    let config = document.viewport_config().layout();
    let geometries = viewport_snapshot
        .blocks()
        .iter()
        .copied()
        .map(|block| {
            ViewportBlockGeometry::new(
                revision,
                block.index(),
                block.source(),
                block.y(),
                block.height(),
                block.is_measured(),
                block.kind().viewport_tag(),
            )
        })
        .collect::<Result<Vec<_>, SceneError>>()?;
    let input = ViewportSceneInput::new(
        revision,
        viewport_snapshot.range().start()..viewport_snapshot.range().end(),
        viewport_snapshot.content_height(),
        geometries,
    )?;

    let mut layouts = Vec::with_capacity(viewport_snapshot.blocks().len());
    for block in viewport_snapshot.blocks() {
        layouts.push(
            document
                .block_layout_with_shaper(block.index(), config, shaper)?
                .clone(),
        );
    }
    let layout_refs = layouts.iter().collect::<Vec<_>>();
    let mut builder = SceneBuilder::new(revision, scene_viewport)?;
    builder.append_viewport(&input, &layout_refs, atlas, font_size, color)?;
    Ok(ViewportSceneFrame {
        input,
        scene: builder.finish(),
    })
}

/// Builds a scene and its backend-neutral render plan from one editor viewport
/// measurement. The render-plan builder is updated only after scene assembly
/// succeeds; callers can then publish the returned pair through
/// `ViewportFrameCache::publish_if_current`.
pub fn assemble_viewport_render_frame<S: ShapingProvider>(
    document: &mut EditorDocument,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    let scene = assemble_viewport_scene(
        document,
        config.viewport(),
        shaper,
        config.font_size(),
        config.scene_viewport(),
        atlas,
        config.color(),
    )?;
    if scene.revision() != document.revision() {
        return Err(ViewportFrameError::Stale {
            expected: document.revision(),
            actual: scene.revision(),
        }
        .into());
    }
    let plan = render_plans.build(scene.scene(), atlas)?;
    ViewportRenderFrame::new(scene, plan).map_err(ViewportSceneError::from)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yu_editor::{EditorCommand, LayoutConfig, ViewportConfig};
    use yu_font::{
        FontDatabase, FontFaceSpec, FontRequest, FontShaper, GlyphAtlasConfig, GlyphBitmap,
        GlyphMetrics, GlyphRasterKey, RasterizedGlyph,
    };
    use yu_render::RenderPlanBuilder;
    use yu_scene::{Primitive, Rgba8};

    use super::*;

    fn shaper(font_size: f32) -> FontShaper {
        let mut database = FontDatabase::new();
        database
            .register(FontFaceSpec::new("Test", 0.5))
            .expect("font face");
        FontShaper::new(
            Arc::new(database),
            FontRequest::new("Test", font_size).expect("font request"),
        )
        .expect("shaper")
    }

    fn atlas_for_document(
        document: &mut EditorDocument,
        viewport: ViewportRect,
        shaper: &FontShaper,
        font_size: f32,
    ) -> GlyphAtlas {
        let snapshot = document
            .visible_blocks_with_shaper(viewport, shaper)
            .expect("viewport");
        let config = document.viewport_config().layout();
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("atlas config"));
        for block in snapshot.blocks() {
            let layout = document
                .block_layout_with_shaper(block.index(), config, shaper)
                .expect("layout");
            for placement in layout.glyphs() {
                let key = GlyphRasterKey::new(placement.face(), placement.glyph(), font_size)
                    .expect("glyph key");
                if atlas.entry(key).is_none() {
                    let glyph = RasterizedGlyph::new(
                        key,
                        GlyphMetrics::new(0.0, 10.0, 7.0).expect("metrics"),
                        GlyphBitmap::new(2, 3, 2, vec![255; 6]).expect("bitmap"),
                    );
                    atlas.insert(glyph).expect("atlas insert");
                }
            }
        }
        atlas
    }

    #[test]
    fn editor_viewport_is_assembled_into_revision_bound_render_plan() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("# title\n\nhello **world**");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("scene frame");

        assert_eq!(frame.revision(), document.revision());
        assert_eq!(frame.input().revision(), document.revision());
        assert_eq!(frame.input().blocks().len(), 3);
        assert!(!frame.scene().primitives().is_empty());
        let mut plans = RenderPlanBuilder::new();
        let plan = plans.build(frame.scene(), &atlas).expect("render plan");
        assert_eq!(plan.revision(), document.revision());
        assert_eq!(plan.commands().len(), frame.scene().primitives().len());
        assert!(
            plan.commands()
                .iter()
                .any(|command| matches!(command, yu_render::RenderCommand::Glyph { .. }))
        );
        assert!(
            frame
                .scene()
                .primitives()
                .iter()
                .all(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
    }

    #[test]
    fn missing_atlas_is_rejected_before_frame_publication() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 80.0);
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let empty_atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("atlas config"));
        let error = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
            &empty_atlas,
            Rgba8::black(),
        )
        .expect_err("missing glyph must fail");
        assert!(matches!(
            error,
            ViewportSceneError::Scene(SceneError::MissingGlyphAtlas(_))
        ));
    }

    #[test]
    fn frame_cache_rejects_stale_publish_and_replaces_after_edit() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 80.0);
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut plans = RenderPlanBuilder::new();
        let old_frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &atlas,
            &mut plans,
        )
        .expect("old frame");
        let old_revision = document.revision();
        let mut cache = ViewportFrameCache::new();
        cache
            .publish_if_current(old_revision, old_frame.clone())
            .expect("initial publish");
        assert_eq!(cache.current_revision(), Some(old_revision));
        assert!(cache.get(old_revision).is_some());

        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit");
        let new_revision = document.revision();
        assert_ne!(new_revision, old_revision);
        assert_eq!(
            cache.publish_if_current(new_revision, old_frame.clone()),
            Err(ViewportFrameError::Stale {
                expected: new_revision,
                actual: old_revision,
            })
        );
        assert_eq!(cache.current_revision(), Some(old_revision));
        assert!(cache.invalidate_stale(new_revision));
        assert!(cache.get(old_revision).is_none());
        assert!(!cache.invalidate_stale(new_revision));

        let new_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let new_frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &new_atlas,
            &mut plans,
        )
        .expect("new frame");
        cache
            .publish_if_current(new_revision, new_frame)
            .expect("new publish");
        assert_eq!(cache.current_revision(), Some(new_revision));
        assert_eq!(
            cache.get(new_revision).expect("new frame").revision(),
            new_revision
        );
        assert_eq!(
            cache.publish_if_current(old_revision, old_frame),
            Err(ViewportFrameError::Stale {
                expected: new_revision,
                actual: old_revision,
            })
        );
    }

    #[test]
    fn frame_publisher_returns_owned_revision_bound_publications() {
        use std::sync::Arc;

        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 80.0);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut plans = RenderPlanBuilder::new();
        let mut publisher = ViewportFramePublisher::new();

        let first = publisher
            .publish(&mut document, config, &shaper, &atlas, &mut plans)
            .expect("first publication");
        let first_revision = document.revision();
        assert_eq!(first.revision(), first_revision);
        assert_eq!(first.frame().revision(), first_revision);
        assert_eq!(first.serial(), 1);
        assert_eq!(publisher.current_revision(), Some(first_revision));
        assert_eq!(publisher.last_publication(), Some(&first));
        assert_eq!(publisher.current_frame(first_revision), Some(first.frame()));
        let cached_handle = publisher
            .current_frame_handle(first_revision)
            .expect("cached frame handle");
        assert!(Arc::ptr_eq(&cached_handle, &first.frame_handle()));

        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit");
        let second_revision = document.revision();
        assert!(publisher.invalidate_stale(second_revision));
        assert_eq!(publisher.current_revision(), None);
        assert_eq!(publisher.last_publication(), None);
        assert_eq!(first.frame().revision(), first_revision);

        let new_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let second = publisher
            .publish(&mut document, config, &shaper, &new_atlas, &mut plans)
            .expect("second publication");
        assert_eq!(second.revision(), second_revision);
        assert_eq!(second.serial(), 2);
        assert_eq!(publisher.last_publication(), Some(&second));
    }
}
