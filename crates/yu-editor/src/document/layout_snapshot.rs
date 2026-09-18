//! Immutable, source-backed geometry shared by painting and native queries.

use super::*;
use crate::{BlockCaret, BlockHit, ViewportBlock};
use std::sync::atomic::{AtomicU64, Ordering};
use yu_layout::LayoutRect;

static NEXT_GEOMETRY: AtomicU64 = AtomicU64::new(1);

/// A native geometry request, addressed by visible coordinates or source.
#[derive(Clone, Copy, Debug)]
pub enum LayoutQuery {
    Viewport(ViewportSpan),
    Source(ByteOffset),
}
impl From<ViewportSpan> for LayoutQuery {
    fn from(value: ViewportSpan) -> Self {
        Self::Viewport(value)
    }
}
impl From<ByteOffset> for LayoutQuery {
    fn from(value: ByteOffset) -> Self {
        Self::Source(value)
    }
}

/// One paragraph's exact layout, positioned in document content coordinates.
/// Reading-column padding and the viewport transform are applied outside this
/// coordinate space; no consumer reconstructs a paragraph's origin.
#[derive(Clone, Debug)]
pub struct SnapshotBlock {
    metadata: ViewportBlock,
    layout: Arc<BlockView>,
    content_inset: f32,
}

impl SnapshotBlock {
    pub fn metadata(&self) -> ViewportBlock {
        self.metadata
    }
    pub fn layout_handle(&self) -> Arc<BlockView> {
        Arc::clone(&self.layout)
    }
    pub fn layout(&self) -> &BlockView {
        &self.layout
    }
    pub fn content_y(&self) -> f32 {
        self.metadata.y() + self.content_inset
    }
    pub fn document_point(&self, local: LayoutPoint) -> LayoutPoint {
        LayoutPoint::new(local.x(), self.content_y() + local.y())
    }
    pub fn local_point(&self, document: LayoutPoint) -> LayoutPoint {
        LayoutPoint::new(document.x(), document.y() - self.content_y())
    }
    /// Cell paragraphs can have several text lines within one table row.
    pub fn caret_height(&self, caret: BlockCaret) -> f32 {
        self.layout.caret_line_height(caret)
    }
}

/// The measured part of a semantic container within this snapshot's coverage.
/// Bounds are document coordinates; clipping flags distinguish a partial
/// viewport measurement from a fully measured container.
#[derive(Clone, Debug)]
pub struct SnapshotContainer {
    pub node: usize,
    pub kind: yu_markdown::PresentationKind,
    pub source: TextRange,
    pub bounds: LayoutRect,
    pub clipped_start: bool,
    pub clipped_end: bool,
    pub quote_bar: Option<LayoutRect>,
}

fn container_geometry(
    context: &LayoutContext,
    blocks: &[SnapshotBlock],
) -> Result<Vec<SnapshotContainer>, LayoutError> {
    use yu_markdown::PresentationKind;
    let config = context.viewport_config().layout();
    let metrics = crate::layout_tokens::ContainerMetrics::new(config);
    let tree = context.markdown.presentation();
    let mut containers = std::collections::BTreeMap::<usize, SnapshotContainer>::new();
    for block in blocks {
        if block.metadata.height() <= 0.0 {
            continue;
        }
        let mut x = 0.0;
        let mut inside_list = false;
        for id in tree.path_for_range(block.metadata.source()) {
            let node = &tree.nodes()[id];
            if !matches!(
                node.kind,
                PresentationKind::Quote
                    | PresentationKind::List { .. }
                    | PresentationKind::ListItem
            ) {
                continue;
            }
            let continues = block.metadata.source().end() < tree.content_end(id);
            let bottom = if continues {
                block.metadata.y() + block.metadata.height()
            } else {
                block.content_y()
                    + block.layout.height()
                    + content_bottom_inset(block.metadata.kind(), block.layout.config())
            };
            let top = containers.get(&id).map_or(
                block.content_y() - content_origin_y(block.metadata.kind(), block.layout.config()),
                |entry| entry.bounds.y(),
            );
            let container_x = x + if node.kind == PresentationKind::Quote && !inside_list {
                metrics.quote_margin_left
            } else {
                0.0
            };
            let bounds = LayoutRect::new(
                container_x,
                top,
                (config.max_width() - container_x).max(1.0),
                (bottom - top).max(0.0),
            )?;
            let clipped_start = blocks
                .first()
                .is_some_and(|block| block.metadata.source().start() > node.source.start());
            let clipped_end = blocks
                .last()
                .is_some_and(|block| block.metadata.source().end() < node.source.end());
            containers.insert(
                id,
                SnapshotContainer {
                    node: id,
                    kind: node.kind,
                    source: node.source,
                    bounds,
                    clipped_start,
                    clipped_end,
                    quote_bar: if node.kind == PresentationKind::Quote {
                        Some(LayoutRect::new(
                            container_x,
                            top,
                            metrics.quote_border,
                            bounds.height(),
                        )?)
                    } else {
                        None
                    },
                },
            );
            x += match node.kind {
                PresentationKind::Quote => {
                    metrics.quote_indent
                        - if inside_list {
                            metrics.quote_margin_left
                        } else {
                            0.0
                        }
                }
                PresentationKind::ListItem => {
                    inside_list = true;
                    metrics.list_indent
                }
                _ => 0.0,
            };
        }
    }
    Ok(containers.into_values().collect())
}

/// A completed geometry version. Its paragraphs include active syntax reveal,
/// composition, image dimensions and table previews, exactly as painted.
/// Cloning shares the layouts and does not invoke a shaper or copy source text.
#[derive(Clone, Debug)]
pub struct LayoutSnapshot {
    id: u64,
    pub(super) measurements: LayoutMeasurements,
    source: TextSnapshot,
    viewport: ViewportSnapshot,
    blocks: Arc<[SnapshotBlock]>,
    containers: Arc<[SnapshotContainer]>,
    resource_geometry_version: u64,
    table_resize: Option<TableResizeCommit>,
}

// Equality identifies an immutable publication, not two independently prepared
// frames which happen to contain visually equal pixels.
impl PartialEq for LayoutSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl LayoutSnapshot {
    pub(super) fn new(
        context: &LayoutContext,
        viewport: ViewportSnapshot,
        layouts: Vec<Arc<BlockView>>,
        table_resize: Option<TableResizeCommit>,
    ) -> Result<Self, EditorDocumentError> {
        if viewport.revision() != context.revision()
            || layouts.len() != viewport.blocks().len()
            || layouts
                .iter()
                .zip(viewport.blocks())
                .any(|(layout, block)| {
                    layout.revision() != context.revision()
                        || layout.source_range() != block.source()
                        || layout.config()
                            != crate::layout_tokens::box_layout_config_with_quote(
                                block.kind(),
                                context.viewport_config().layout(),
                                layout.ornaments().quote().is_some(),
                            )
                })
        {
            return Err(LayoutError::Upstream(
                "snapshot paragraphs do not match their placement or source".into(),
            )
            .into());
        }
        let blocks: Arc<[SnapshotBlock]> = viewport
            .blocks()
            .iter()
            .copied()
            .zip(layouts)
            .map(|(metadata, layout)| SnapshotBlock {
                content_inset: context.block_content_origin(metadata.index(), layout.config()),
                metadata,
                layout,
            })
            .collect();
        let containers = container_geometry(context, &blocks)?.into();
        Ok(Self {
            containers,
            id: NEXT_GEOMETRY.fetch_add(1, Ordering::Relaxed),
            measurements: context.measurements(),
            source: context.snapshot(),
            viewport,
            blocks,
            resource_geometry_version: context.resource_geometry_version,
            table_resize,
        })
    }
    pub fn geometry_version(&self) -> u64 {
        self.id
    }
    pub fn resource_geometry_version(&self) -> u64 {
        self.resource_geometry_version
    }
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }
    pub fn source(&self) -> &TextSnapshot {
        &self.source
    }
    pub fn config(&self) -> LayoutConfig {
        self.measurements.viewport.config().layout()
    }
    pub fn viewport(&self) -> &ViewportSnapshot {
        &self.viewport
    }
    pub fn layout_pending(&self) -> bool {
        !self.measurements.viewport.unmeasured_sources(1).is_empty()
    }

    pub fn containers(&self) -> &[SnapshotContainer] {
        &self.containers
    }
    pub fn blocks(&self) -> &[SnapshotBlock] {
        &self.blocks
    }
    pub fn content_height(&self) -> f32 {
        if self.source.is_empty() && self.blocks.is_empty() {
            self.config().line_height()
        } else {
            self.viewport.content_height()
        }
    }
    pub fn block(&self, index: usize) -> Option<&SnapshotBlock> {
        let offset = self
            .blocks
            .partition_point(|block| block.metadata.index() < index);
        self.blocks
            .get(offset)
            .filter(|block| block.metadata.index() == index)
    }
    pub fn block_for_source(&self, source: ByteOffset) -> Option<&SnapshotBlock> {
        let next = self
            .blocks
            .partition_point(|block| block.metadata.source().start() <= source);
        let block = self.blocks.get(next.checked_sub(1)?)?;
        (block.metadata.source().contains(source) || block.metadata.source().end() == source)
            .then_some(block)
    }
    /// Zero-height separators do not mask the visible paragraph at the same y.
    pub fn hit_test(
        &self,
        point: LayoutPoint,
    ) -> Result<Option<(&SnapshotBlock, BlockHit)>, LayoutError> {
        if !point.x().is_finite() || !point.y().is_finite() {
            return Err(LayoutError::Upstream("hit point must be finite".into()));
        }
        let mut selected = None;
        let mut distance = f32::INFINITY;
        for block in self
            .blocks
            .iter()
            .filter(|block| block.metadata.height() > 0.0 && !block.layout.lines().is_empty())
        {
            let top = block.metadata.y();
            let bottom = top + block.metadata.height();
            let next = (top - point.y()).max(point.y() - bottom).max(0.0);
            // At a shared boundary the following visible block owns the
            // point; choosing the preceding bottom edge jumps to its last line.
            if next <= distance {
                selected = Some(block);
                distance = next;
            }
        }
        selected
            .map(|block| {
                block
                    .layout
                    .hit_test(block.local_point(point))
                    .map(|hit| (block, hit))
            })
            .transpose()
    }
    pub fn caret_scroll_request(
        &self,
        block: &SnapshotBlock,
        caret: BlockCaret,
        viewport: ViewportSpan,
        margin: f32,
    ) -> Result<CaretScrollRequest, EditorDocumentError> {
        viewport.validate()?;
        validate_caret_margin(margin)?;
        let point = block.document_point(caret.point());
        let height = block.caret_height(caret);
        let margin = margin.min(viewport.height() / 2.0);
        let mut target = viewport.scroll_y();
        if point.y() < target + margin {
            target = point.y() - margin;
        } else if point.y() + height > target + viewport.height() - margin {
            target = point.y() + height + margin - viewport.height();
        }
        target = target.clamp(0.0, (self.content_height() - viewport.height()).max(0.0));
        Ok(CaretScrollRequest::new(
            self.revision(),
            ViewportCaret::new(
                caret.source(),
                block.metadata.index(),
                point.x(),
                point.y(),
                0.0,
                height,
            )?,
            viewport.scroll_y(),
            target,
            margin,
            (target - viewport.scroll_y()).abs() > f32::EPSILON,
        ))
    }
    pub(super) fn covers(&self, span: ViewportSpan) -> bool {
        match (self.blocks.first(), self.blocks.last()) {
            (Some(first), Some(last)) => {
                let bottom = (span.scroll_y() + span.height()).min(self.content_height());
                span.scroll_y().min(self.content_height()) >= first.metadata.y()
                    && bottom <= last.metadata.y() + last.metadata.height()
            }
            _ => self.source.is_empty(),
        }
    }
    pub(super) fn table_resize(&self) -> Option<TableResizeCommit> {
        self.table_resize
    }
}

impl LayoutContext {
    /// Bounded measurement batch. Do not construct a document-wide snapshot for
    /// each paragraph: publication takes one immutable snapshot after the batch.
    pub fn measure_background_paragraphs<S, F, C>(
        &mut self,
        shaper: &S,
        resolver: &F,
        resize: Option<TableResizeCommit>,
        cancel: &mut C,
    ) -> Result<usize, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
        C: FnMut() -> bool,
    {
        let started = std::time::Instant::now();
        let config = self.viewport_config().layout();
        let mut measured = 0;
        let focus = self.block_index_for_source(self.selection().focus());
        let composition = self.composition_block_range();
        for source in self.pending_layout_sources(128) {
            if cancel() {
                return Err(EditorDocumentError::Cancelled);
            }
            if measured > 0 && started.elapsed() >= std::time::Duration::from_millis(8) {
                break;
            }
            let Some(index) = self.block_index_for_source(source) else {
                continue;
            };
            // Inactive separator/empty paragraphs have structural height but
            // no glyphs to shape. Active input still uses the normal path.
            if self
                .markdown
                .blocks()
                .get(index)
                .is_some_and(|block| block.kind() == BlockKind::BlankLine)
                && focus != Some(index)
                && !composition
                    .as_ref()
                    .is_some_and(|span| span.contains(&index))
            {
                let height = self.block_box_height(index, 0.0, config);
                self.viewport.set_block_height(index, height)?;
                measured += 1;
                continue;
            }
            let sizes = self.block_image_sizes(index, resolver)?;
            let mut layout = self.block_layout_for_visual_state_with_shaper_and_images(
                index, config, shaper, &sizes,
            )?;
            if let Some(resize) = resize.filter(|resize| resize.block_index() == index) {
                layout.apply_table_resize_with_shaper(resize, shaper)?;
            }
            let height = self.block_box_height(index, layout.height(), config);
            self.viewport.set_block_height(index, height)?;
            measured += 1;
        }
        if measured > 0 {
            self.layout_snapshot = None;
        }
        Ok(measured)
    }

    /// Only snapshots matching the live source, projection, width and resource
    /// geometry may answer native synchronous queries.
    pub fn current_layout_snapshot(&self) -> Option<Arc<LayoutSnapshot>> {
        self.layout_snapshot
            .as_ref()
            .filter(|snapshot| {
                self.accepts_layout_snapshot(snapshot)
                    && self.viewport.geometry_generation()
                        == snapshot.measurements.viewport.geometry_generation()
            })
            .cloned()
    }
    pub fn accepts_layout_snapshot(&self, snapshot: &LayoutSnapshot) -> bool {
        self.accepts_measurements(&snapshot.measurements)
            && self.resource_geometry_version == snapshot.resource_geometry_version
    }
    pub fn set_resource_geometry_version(&mut self, version: u64) {
        self.resource_geometry_version = version;
    }
    /// Merge worker measurements before accepting its pixels. If foreground
    /// measurements change the published block origins, keep the work but
    /// request a new frame instead of pairing old pixels with new hit geometry.
    pub fn integrate_layout_measurements(&mut self, snapshot: &LayoutSnapshot) -> bool {
        if !self.accepts_layout_snapshot(snapshot) {
            return false;
        }
        if !self.adopt_measurements(snapshot.measurements.clone()) {
            return false;
        }
        snapshot.blocks().iter().all(|block| {
            let index = block.metadata().index();
            (self.viewport.height_index().prefix_height(index) - block.metadata().y()).abs() < 0.001
        })
    }

    pub fn adopt_layout_snapshot(&mut self, snapshot: Arc<LayoutSnapshot>) -> bool {
        if !self.accepts_layout_snapshot(&snapshot) {
            return false;
        }
        let adopted = self.adopt_measurements(snapshot.measurements.clone());
        if adopted {
            self.layout_snapshot = Some(snapshot);
        }
        adopted
    }
    pub fn prepare_layout_snapshot<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
    ) -> Result<Arc<LayoutSnapshot>, EditorDocumentError> {
        self.prepare_layout_snapshot_with_images(viewport, shaper, &|_| None, None)
    }
    pub fn prepare_layout_snapshot_with_images<S, F>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: &F,
        table_resize: Option<TableResizeCommit>,
    ) -> Result<Arc<LayoutSnapshot>, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
    {
        self.prepare_layout_snapshot_with_images_cancelable(
            viewport,
            shaper,
            image_resolver,
            table_resize,
            &mut || false,
        )
    }
    pub fn prepare_layout_snapshot_with_images_cancelable<S, F, C>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: &F,
        table_resize: Option<TableResizeCommit>,
        should_cancel: &mut C,
    ) -> Result<Arc<LayoutSnapshot>, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
        C: FnMut() -> bool,
    {
        if should_cancel() {
            return Err(EditorDocumentError::Cancelled);
        }
        viewport.validate()?;
        if let Some(snapshot) = self.current_layout_snapshot()
            && snapshot.table_resize() == table_resize
            && snapshot.covers(viewport)
        {
            let mut changed = false;
            for block in snapshot.blocks() {
                let sizes = self.block_image_sizes(block.metadata().index(), image_resolver)?;
                changed |= block.layout().needs_widget_rebuild(&sizes);
            }
            if !changed {
                return Ok(snapshot);
            }
        }
        if let Some(resize) = table_resize {
            self.validate_table_resize_commit(resize.block_index(), resize)?;
        }
        let viewport_snapshot = self.visible_blocks_with_visual_state_and_images_cancelable(
            viewport,
            shaper,
            image_resolver,
            should_cancel,
        )?;
        let config = self.viewport_config().layout();
        let mut layouts = Vec::with_capacity(viewport_snapshot.blocks().len());
        for block in viewport_snapshot.blocks() {
            if should_cancel() {
                return Err(EditorDocumentError::Cancelled);
            }
            let sizes = self.block_image_sizes(block.index(), image_resolver)?;
            let mut layout = self.block_layout_for_visual_state_with_shaper_and_images(
                block.index(),
                config,
                shaper,
                &sizes,
            )?;
            if let Some(resize) =
                table_resize.filter(|resize| resize.block_index() == block.index())
            {
                layout.apply_table_resize_with_shaper(resize, shaper)?;
                let height = self.block_box_height(block.index(), layout.height(), config);
                self.viewport.set_block_height(block.index(), height)?;
            }
            layouts.push(Arc::new(layout));
        }
        let viewport_snapshot = self
            .viewport
            .snapshot(&self.markdown, viewport_snapshot.range())?;
        let snapshot = Arc::new(LayoutSnapshot::new(
            self,
            viewport_snapshot,
            layouts,
            table_resize,
        )?);
        self.layout_snapshot = Some(Arc::clone(&snapshot));
        Ok(snapshot)
    }
    pub fn prepare_query_layout_snapshot_with_images<S, F>(
        &mut self,
        query: LayoutQuery,
        shaper: &S,
        image_resolver: &F,
        table_resize: Option<TableResizeCommit>,
    ) -> Result<Arc<LayoutSnapshot>, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
    {
        self.prepare_query_layout_snapshot_with_images_cancelable(
            query,
            shaper,
            image_resolver,
            table_resize,
            &mut || false,
        )
    }
    pub fn prepare_query_layout_snapshot_with_images_cancelable<S, F, C>(
        &mut self,
        query: LayoutQuery,
        shaper: &S,
        image_resolver: &F,
        table_resize: Option<TableResizeCommit>,
        should_cancel: &mut C,
    ) -> Result<Arc<LayoutSnapshot>, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
        C: FnMut() -> bool,
    {
        if should_cancel() {
            return Err(EditorDocumentError::Cancelled);
        }
        let source = match query {
            LayoutQuery::Viewport(viewport) => {
                return self.prepare_layout_snapshot_with_images_cancelable(
                    viewport,
                    shaper,
                    image_resolver,
                    table_resize,
                    should_cancel,
                );
            }
            LayoutQuery::Source(source) => source,
        };
        if let Some(snapshot) = self.current_layout_snapshot()
            && snapshot.table_resize() == table_resize
            && snapshot.block_for_source(source).is_some()
        {
            let mut changed = false;
            for block in snapshot.blocks() {
                let sizes = self.block_image_sizes(block.metadata().index(), image_resolver)?;
                changed |= block.layout().needs_widget_rebuild(&sizes);
            }
            if !changed {
                return Ok(snapshot);
            }
        }
        self.snapshot().utf16_offset(source)?;
        self.viewport.sync(&self.markdown)?;
        let index =
            self.block_index_for_source(source)
                .ok_or(EditorDocumentError::BlockOutOfBounds {
                    index: 0,
                    blocks: self.markdown.blocks().len(),
                })?;
        // Native synchronous geometry queries target the active paragraph first.
        // Never shape an estimated viewport/overscan before the requested source.
        if should_cancel() {
            return Err(EditorDocumentError::Cancelled);
        }
        self.viewport.set_backend(crate::LayoutBackend::Shaped)?;
        let config = self.viewport_config().layout();
        let sizes = self.block_image_sizes(index, image_resolver)?;
        let mut layout = self
            .block_layout_for_visual_state_with_shaper_and_images(index, config, shaper, &sizes)?;
        if let Some(resize) = table_resize.filter(|resize| resize.block_index() == index) {
            layout.apply_table_resize_with_shaper(resize, shaper)?;
        }
        let height = self.block_box_height(index, layout.height(), config);
        self.viewport.set_block_height(index, height)?;
        let range = crate::ViewportRange::new(index, index + 1).expect("one source block");
        let viewport = self.viewport.snapshot(&self.markdown, range)?;
        let snapshot = Arc::new(LayoutSnapshot::new(
            self,
            viewport,
            vec![Arc::new(layout)],
            table_resize,
        )?);
        self.layout_snapshot = Some(Arc::clone(&snapshot));
        Ok(snapshot)
    }
    pub fn layout_snapshot_for_source<S: ShapingProvider>(
        &mut self,
        source: ByteOffset,
        shaper: &S,
    ) -> Result<Arc<LayoutSnapshot>, EditorDocumentError> {
        self.prepare_query_layout_snapshot_with_images(
            LayoutQuery::Source(source),
            shaper,
            &|_| None,
            None,
        )
    }
}
