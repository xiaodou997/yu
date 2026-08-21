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

use yu_assets::{
    EmbeddedRenderPayload, EmbeddedRenderPublication, ImageIntrinsicPublication, ImageKey,
    ImagePublication,
};
use yu_core::{Revision, TextRange};
use yu_editor::{
    BlockKind, CaretAffinity, EditorDocument, EditorDocumentError, ImageSource, LayoutError,
    ProjectionBias, ShapingProvider, TableResizeCommit, TableResizeTarget, TaskState, ViewportRect,
    task_marker,
};
use yu_font::GlyphAtlas;
use yu_layout::ImageIntrinsicSize;
use yu_render::{RenderError, RenderPlan, RenderPlanBuilder};
use yu_scene::{
    EditorDecorationPrimitive, EditorDecorationPrimitiveRole, EmbeddedSvgPrimitive, ImagePrimitive,
    Point, Primitive, Rect, Rgba8, Scene, SceneBuilder, SceneError, TableSceneStyle,
    TaskCheckboxPrimitive, TaskCheckboxPrimitiveRole, ViewportBlockGeometry, ViewportSceneInput,
};

mod workspace;

pub use workspace::{
    CloseAction, CloseResult, OpenTabResult, TabId, Workspace, WorkspaceCloseRequest,
    WorkspaceError, WorkspaceTab,
};

/// A validated scene together with the viewport metadata that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportSceneFrame {
    input: ViewportSceneInput,
    scene: Scene,
}

/// A parser- and Revision-bound task checkbox target from one published scene.
///
/// `source` is the exact `[ ]`/`[x]` marker range rather than the containing
/// list item. Platform shells use `block_index` only to invoke the existing
/// canonical `ToggleTask` editor command after validating this hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskCheckboxHit {
    revision: Revision,
    block_index: usize,
    source: yu_core::TextRange,
    bounds: Rect,
}

impl TaskCheckboxHit {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn block_index(self) -> usize {
        self.block_index
    }

    #[must_use]
    pub const fn source(self) -> yu_core::TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }
}

/// Returns the optional background used by the product visual projection for
/// one parser block. The scene crate receives only the resulting color, so
/// Markdown semantics stay at this editor-to-scene integration boundary.
#[must_use]
pub fn viewport_block_background(kind: BlockKind) -> Option<Rgba8> {
    match kind {
        BlockKind::FencedCodeBlock { .. } => Some(Rgba8::new(245, 246, 248, 255)),
        _ => None,
    }
}

#[must_use]
fn viewport_table_style() -> TableSceneStyle {
    TableSceneStyle::new(
        1.0,
        Rgba8::new(190, 195, 205, 255),
        Some(Rgba8::new(248, 249, 251, 255)),
        Some(Rgba8::new(210, 225, 255, 255)),
    )
}

#[must_use]
const fn viewport_block_quote_color() -> Rgba8 {
    Rgba8::new(176, 181, 190, 255)
}

fn append_task_checkbox(
    builder: &mut SceneBuilder,
    layout: &yu_layout::LayoutSnapshot,
    block_y: f32,
    marker: yu_editor::TaskMarker,
    state: TaskState,
) -> Result<(), ViewportSceneError> {
    let caret = layout
        .caret_for_source(marker.range().start(), ProjectionBias::After)
        .map_err(EditorDocumentError::from)?;
    let line_height = layout.config().line_height();
    let size = line_height * 0.68;
    let x = caret.point().x();
    let y = block_y + caret.point().y() + (line_height - size) * 0.5;
    let bounds = Rect::new(x, y, size, size)?;
    let border = match state {
        TaskState::Todo => Rgba8::new(118, 124, 134, 255),
        TaskState::Done => Rgba8::new(38, 111, 219, 255),
    };
    builder.task_checkbox(TaskCheckboxPrimitive::new(
        marker.range(),
        bounds,
        border,
        TaskCheckboxPrimitiveRole::Border,
    ))?;

    match state {
        TaskState::Todo => {
            let inset = (size * 0.14).max(0.5).min(size * 0.3);
            builder.task_checkbox(TaskCheckboxPrimitive::new(
                marker.range(),
                Rect::new(x + inset, y + inset, size - inset * 2.0, size - inset * 2.0)?,
                Rgba8::white(),
                TaskCheckboxPrimitiveRole::Interior,
            ))?;
        }
        TaskState::Done => {
            let unit = size / 5.0;
            for (column, row) in [(1.0, 2.4), (1.8, 3.1), (2.7, 2.4), (3.6, 1.5)] {
                builder.task_checkbox(TaskCheckboxPrimitive::new(
                    marker.range(),
                    Rect::new(x + unit * column, y + unit * row, unit * 0.85, unit * 0.85)?,
                    Rgba8::white(),
                    TaskCheckboxPrimitiveRole::Check,
                ))?;
            }
        }
    }
    Ok(())
}

/// Platform-selected colors and geometry for selection/caret scene layers.
///
/// This stays outside `yu-scene`: the retained scene records semantic roles,
/// while the workspace/platform boundary chooses product colors and whether
/// editor chrome belongs in a publication at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorDecorationStyle {
    selection: Rgba8,
    caret: Rgba8,
    composition_caret: Rgba8,
    caret_width: f32,
}

impl EditorDecorationStyle {
    #[must_use]
    pub const fn new(
        selection: Rgba8,
        caret: Rgba8,
        composition_caret: Rgba8,
        caret_width: f32,
    ) -> Self {
        Self {
            selection,
            caret,
            composition_caret,
            caret_width,
        }
    }
}

fn append_editor_decorations(
    builder: &mut SceneBuilder,
    document: &EditorDocument,
    input: &ViewportSceneInput,
    layouts: &[yu_layout::LayoutSnapshot],
    style: EditorDecorationStyle,
) -> Result<(), ViewportSceneError> {
    if !style.caret_width.is_finite() || style.caret_width <= 0.0 {
        return Err(
            SceneError::InvalidGeometry("editor caret width must be finite and positive").into(),
        );
    }
    let selection = document.selection();
    let selection_range = selection.ordered_range();
    let composition = document.composition();
    let composition_blocks = document.composition_block_range();
    let focus_block = if composition.is_some() {
        composition_blocks.as_ref().map(|span| span.start)
    } else {
        document.block_index_for_source(selection.focus())
    };

    // An empty canonical document intentionally has no Markdown block or text
    // layout. Publish its insertion point directly so the retained frame can
    // still own the visible editor instead of requiring a TextKit glyph pass.
    if input.blocks().is_empty() && document.snapshot().is_empty() && composition.is_none() {
        builder.editor_decoration(EditorDecorationPrimitive::new(
            TextRange::empty(selection.focus()),
            Rect::new(
                0.0,
                0.0,
                style.caret_width,
                document.viewport_config().layout().line_height(),
            )?,
            style.caret,
            EditorDecorationPrimitiveRole::Caret,
        ))?;
        return Ok(());
    }

    let mut caret = None;
    for (geometry, layout) in input.blocks().iter().copied().zip(layouts.iter()) {
        let (visual_start, visual_end, layer_source) = if let Some(overlay) = composition {
            let Some(visual) = layout.projection().composition_selection_visual() else {
                if focus_block == Some(geometry.index()) {
                    let visual = layout
                        .projection()
                        .source_to_visual(
                            overlay.replacement_range().start(),
                            ProjectionBias::Before,
                        )
                        .map_err(EditorDocumentError::from)?;
                    let layout_caret = layout
                        .caret_for_visual(visual, ProjectionBias::After)
                        .map_err(EditorDocumentError::from)?;
                    caret = Some((
                        TextRange::empty(overlay.replacement_range().start()),
                        layout_caret,
                        EditorDecorationPrimitiveRole::CompositionCaret,
                        geometry.y(),
                        layout.config().line_height(),
                    ));
                }
                continue;
            };
            if focus_block == Some(geometry.index()) {
                let layout_caret = layout
                    .caret_for_visual(visual.end(), ProjectionBias::After)
                    .map_err(EditorDocumentError::from)?;
                caret = Some((
                    TextRange::empty(overlay.replacement_range().start()),
                    layout_caret,
                    EditorDecorationPrimitiveRole::CompositionCaret,
                    geometry.y(),
                    layout.config().line_height(),
                ));
            }
            (visual.start(), visual.end(), overlay.replacement_range())
        } else {
            if focus_block == Some(geometry.index()) {
                let bias = match selection.affinity() {
                    CaretAffinity::Upstream => ProjectionBias::Before,
                    CaretAffinity::Downstream => ProjectionBias::After,
                };
                let layout_caret = layout
                    .caret_for_source(selection.focus(), bias)
                    .map_err(EditorDocumentError::from)?;
                caret = Some((
                    TextRange::empty(selection.focus()),
                    layout_caret,
                    EditorDecorationPrimitiveRole::Caret,
                    geometry.y(),
                    layout.config().line_height(),
                ));
            }
            if selection.is_empty() {
                continue;
            }
            let start = selection_range.start().max(geometry.source().start());
            let end = selection_range.end().min(geometry.source().end());
            if start >= end {
                continue;
            }
            let source = TextRange::new(start, end).expect("ordered selection intersection");
            (
                layout
                    .projection()
                    .source_to_visual(start, ProjectionBias::Before)
                    .map_err(EditorDocumentError::from)?,
                layout
                    .projection()
                    .source_to_visual(end, ProjectionBias::After)
                    .map_err(EditorDocumentError::from)?,
                source,
            )
        };

        if visual_start >= visual_end {
            continue;
        }
        for line in layout.lines() {
            let line_start = line.visual().start().max(visual_start);
            let line_end = line.visual().end().min(visual_end);
            if line_start >= line_end {
                continue;
            }
            let mut left = f32::INFINITY;
            let mut right = f32::NEG_INFINITY;
            for cluster_index in line.cluster_range() {
                let cluster = layout.clusters()[cluster_index];
                if cluster.is_line_break()
                    || cluster.visual().end() <= line_start
                    || cluster.visual().start() >= line_end
                {
                    continue;
                }
                left = left.min(cluster.x());
                right = right.max(cluster.x() + cluster.width());
            }
            if left.is_finite() && right.is_finite() && right > left {
                builder.editor_decoration(EditorDecorationPrimitive::new(
                    layer_source,
                    Rect::new(
                        left,
                        geometry.y() + line.y(),
                        right - left,
                        layout.config().line_height(),
                    )?,
                    style.selection,
                    EditorDecorationPrimitiveRole::Selection,
                ))?;
            }
        }
    }

    if let Some((source, caret, role, block_y, line_height)) = caret {
        let color = if role == EditorDecorationPrimitiveRole::CompositionCaret {
            style.composition_caret
        } else {
            style.caret
        };
        builder.editor_decoration(EditorDecorationPrimitive::new(
            source,
            Rect::new(
                caret.point().x(),
                block_y + caret.point().y(),
                style.caret_width,
                line_height,
            )?,
            color,
            role,
        ))?;
    }
    Ok(())
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

    /// Resolves a document-space point against the task checkbox borders in
    /// this exact published frame. Interior/check painter layers are ignored,
    /// so one visible checkbox can produce at most one semantic hit.
    pub fn task_checkbox_hit_test(
        &self,
        expected_revision: Revision,
        point: Point,
    ) -> Result<Option<TaskCheckboxHit>, ViewportFrameError> {
        if self.revision() != expected_revision {
            return Err(ViewportFrameError::Stale {
                expected: expected_revision,
                actual: self.revision(),
            });
        }
        let Some(task) = self.scene.primitives().iter().find_map(|primitive| {
            let Primitive::TaskCheckbox(task) = primitive else {
                return None;
            };
            (task.role() == TaskCheckboxPrimitiveRole::Border && task.bounds().contains(point))
                .then_some(*task)
        }) else {
            return Ok(None);
        };
        let block = self.input.blocks().iter().copied().find(|block| {
            block.source().start() <= task.source().start()
                && task.source().end() <= block.source().end()
        });
        Ok(block.map(|block| TaskCheckboxHit {
            revision: self.revision(),
            block_index: block.index(),
            source: task.source(),
            bounds: task.bounds(),
        }))
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
    /// 编辑区背景色。
    ///
    /// 删除 TextKit fallback 后，Rust surface 是唯一渲染路径（不变量 I5），
    /// 背景必须由这一帧自己画出来：Metal layer 是透明的，未触及的像素会露出
    /// 下层视图，而下层视图已经不再绘制任何东西。
    background: Rgba8,
    /// glyph 栅格化相对逻辑尺寸的倍率，默认 `1.0`。
    ///
    /// Retina 上应设为 backing scale：字形按 `font_size × raster_scale`
    /// 取样，后端再把 atlas 矩形除回逻辑坐标，纹理才能与物理像素 1:1 对应。
    raster_scale: f32,
    scene_viewport: Rect,
    color: Rgba8,
    table_resize: Option<TableResizeCommit>,
    editor_decorations: Option<EditorDecorationStyle>,
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
            background: Rgba8::white(),
            raster_scale: 1.0,
            scene_viewport,
            color,
            table_resize: None,
            editor_decorations: None,
        }
    }

    /// Returns a copy that carries one caller-owned, session-only table
    /// geometry override into the next scene/render-plan build.
    #[must_use]
    pub const fn with_table_resize(mut self, resize: TableResizeCommit) -> Self {
        self.table_resize = Some(resize);
        self
    }

    #[must_use]
    pub const fn with_editor_decorations(mut self, style: EditorDecorationStyle) -> Self {
        self.editor_decorations = Some(style);
        self
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
    pub const fn with_background(mut self, background: Rgba8) -> Self {
        self.background = background;
        self
    }

    #[must_use]
    pub const fn background(self) -> Rgba8 {
        self.background
    }

    /// 设置 glyph 栅格化倍率。非有限或非正值会被忽略，保留原值。
    #[must_use]
    pub fn with_raster_scale(mut self, scale: f32) -> Self {
        if scale.is_finite() && scale > 0.0 {
            self.raster_scale = scale;
        }
        self
    }

    #[must_use]
    pub const fn raster_scale(self) -> f32 {
        self.raster_scale
    }

    #[must_use]
    pub const fn scene_viewport(self) -> Rect {
        self.scene_viewport
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    #[must_use]
    pub const fn table_resize(self) -> Option<TableResizeCommit> {
        self.table_resize
    }

    #[must_use]
    pub const fn editor_decorations(self) -> Option<EditorDecorationStyle> {
        self.editor_decorations
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
    InvalidTaskMarker { block: usize },
}

impl fmt::Display for ViewportSceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => error.fmt(formatter),
            Self::Scene(error) => error.fmt(formatter),
            Self::Render(error) => error.fmt(formatter),
            Self::Frame(error) => error.fmt(formatter),
            Self::InvalidTaskMarker { block } => {
                write!(
                    formatter,
                    "task-list block {block} has no parser-owned marker"
                )
            }
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
            Self::InvalidTaskMarker { .. } => None,
        }
    }
}

impl From<EditorDocumentError> for ViewportSceneError {
    fn from(error: EditorDocumentError) -> Self {
        Self::Document(error)
    }
}

impl From<yu_core::GeometryError> for ViewportSceneError {
    fn from(error: yu_core::GeometryError) -> Self {
        Self::Scene(SceneError::from(error))
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

    /// Creates a publisher whose next publication serial follows an existing
    /// publisher. This is used when a platform rebuilds only its shaping or
    /// layout state while retaining the same document/session lifecycle.
    #[must_use]
    pub fn with_next_serial(next_serial: u64) -> Self {
        Self {
            cache: ViewportFrameCache::new(),
            next_serial,
            last_publication: None,
        }
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
        self.publish_with_images(document, config, shaper, atlas, render_plans, &[])
    }

    /// Publishes the viewport while applying dimensions from already-ready
    /// image publications. Missing images keep the normal placeholder layout.
    pub fn publish_with_images<S: ShapingProvider>(
        &mut self,
        document: &mut EditorDocument,
        config: ViewportRenderConfig,
        shaper: &S,
        atlas: &GlyphAtlas,
        render_plans: &mut RenderPlanBuilder,
        image_publications: &[ImagePublication],
    ) -> Result<ViewportFramePublication, ViewportPublishError> {
        self.publish_with_images_and_intrinsics(
            document,
            config,
            shaper,
            atlas,
            render_plans,
            image_publications,
            &[],
        )
    }

    /// Publishes the viewport while applying ready pixels and/or intrinsic
    /// dimensions retained after those pixels were evicted.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_with_images_and_intrinsics<S: ShapingProvider>(
        &mut self,
        document: &mut EditorDocument,
        config: ViewportRenderConfig,
        shaper: &S,
        atlas: &GlyphAtlas,
        render_plans: &mut RenderPlanBuilder,
        image_publications: &[ImagePublication],
        image_intrinsics: &[ImageIntrinsicPublication],
    ) -> Result<ViewportFramePublication, ViewportPublishError> {
        // RenderPlanBuilder carries page-fingerprint state across frames. Build against a
        // staged copy so a later publication failure cannot advance caller-owned state.
        let mut staged_render_plans = render_plans.clone();
        let frame = assemble_viewport_render_frame_with_images_and_intrinsics(
            document,
            config.viewport(),
            config,
            shaper,
            atlas,
            &mut staged_render_plans,
            image_publications,
            image_intrinsics,
        )?;
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

        *render_plans = staged_render_plans;
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
    assemble_viewport_scene_with_images(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        &[],
    )
}

/// Builds a viewport scene with one caller-owned session-only table resize.
/// The override is applied only to the transient block layout used by this
/// scene; source, selection, history and the document layout cache remain
/// unchanged.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_table_resize<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    table_resize: TableResizeCommit,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics_and_table_resize(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        &[],
        &[],
        Some(table_resize),
    )
}

/// Builds a viewport scene with dimensions from ready image publications.
/// Publications are optional and are matched by the same source-backed
/// destination fingerprint used by the image primitive.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    image_publications: &[ImagePublication],
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        image_publications,
        &[],
    )
}

/// Builds a viewport scene with ready pixels and/or retained intrinsic image
/// dimensions. Intrinsic publications are deliberately separate from decoded
/// pixels so CPU/GPU cache eviction cannot make document geometry jump.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images_and_intrinsics<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics_and_table_resize(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        image_publications,
        image_intrinsics,
        None,
    )
}

/// Builds a viewport scene with ready image dimensions and an optional
/// caller-owned session-only table column override. The override is rejected
/// when stale or when it targets the deferred variable-row path.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images_and_intrinsics_and_table_resize<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
    table_resize: Option<TableResizeCommit>,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics_and_embedded_and_table_resize(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        // 这些兼容入口不带背景配置，按白底渲染。
        Rgba8::white(),
        image_publications,
        image_intrinsics,
        &[],
        table_resize,
        None,
    )
}

/// Builds a viewport scene with ready image dimensions and revision-bound SVG
/// publications. Embedded primitives are appended only for matching visible
/// fenced blocks; source glyphs remain in painter order and the primitive uses
/// a transparent fallback until a native SVG consumer is available.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images_and_intrinsics_and_embedded_and_table_resize<
    S: ShapingProvider,
>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    background: Rgba8,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
    embedded_publications: &[EmbeddedRenderPublication],
    table_resize: Option<TableResizeCommit>,
    editor_decorations: Option<EditorDecorationStyle>,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    let source = document.snapshot();
    let definitions = document.markdown().reference_definitions().clone();
    let document_revision = document.revision();
    if let Some(resize) = table_resize {
        if resize.revision() != document_revision {
            return Err(EditorDocumentError::Layout(LayoutError::InvalidTable(
                "table resize and viewport document revisions differ",
            ))
            .into());
        }
        if matches!(resize.target(), TableResizeTarget::Row { .. }) {
            return Err(EditorDocumentError::Layout(LayoutError::InvalidTable(
                "row resize requires variable-row table layout",
            ))
            .into());
        }
    }
    let selection = Some(document.selection().ordered_range());
    let image_key = |image: ImageSource| {
        let destination = image.destination().or_else(|| {
            image
                .reference()
                .and_then(|reference| definitions.lookup(&source, reference))
                .map(|definition| definition.destination())
        })?;
        let start = usize::try_from(destination.start().get()).ok()?;
        let end = usize::try_from(destination.end().get()).ok()?;
        let destination = source.as_str().get(start..end)?;
        ImageKey::new(destination.to_owned()).ok()
    };
    let intrinsic_size = |image: ImageSource| {
        let key = image_key(image)?;
        if let Some(publication) = image_publications.iter().find(|publication| {
            publication.revision() == document_revision
                && publication.key().fingerprint() == key.fingerprint()
        }) {
            return ImageIntrinsicSize::new(
                publication.image().width(),
                publication.image().height(),
            )
            .ok();
        }
        let intrinsic = image_intrinsics.iter().find(|intrinsic| {
            intrinsic.revision() == document_revision
                && intrinsic.key().fingerprint() == key.fingerprint()
        })?;
        let dimensions = intrinsic.dimensions();
        ImageIntrinsicSize::new(dimensions.width(), dimensions.height()).ok()
    };
    let viewport_snapshot = document
        .visible_blocks_with_visual_state_and_shaper_and_image_resolver(
            viewport,
            shaper,
            intrinsic_size,
        )?;
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
    let content_height = if viewport_snapshot.blocks().is_empty()
        && source.is_empty()
        && document.composition().is_none()
    {
        config.line_height()
    } else {
        viewport_snapshot.content_height()
    };
    let input = ViewportSceneInput::new(
        revision,
        viewport_snapshot.range().start()..viewport_snapshot.range().end(),
        content_height,
        geometries,
    )?;

    let mut layouts = Vec::with_capacity(viewport_snapshot.blocks().len());
    for block in viewport_snapshot.blocks() {
        let mut layout =
            document.block_layout_for_visual_state_with_shaper(block.index(), config, shaper)?;
        let measurements = layout
            .projection()
            .images()
            .iter()
            .copied()
            .filter_map(|image| {
                image_key(image)?;
                let size = intrinsic_size(image)?;
                Some((image.source(), size))
            })
            .collect::<Vec<_>>();
        layout
            .apply_image_intrinsic_sizes(&measurements)
            .map_err(EditorDocumentError::from)?;
        if let Some(resize) = table_resize.filter(|resize| resize.block_index() == block.index()) {
            layout
                .apply_table_resize(resize)
                .map_err(EditorDocumentError::from)?;
        }
        layouts.push(layout);
    }
    let layout_refs = layouts.iter().collect::<Vec<_>>();
    let mut builder = SceneBuilder::new(revision, scene_viewport)?;
    // 背景必须是这一帧的第一个 primitive：Metal layer 透明，未触及的像素会
    // 露出下层视图，而 TextKit fallback 删除后下层已不再绘制任何东西
    // （不变量 I5）。放在最前面也保证它位于所有内容之下。
    builder.fill_rect(scene_viewport, background)?;
    let fills = viewport_snapshot
        .blocks()
        .iter()
        .map(|block| viewport_block_background(block.kind()))
        .collect::<Vec<_>>();
    let mut images = Vec::with_capacity(layouts.len());
    for (block, layout) in viewport_snapshot.blocks().iter().zip(layouts.iter()) {
        let mut block_images = Vec::new();
        for placement in layout.images() {
            let Some(image) = layout
                .projection()
                .images()
                .iter()
                .copied()
                .find(|image| image.source() == placement.source())
            else {
                continue;
            };
            let Some(key) = image_key(image) else {
                continue;
            };
            let bounds = placement.bounds();
            let bounds = Rect::new(
                bounds.x(),
                bounds.y() + block.y(),
                bounds.width(),
                bounds.height(),
            )?;
            block_images.push(ImagePrimitive::new(
                key.fingerprint(),
                bounds,
                Rgba8::new(232, 234, 238, 255),
            ));
        }
        images.push(block_images);
    }
    builder.append_viewport_with_decorations(
        &input,
        &layout_refs,
        atlas,
        font_size,
        color,
        &fills,
        &images,
        Some(viewport_table_style()),
        selection,
        Some(viewport_block_quote_color()),
    )?;
    for (block, layout) in viewport_snapshot.blocks().iter().zip(layouts.iter()) {
        let BlockKind::TaskListItem { state, .. } = block.kind() else {
            continue;
        };
        let Some(markdown_block) = document.markdown().blocks().get(block.index()) else {
            return Err(ViewportSceneError::InvalidTaskMarker {
                block: block.index(),
            });
        };
        let Some(marker) = task_marker(&source, markdown_block) else {
            return Err(ViewportSceneError::InvalidTaskMarker {
                block: block.index(),
            });
        };
        append_task_checkbox(&mut builder, layout, block.y(), marker, state)?;
    }
    for (block, layout) in viewport_snapshot.blocks().iter().zip(layouts.iter()) {
        let Some(publication) = embedded_publications.iter().find(|publication| {
            publication.revision() == revision
                && publication.source_range() == layout.projection().source_range()
        }) else {
            continue;
        };
        let EmbeddedRenderPayload::Svg { dimensions, .. } = publication.payload() else {
            continue;
        };
        let width = (dimensions.width() as f32).min(scene_viewport.width());
        let height = (dimensions.height() as f32).min(block.height().max(1.0));
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let bounds = Rect::new(0.0, block.y(), width, height)?;
        builder.embedded_svg(EmbeddedSvgPrimitive::new(
            publication.key().fingerprint(),
            publication.generation(),
            publication.kind().tag(),
            publication.source_range(),
            bounds,
            dimensions.width(),
            dimensions.height(),
            Rgba8::new(0, 0, 0, 0),
        ))?;
    }
    if let Some(style) = editor_decorations {
        append_editor_decorations(&mut builder, document, &input, &layouts, style)?;
    }
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
    assemble_viewport_render_frame_with_images(
        document,
        config.viewport(),
        config,
        shaper,
        atlas,
        render_plans,
        &[],
    )
}

/// Builds a render frame while applying ready image intrinsic dimensions.
pub fn assemble_viewport_render_frame_with_images<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
    image_publications: &[ImagePublication],
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    assemble_viewport_render_frame_with_images_and_intrinsics(
        document,
        viewport,
        config,
        shaper,
        atlas,
        render_plans,
        image_publications,
        &[],
    )
}

/// Builds a render frame while applying ready image pixels and retained
/// intrinsic dimensions.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_render_frame_with_images_and_intrinsics<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded(
        document,
        viewport,
        config,
        shaper,
        atlas,
        render_plans,
        image_publications,
        image_intrinsics,
        &[],
    )
}

/// Builds a render frame while applying ready image dimensions and explicit
/// revision-bound embedded SVG publications. The same publication list is
/// passed to scene assembly and RenderPlan construction so a command cannot
/// be emitted without its matching upload payload.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded<
    S: ShapingProvider,
>(
    document: &mut EditorDocument,
    viewport: ViewportRect,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
    embedded_publications: &[EmbeddedRenderPublication],
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    let scene = assemble_viewport_scene_with_images_and_intrinsics_and_embedded_and_table_resize(
        document,
        viewport,
        shaper,
        config.font_size(),
        config.scene_viewport(),
        atlas,
        config.color(),
        config.background(),
        image_publications,
        image_intrinsics,
        embedded_publications,
        config.table_resize(),
        config.editor_decorations(),
    )?;
    if scene.revision() != document.revision() {
        return Err(ViewportFrameError::Stale {
            expected: document.revision(),
            actual: scene.revision(),
        }
        .into());
    }
    let plan = render_plans.build_with_embedded(scene.scene(), atlas, embedded_publications)?;
    ViewportRenderFrame::new(scene, plan).map_err(ViewportSceneError::from)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yu_assets::{EmbeddedRenderPayload, EmbeddedRenderRequest, EmbeddedResourceKind};
    use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
    use yu_editor::{
        CaretAffinity, EditorCommand, EditorSelection, LayoutConfig, LayoutPoint,
        TableResizeGesture, ViewportConfig,
    };
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
            .visible_blocks_with_visual_state_and_shaper(viewport, shaper)
            .expect("viewport");
        let config = document.viewport_config().layout();
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("atlas config"));
        for block in snapshot.blocks() {
            let layout = document
                .block_layout_for_visual_state_with_shaper(block.index(), config, shaper)
                .expect("layout");
            for placement in layout.glyphs() {
                let key = GlyphRasterKey::new(
                    placement.face(),
                    placement.glyph(),
                    font_size * placement.font_scale(),
                )
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
        let viewport = ViewportRect::new(0.0, 200.0);
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
            Rect::new(0.0, 0.0, 240.0, 200.0).expect("scene viewport"),
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
        // 首个 primitive 是整帧背景，其余都应是字形。
        let primitives = frame.scene().primitives();
        assert!(matches!(primitives[0], Primitive::FillRect { .. }));
        assert!(
            primitives[1..]
                .iter()
                .all(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
    }

    #[test]
    fn blockquote_bar_precedes_glyphs_and_lowers_to_source_backed_fill() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let source = "> quoted\n> second\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("blockquote frame");
        let primitives = frame.scene().scene().primitives();
        let quote_index = primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::BlockQuote(_)))
            .expect("blockquote primitive");
        let glyph_index = primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .expect("glyph primitive");
        assert!(quote_index < glyph_index);

        let Primitive::BlockQuote(quote) = primitives[quote_index] else {
            unreachable!("located blockquote primitive");
        };
        assert_eq!(
            quote.source(),
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(source.len() as u64))
                .expect("source range")
        );
        assert_eq!(quote.color(), viewport_block_quote_color());
        assert_eq!(
            quote.bounds().height(),
            frame.scene().input().blocks()[0].height()
        );
        assert!(quote.bounds().right() < 7.0);

        let yu_render::RenderCommand::FillRect { bounds, color } =
            frame.plan().commands()[quote_index]
        else {
            panic!("blockquote must lower to a fill command");
        };
        assert_eq!(bounds, quote.bounds());
        assert_eq!(color, quote.color());
    }

    #[test]
    fn heading_and_body_share_a_frame_with_distinct_raster_sizes() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 180.0);
        let mut document = EditorDocument::new("# title\n\nplain");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 180.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("mixed typography frame");

        let glyph_sizes = frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::Glyph(glyph) => Some(glyph.key().size()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let heading = glyph_sizes
            .iter()
            .position(|size| *size == 28.0)
            .expect("H1 raster size");
        let body = glyph_sizes
            .iter()
            .position(|size| *size == 14.0)
            .expect("body raster size");
        assert!(heading < body);
        assert_eq!(
            frame.plan().commands().len(),
            frame.scene().scene().primitives().len()
        );
        assert!(frame.scene().input().blocks()[0].height() > 20.0);
    }

    #[test]
    fn empty_document_publishes_one_line_retained_caret_frame() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("empty retained frame");

        assert!(frame.scene().input().blocks().is_empty());
        assert_eq!(frame.scene().input().content_height(), 20.0);
        let primitives = frame.scene().scene().primitives();
        // 背景 + caret：空文档也必须画出背景，否则透出下层视图。
        assert_eq!(primitives.len(), 2);
        assert!(matches!(primitives[0], Primitive::FillRect { .. }));
        let Primitive::EditorDecoration(caret) = primitives[1] else {
            panic!("empty scene must contain its background and caret");
        };
        assert_eq!(caret.role(), EditorDecorationPrimitiveRole::Caret);
        assert_eq!(caret.source(), TextRange::empty(ByteOffset::new(0)));
        assert_eq!(
            caret.bounds(),
            Rect::new(0.0, 0.0, 1.0, 20.0).expect("empty caret bounds")
        );
        // 背景 + caret 两条 fill 指令。
        assert!(matches!(
            frame.plan().commands(),
            [
                yu_render::RenderCommand::FillRect { .. },
                yu_render::RenderCommand::FillRect { .. }
            ]
        ));
    }

    #[test]
    fn whitespace_document_keeps_a_submittable_retained_caret_frame() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("   \n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("whitespace retained frame");

        assert!(frame.scene().input().content_height() >= 20.0);
        assert!(!frame.plan().commands().is_empty());
        assert!(frame.scene().scene().primitives().iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::EditorDecoration(decoration)
                    if decoration.role() == EditorDecorationPrimitiveRole::Caret
            )
        }));
    }

    #[test]
    fn empty_document_composition_does_not_hide_unprojected_preedit() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        document
            .begin_composition(
                TextRange::empty(ByteOffset::new(0)),
                "日",
                Utf16Range::new(Utf16Offset::new(0), Utf16Offset::new(1))
                    .expect("preedit selection"),
            )
            .expect("begin composition");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("composition fallback frame");

        // 只剩背景：preedit 未投影时不得凭空画出内容，但背景仍要覆盖，
        // 否则会透出下层视图。
        assert!(matches!(
            frame.plan().commands(),
            [yu_render::RenderCommand::FillRect { .. }]
        ));
        assert!(matches!(
            frame.scene().scene().primitives(),
            [Primitive::FillRect { .. }]
        ));
    }

    #[test]
    fn configured_editor_decorations_publish_selection_and_caret_layers() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("alpha beta");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::range(
                    &snapshot,
                    ByteOffset::new(0),
                    ByteOffset::new(5),
                    CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("set selection");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let style = EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        );
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(style);
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("decorated frame");
        let decorations = frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::EditorDecoration(decoration) => Some(*decoration),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            decorations
                .iter()
                .filter(|decoration| {
                    decoration.role() == EditorDecorationPrimitiveRole::Selection
                })
                .count(),
            1
        );
        let caret = decorations
            .iter()
            .find(|decoration| decoration.role() == EditorDecorationPrimitiveRole::Caret)
            .expect("caret decoration");
        assert_eq!(caret.source(), TextRange::empty(ByteOffset::new(5)));
        assert_eq!(caret.bounds().width(), 1.0);
        assert_eq!(
            frame.plan().commands().len(),
            frame.scene().scene().primitives().len()
        );
    }

    #[test]
    fn retained_frame_reveals_active_markdown_syntax_without_source_edit() {
        let source = "# before **strong** after\n\nplain";
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new(source);
        // 基线要求「没有任何语法被聚焦」。新文档的光标落在文首，正好会揭示
        // 第一个块的语法，因此这里把它显式移到最后一个普通段落里。
        {
            let snapshot = document.snapshot();
            document
                .set_selection(
                    EditorSelection::cursor(
                        &snapshot,
                        snapshot.len_bytes(),
                        CaretAffinity::Downstream,
                    )
                    .expect("baseline caret"),
                )
                .expect("baseline selection");
        }
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let style = EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        );
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(style);
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut plans = RenderPlanBuilder::new();
        let hidden =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("hidden frame");
        let hidden_glyphs = hidden
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .count();
        let revision = document.revision();

        let strong = source.find("strong").expect("strong content");
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new((strong + 2) as u64),
                    CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("set selection");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let revealed =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("revealed frame");
        let revealed_glyphs = revealed
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .count();

        assert_eq!(document.revision(), revision);
        assert_eq!(revealed.revision(), revision);
        assert_eq!(revealed_glyphs, hidden_glyphs + 6);
        let caret = revealed
            .scene()
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::EditorDecoration(decoration)
                    if decoration.role() == EditorDecorationPrimitiveRole::Caret =>
                {
                    Some(*decoration)
                }
                _ => None,
            })
            .expect("revealed caret");
        assert_eq!(
            caret.source(),
            TextRange::empty(ByteOffset::new((strong + 2) as u64))
        );
        assert!(caret.bounds().x() > 0.0);
    }

    #[test]
    fn composition_decorations_use_transient_projection_without_source_edit() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("日本 alpha");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let revision = document.revision();
        document
            .begin_composition(
                TextRange::new(ByteOffset::new(7), ByteOffset::new(12)).expect("replacement"),
                "日本",
                Utf16Range::new(Utf16Offset::new(0), Utf16Offset::new(1))
                    .expect("preedit selection"),
            )
            .expect("begin composition");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("composition frame");
        assert_eq!(document.revision(), revision);
        let roles = frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::EditorDecoration(decoration) => Some(decoration.role()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(roles.contains(&EditorDecorationPrimitiveRole::Selection));
        assert!(roles.contains(&EditorDecorationPrimitiveRole::CompositionCaret));
        assert!(!roles.contains(&EditorDecorationPrimitiveRole::Caret));
    }

    #[test]
    fn task_markers_become_source_backed_checkbox_layers() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("- [ ] todo\n- [x] done\n");
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
        .expect("task scene");
        let layers = frame
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::TaskCheckbox(task) => Some(*task),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(layers.len(), 7);
        let snapshot = document.snapshot();
        let todo = task_marker(
            &snapshot,
            document.markdown().blocks().get(0).expect("todo block"),
        )
        .expect("todo marker")
        .range();
        let done = task_marker(
            &snapshot,
            document.markdown().blocks().get(1).expect("done block"),
        )
        .expect("done marker")
        .range();
        assert_eq!(
            layers.iter().filter(|layer| layer.source() == todo).count(),
            2
        );
        assert_eq!(
            layers.iter().filter(|layer| layer.source() == done).count(),
            5
        );
        assert!(layers.iter().any(|layer| {
            layer.source() == todo && layer.role() == TaskCheckboxPrimitiveRole::Interior
        }));
        assert!(layers.iter().any(|layer| {
            layer.source() == done && layer.role() == TaskCheckboxPrimitiveRole::Check
        }));
        assert!(
            layers
                .iter()
                .all(|layer| { layer.bounds().width() > 0.0 && layer.bounds().height() > 0.0 })
        );

        let initial_revision = document.revision();
        document
            .execute(EditorCommand::toggle_task(0))
            .expect("toggle todo");
        assert!(document.revision() > initial_revision);
        let toggled_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let toggled = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            &toggled_atlas,
            Rgba8::black(),
        )
        .expect("toggled task scene");
        assert_eq!(
            toggled
                .scene()
                .primitives()
                .iter()
                .filter(|primitive| {
                    matches!(
                        primitive,
                        Primitive::TaskCheckbox(layer) if layer.source() == todo
                    )
                })
                .count(),
            5
        );
    }

    #[test]
    fn task_checkbox_hit_test_uses_published_revision_and_border_geometry() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 120.0);
        let mut document = EditorDocument::new("- [ ] todo\nparagraph\n- [x] done\n");
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
        .expect("task scene");
        let border = frame
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::TaskCheckbox(task)
                    if task.role() == TaskCheckboxPrimitiveRole::Border =>
                {
                    Some(*task)
                }
                _ => None,
            })
            .expect("todo checkbox border");
        let bounds = border.bounds();
        let point = Point::new(
            bounds.x() + bounds.width() * 0.5,
            bounds.y() + bounds.height() * 0.5,
        );
        let hit = frame
            .task_checkbox_hit_test(frame.revision(), point)
            .expect("current revision")
            .expect("checkbox hit");
        assert_eq!(hit.revision(), document.revision());
        assert_eq!(hit.block_index(), 0);
        assert_eq!(hit.source(), border.source());
        assert_eq!(hit.bounds(), bounds);
        assert_eq!(
            frame
                .task_checkbox_hit_test(
                    frame.revision(),
                    Point::new(bounds.right() + 1.0, bounds.y())
                )
                .expect("outside query"),
            None
        );
        assert!(matches!(
            frame.task_checkbox_hit_test(Revision::new(frame.revision().get() + 1), point),
            Err(ViewportFrameError::Stale { .. })
        ));
        assert_eq!(
            frame
                .task_checkbox_hit_test(frame.revision(), Point::new(f32::NAN, bounds.y()))
                .expect("non-finite point is not a hit"),
            None
        );
    }

    #[test]
    fn published_math_is_consumed_by_viewport_scene_and_render_plan() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 240.0);
        let mut document = EditorDocument::new("```math\nx^2 + y^2\n```\n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let projection = document.block_projection(0).expect("math projection");
        let yu_editor::BlockProjection::FencedCode(code) = projection else {
            panic!("expected fenced math block");
        };
        let source_range = code.source_range();
        let request = EmbeddedRenderRequest::new(
            document.revision(),
            source_range,
            EmbeddedResourceKind::Math,
            "x^2 + y^2",
        )
        .expect("math request");
        let mut cache = yu_assets::EmbeddedResourceCache::new();
        let publication = cache
            .publish(
                request,
                document.revision(),
                EmbeddedRenderPayload::svg(640, 320, "<svg viewBox=\"0 0 640 320\"/>")
                    .expect("math SVG"),
            )
            .expect("math publication");
        let mut plans = RenderPlanBuilder::new();
        let frame = assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded(
            &mut document,
            viewport,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 240.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &atlas,
            &mut plans,
            &[],
            &[],
            std::slice::from_ref(&publication),
        )
        .expect("embedded frame");
        assert!(frame.scene().scene().primitives().iter().any(|primitive| {
            matches!(primitive, Primitive::EmbeddedSvg(svg) if svg.source() == source_range)
        }));
        assert!(frame.plan().commands().iter().any(|command| {
            matches!(command, yu_render::RenderCommand::EmbeddedSvg {
                resource,
                generation,
                ..
            } if *resource == publication.key().fingerprint()
                && *generation == publication.generation())
        }));
        assert_eq!(frame.plan().embedded_uploads().len(), 1);
        assert_eq!(frame.plan().embedded_uploads()[0].source(), source_range);
    }

    #[test]
    fn ready_image_publication_updates_scene_intrinsic_bounds() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 160.0);
        let mut document = EditorDocument::new("![alt](image.png)");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let source = document
            .block_projection(0)
            .expect("image projection")
            .visual()
            .images()[0]
            .source();
        let mut cache = yu_assets::ImageCache::new();
        let publication = cache
            .publish_decoded(
                yu_assets::ImageRequest::new(document.revision(), source, "image.png")
                    .expect("image request"),
                document.revision(),
                yu_assets::DecodedImage::new(200, 100, vec![255; 200 * 100 * 4])
                    .expect("decoded image"),
            )
            .expect("image publication");
        let intrinsic = publication.intrinsic_publication();
        let frame = assemble_viewport_scene_with_images(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
            &[publication],
        )
        .expect("scene frame");
        let image = frame
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::Image(image) => Some(*image),
                Primitive::FillRect { .. }
                | Primitive::Glyph(_)
                | Primitive::EmbeddedSvg(_)
                | Primitive::BlockQuote(_)
                | Primitive::Table(_)
                | Primitive::TaskCheckbox(_)
                | Primitive::EditorDecoration(_) => None,
            })
            .expect("image primitive");
        assert_eq!(image.bounds().width(), 200.0);
        assert_eq!(image.bounds().height(), 100.0);
        assert!(frame.input().content_height() >= 100.0);

        let metadata_only = assemble_viewport_scene_with_images_and_intrinsics(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
            &[],
            &[intrinsic],
        )
        .expect("metadata-only scene frame");
        let metadata_image = metadata_only
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::Image(image) => Some(*image),
                Primitive::FillRect { .. }
                | Primitive::Glyph(_)
                | Primitive::EmbeddedSvg(_)
                | Primitive::BlockQuote(_)
                | Primitive::Table(_)
                | Primitive::TaskCheckbox(_)
                | Primitive::EditorDecoration(_) => None,
            })
            .expect("metadata-only image primitive");
        assert_eq!(metadata_image.bounds().width(), 200.0);
        assert_eq!(metadata_image.bounds().height(), 100.0);
        assert!(metadata_only.input().content_height() >= 100.0);
    }

    #[test]
    fn fenced_code_viewport_emits_fill_before_glyphs() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 160.0);
        let mut document = EditorDocument::new("```rust\nlet x = 1;\n```\n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &atlas,
            &mut RenderPlanBuilder::new(),
        )
        .expect("code block frame");

        let primitives = frame.scene().scene().primitives();
        // 跳过整帧背景，再检查代码块自己的底色先于字形。
        let Some((background, primitives)) = primitives.split_first() else {
            panic!("code block scene should not be empty");
        };
        assert!(matches!(background, Primitive::FillRect { .. }));
        let Some((first, rest)) = primitives.split_first() else {
            panic!("code block scene should carry its own fill");
        };
        match first {
            Primitive::FillRect { bounds, color } => {
                assert_eq!(*color, Rgba8::new(245, 246, 248, 255));
                assert_eq!(bounds.x(), 0.0);
                assert_eq!(bounds.y(), 0.0);
                assert_eq!(bounds.width(), 240.0);
                assert!(bounds.height() > 0.0);
            }
            Primitive::Glyph(_)
            | Primitive::Image(_)
            | Primitive::EmbeddedSvg(_)
            | Primitive::BlockQuote(_)
            | Primitive::Table(_)
            | Primitive::TaskCheckbox(_)
            | Primitive::EditorDecoration(_) => {
                panic!("code block background must precede glyphs")
            }
        }
        assert!(
            rest.iter()
                .any(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
        assert!(matches!(
            frame.plan().commands().first(),
            Some(yu_render::RenderCommand::FillRect { .. })
        ));
        assert!(
            frame
                .plan()
                .commands()
                .iter()
                .any(|command| matches!(command, yu_render::RenderCommand::Glyph { .. }))
        );
    }

    #[test]
    fn table_viewport_emits_decorations_before_source_backed_cell_glyphs() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let viewport = ViewportRect::new(0.0, 160.0);
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let body_offset = ByteOffset::new(source.rfind('2').expect("body cell") as u64);
        let selection = EditorSelection::range(
            &document.snapshot(),
            body_offset,
            ByteOffset::new(body_offset.get() + 1),
            CaretAffinity::Downstream,
        )
        .expect("selection");
        document.set_selection(selection).expect("set selection");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("table scene frame");

        let primitives = frame.scene().primitives();
        assert!(matches!(primitives[0], Primitive::FillRect { .. }));
        // 背景之后、首个字形之前只应有表格装饰。
        let primitives = &primitives[1..];
        let first_glyph = primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .expect("cell glyph");
        assert!(
            primitives[..first_glyph]
                .iter()
                .all(|primitive| matches!(primitive, Primitive::Table(_)))
        );
        assert!(primitives[..first_glyph].iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::Table(table) if table.role() == yu_scene::TablePrimitiveRole::HeaderFill
            )
        }));
        assert!(primitives[..first_glyph].iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::Table(table)
                    if table.role() == yu_scene::TablePrimitiveRole::SelectionFill
                        && table.source().contains(body_offset)
            )
        }));
        assert!(
            primitives[first_glyph..]
                .iter()
                .any(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
    }

    #[test]
    fn table_resize_override_reaches_scene_and_render_plan_without_mutating_document() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 160.0);
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let layout_config = document.viewport_config().layout();
        let canonical = document
            .block_layout_with_shaper(0, layout_config, &shaper)
            .expect("canonical table layout")
            .clone();
        let table = canonical.table().expect("table metadata");
        let divider = table.bounds().x() + table.column_widths()[0];
        let hit = table
            .resize_hit_test(LayoutPoint::new(divider, 0.5), 0.0)
            .expect("divider hit-test")
            .expect("column divider");
        let mut gesture = TableResizeGesture::begin(canonical.revision(), 0, hit, divider)
            .expect("resize gesture");
        gesture
            .update(canonical.revision(), divider + 1.0)
            .expect("resize update");
        let commit = gesture.finish(canonical.revision()).expect("resize commit");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
                Rgba8::black(),
            )
            .with_table_resize(commit),
            &shaper,
            &atlas,
            &mut RenderPlanBuilder::new(),
        )
        .expect("transient table render frame");

        let expected_divider = divider + 1.0;
        assert!(frame.scene().scene().primitives().iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::Table(table)
                    if table.role() == yu_scene::TablePrimitiveRole::Border
                        && (table.bounds().x() - expected_divider).abs() < 0.001
            )
        }));
        assert!(frame.plan().commands().iter().any(|command| {
            matches!(
                command,
                yu_render::RenderCommand::FillRect { bounds, .. }
                    if (bounds.x() - expected_divider).abs() < 0.001
            )
        }));
        assert_eq!(document.snapshot().as_str(), source);
        let canonical_after = document
            .block_layout_with_shaper(0, layout_config, &shaper)
            .expect("canonical layout after frame");
        assert!(
            (canonical_after.table().expect("table").column_widths()[0] - table.column_widths()[0])
                .abs()
                < 0.001
        );
        assert_eq!(document.layout_cache_stats().entries(), 1);

        document
            .apply_transaction(&yu_text::Transaction::new(
                canonical.revision(),
                [yu_text::Edit::new(
                    yu_core::TextRange::empty(ByteOffset::ZERO),
                    "前",
                )],
            ))
            .expect("source edit");
        let stale_error = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
                Rgba8::black(),
            )
            .with_table_resize(commit),
            &shaper,
            &atlas,
            &mut RenderPlanBuilder::new(),
        )
        .expect_err("stale table override");
        assert!(matches!(
            stale_error,
            ViewportSceneError::Document(EditorDocumentError::Layout(
                yu_editor::LayoutError::InvalidTable(
                    "table resize and viewport document revisions differ"
                )
            ))
        ));
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

    #[test]
    fn frame_publisher_failure_does_not_commit_builder_or_cache() {
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
        let mut initial_plans = RenderPlanBuilder::new();
        let mut publisher = ViewportFramePublisher::new();
        let first = publisher
            .publish(&mut document, config, &shaper, &atlas, &mut initial_plans)
            .expect("initial publication");
        let first_revision = first.revision();

        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit");
        let new_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut retry_plans = RenderPlanBuilder::new();
        publisher.next_serial = u64::MAX;

        let error = publisher
            .publish(&mut document, config, &shaper, &new_atlas, &mut retry_plans)
            .expect_err("serial overflow must reject publication");
        assert_eq!(error, ViewportPublishError::SerialOverflow);
        assert_eq!(retry_plans.uploaded_page_count(), 0);
        assert_eq!(publisher.current_revision(), Some(first_revision));
        assert_eq!(publisher.last_publication(), Some(&first));
        assert_eq!(publisher.next_serial, u64::MAX);

        publisher.next_serial = first.serial();
        let retry = publisher
            .publish(&mut document, config, &shaper, &new_atlas, &mut retry_plans)
            .expect("retry after overflow");
        assert_eq!(retry.serial(), 2);
        assert_eq!(retry.revision(), document.revision());
        assert!(retry_plans.uploaded_page_count() > 0);
    }

    /// 守护不变量 I5：Rust 渲染器是唯一渲染路径，不存在第二条兜底路径。
    ///
    /// parser 能产出的每一种 block kind 都必须能被投影、布局并产出 glyph。
    /// 一旦某个 kind 画不出来，删掉 TextKit fallback 后它就会变成空白区域，
    /// 因此这条断言是「允许删除 fallback」的前提证明，不能降级为警告。
    #[test]
    fn every_parser_block_kind_produces_renderable_glyphs() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportRect::new(0.0, 900.0);
        let source = concat!(
            "# heading\n",
            "\n",
            "paragraph with **bold**\n",
            "\n",
            "> quoted line\n",
            "\n",
            "- list item\n",
            "- [ ] task item\n",
            "\n",
            "```rust\n",
            "code line\n",
            "```\n",
            "\n",
            "| a | b |\n",
            "| --- | --- |\n",
            "| 1 | 2 |\n",
            "\n",
            "[ref]: /url\n",
        );
        let mut document = EditorDocument::new(source);
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
            Rect::new(0.0, 0.0, 240.0, 900.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("scene frame");

        // 全部 8 种 parser block kind 都必须出现在这一个 viewport 里，
        // 否则这条测试就没有真正覆盖到它声称覆盖的范围。
        let kinds: std::collections::BTreeSet<u8> = frame
            .input()
            .blocks()
            .iter()
            .map(|block| block.kind())
            .collect();
        let expected: std::collections::BTreeSet<u8> = (0..=7).collect();
        assert_eq!(
            kinds, expected,
            "fixture 未覆盖全部 block kind，实际 {kinds:?}"
        );

        // 每个非空行 block 都必须产出 glyph。
        let config = document.viewport_config().layout();
        for block in frame.input().blocks() {
            if block.kind() == 0 {
                continue; // BlankLine 没有可见字形
            }
            let layout = document
                .block_layout_for_visual_state_with_shaper(block.index(), config, &shaper)
                .expect("block layout");
            assert!(
                !layout.glyphs().is_empty(),
                "block kind {} (source {:?}) 没有产出任何 glyph，\
                 删除 TextKit fallback 后它会变成空白",
                block.kind(),
                block.source()
            );
        }

        // 这些 glyph 必须真的进入 scene，而不是只存在于 layout 里。
        let glyph_count = frame
            .scene()
            .primitives()
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .count();
        assert!(glyph_count > 0, "scene 中没有任何 glyph primitive");
    }
}
