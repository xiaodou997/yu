//! Paragraph preparation over immutable source, independent of edit history.

use super::*;

/// Source-bound presentation input and its local paragraph/viewport caches.
/// This type has no text buffer, transaction, undo, or composition commands.
/// A worker can measure it without constructing another editor session.
///
/// ```compile_fail
/// use yu_editor::{EditorDocument, EditorCommand};
/// let mut layout = EditorDocument::new("source")
///     .capture_render_snapshot().into_layout_context();
/// layout.execute(EditorCommand::insert_text("worker cannot edit"));
/// ```
#[derive(Debug)]
pub struct LayoutContext {
    pub(super) table_width_generation: u64,
    pub(super) table_widths: Arc<Vec<super::table_widths::SavedTableWidths>>,
    pub(super) layout_snapshot: Option<Arc<LayoutSnapshot>>,
    pub(super) resource_geometry_version: u64,
    pub(super) render_identity: Arc<()>,
    pub(super) source: TextSnapshot,
    pub(super) markdown: Arc<MarkdownDocument>,
    pub(super) composition: Option<CompositionOverlay>,
    pub(super) selections: Selections,
    pub(super) decorations: DecorationCache,
    pub(super) layouts: LayoutCache,
    pub(super) viewport: ViewportLayout,
    pub(super) search: Option<Arc<SearchState>>,
    pub(super) search_generation: u64,
}

/// Immutable render input. Capturing it neither flattens source storage nor
/// parses Markdown, lays out blocks, or copies edit history. Reconstitution
/// belongs to the preparation worker.
#[derive(Clone, Debug)]
pub struct EditorRenderSnapshot {
    table_width_generation: u64,
    table_widths: Arc<Vec<super::table_widths::SavedTableWidths>>,
    layout_snapshot: Option<Arc<LayoutSnapshot>>,
    resource_geometry_version: u64,
    identity: Arc<()>,
    source: TextSnapshot,
    markdown: Arc<MarkdownDocument>,
    layouts: LayoutCache,
    viewport: ViewportLayout,
    selections: Selections,
    composition: Option<CompositionOverlay>,
    search: Option<Arc<SearchState>>,
    search_generation: u64,
}

/// Owned block heights used by one prepared frame, with no shaper or mutable
/// editor attached. Only the originating document/visual state may adopt it.
#[derive(Clone, Debug)]
pub(super) struct LayoutMeasurements {
    identity: Arc<()>,
    revision: Revision,
    pub(super) viewport: ViewportLayout,
    layouts: LayoutCache,
    selections: Selections,
    composition: Option<CompositionOverlay>,
}

impl EditorRenderSnapshot {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    #[must_use]
    pub fn into_layout_context(self) -> LayoutContext {
        LayoutContext {
            table_width_generation: self.table_width_generation,
            table_widths: self.table_widths,
            layout_snapshot: self.layout_snapshot,
            resource_geometry_version: self.resource_geometry_version,
            render_identity: self.identity,
            source: self.source,
            markdown: self.markdown,
            viewport: self.viewport,
            selections: self.selections,
            composition: self.composition,
            decorations: DecorationCache::default(),
            layouts: self.layouts,
            search: self.search,
            search_generation: self.search_generation,
        }
    }

    pub fn merge_into_layout_context(&self, document: &mut LayoutContext) {
        if self.can_reuse_layout_context(document) {
            document.layouts.adopt_snapshot(self.layouts.clone());
            let mut merged = self.viewport.clone();
            if merged.merge_missing(&document.viewport).is_ok() {
                document.viewport = merged;
                document.layout_snapshot = None;
            }
        }
    }

    /// Returns whether a worker-owned document can keep its parsed Markdown,
    /// decorations and shaped block layout cache for this snapshot. Viewport
    /// scroll/height measurements may differ; the worker updates those while
    /// retaining the expensive source layout state.
    #[must_use]
    pub fn can_reuse_layout_context(&self, document: &LayoutContext) -> bool {
        Arc::ptr_eq(&self.identity, &document.render_identity)
            && self.revision() == document.revision()
            && self.viewport.config() == document.viewport_config()
            && self.selections == document.selections
            && self.composition == document.composition
            && self.search == document.search
            && self.search_generation == document.search_generation
            && self.resource_geometry_version == document.resource_geometry_version
    }
}

impl LayoutContext {
    pub fn semantic_block_decorations(
        &mut self,
        index: usize,
    ) -> Result<BlockDecorations, EditorDocumentError> {
        if !self.markdown.source_mode() {
            return self.block_decorations(index).cloned();
        }
        let block = self.markdown.semantic_blocks().get(index).ok_or(
            EditorDocumentError::BlockOutOfBounds {
                index,
                blocks: self.markdown.semantic_blocks().len(),
            },
        )?;
        Ok(self
            .decorations
            .decorate_semantic(&self.markdown, block, None)?)
    }
    /// Bounded progressive work list; the visible/active paragraphs are measured
    /// first by normal frame preparation. No whole-document shaping task exists.
    pub fn pending_layout_sources(&self, limit: usize) -> Vec<ByteOffset> {
        self.viewport.unmeasured_sources(limit)
    }

    /// Returns the current canonical source revision.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    /// Returns an immutable source snapshot for parser/layout/platform work.
    #[must_use]
    pub fn snapshot(&self) -> TextSnapshot {
        self.source.clone()
    }

    /// Captures only owned source storage and revision-bound visual state.
    #[must_use]
    pub fn capture_render_snapshot(&self) -> EditorRenderSnapshot {
        EditorRenderSnapshot {
            table_width_generation: self.table_width_generation,
            table_widths: Arc::clone(&self.table_widths),
            layout_snapshot: self.layout_snapshot.clone(),
            resource_geometry_version: self.resource_geometry_version,
            identity: Arc::clone(&self.render_identity),
            source: self.snapshot(),
            markdown: Arc::clone(&self.markdown),
            layouts: self.layouts.clone(),
            viewport: self.viewport.clone(),
            selections: self.selections.clone(),
            composition: self.composition.clone(),
            search: self.search.clone(),
            search_generation: self.search_generation,
        }
    }

    /// Copies only the numerical layout state for a publication while keeping
    /// this worker-owned document alive for the next scroll request.
    #[must_use]
    pub(super) fn measurements(&self) -> LayoutMeasurements {
        LayoutMeasurements {
            identity: Arc::clone(&self.render_identity),
            revision: self.source.revision(),
            viewport: self.viewport.clone(),
            layouts: self.layouts.clone(),
            selections: self.selections.clone(),
            composition: self.composition.clone(),
        }
    }

    /// The host also validates request/surface/resource generations. This local
    /// check prevents cross-document, stale-revision and changed-view adoption.
    #[must_use]
    pub(super) fn accepts_measurements(&self, layout: &LayoutMeasurements) -> bool {
        Arc::ptr_eq(&self.render_identity, &layout.identity)
            && self.revision() == layout.revision
            && self.viewport_config() == layout.viewport.config()
            && self.selections == layout.selections
            && self.composition == layout.composition
    }

    /// Adopt revision-bound native paragraph geometry and numerical block heights.
    /// Faces are process-stable; no native layout objects or edit history cross.
    #[must_use]
    pub(super) fn adopt_measurements(&mut self, layout: LayoutMeasurements) -> bool {
        if !self.accepts_measurements(&layout) {
            return false;
        }
        self.layout_snapshot = None;
        let mut merged = layout.viewport;
        if merged.merge_missing(&self.viewport).is_err() {
            return false;
        }
        self.viewport = merged;
        self.layouts.adopt_snapshot(layout.layouts);
        true
    }

    /// Returns the incremental Markdown block document for the current
    /// source revision.
    #[must_use]
    pub fn markdown(&self) -> &MarkdownDocument {
        &self.markdown
    }

    /// Returns the active composition without exposing mutable editor state.
    #[must_use]
    pub fn composition(&self) -> Option<&CompositionOverlay> {
        self.composition.as_ref()
    }

    /// 主选区的端点。
    ///
    /// **多光标之后这个方法的语义是「primary」。** 保留它是刻意的：凡是仍然
    /// 调用它的地方，就是显式选了 primary 降级那条路，`grep` 一遍就数得出来。
    /// 要全部选区用 [`Self::selections`]。
    #[must_use]
    pub fn selection(&self) -> EditorSelection {
        self.selections.primary()
    }

    /// 全部选区，按文档顺序，互不重叠，至少一条。
    #[must_use]
    pub fn selections(&self) -> &Selections {
        &self.selections
    }

    /// 整篇文档的视觉字节流：装饰应用之后长什么样，以及它到源码的映射。
    ///
    /// 这是 v2 里「一份文档一份 `DecorationSet`」那个东西的兑现处。原生
    /// 镜像与 IME 用它把源码坐标换成视觉坐标。
    ///
    /// # Errors
    ///
    /// 解析或装饰产出失败。
    pub fn visual_text(&mut self) -> Result<VisualText, EditorDocumentError> {
        self.visual_text_with_reveal(None)
    }

    /// 带「光标碰到语法就露出来」的那一份。
    ///
    /// 选区变化不推进 Revision，所以这份产出有意绕过规范缓存。
    /// composition 期间不露出——preedit 已经占着这一段的视觉状态了。
    ///
    /// # Errors
    ///
    /// 解析或装饰产出失败。
    pub fn visual_text_for_visual_state(&mut self) -> Result<VisualText, EditorDocumentError> {
        if self.composition.is_some() || self.selections.table_columns().is_some() {
            return self.visual_text_with_reveal(None);
        }
        let active = self.selection_reveal_range();
        self.visual_text_with_reveal(Some(active))
    }

    pub(super) fn visual_text_with_reveal(
        &mut self,
        active: Option<TextRange>,
    ) -> Result<VisualText, EditorDocumentError> {
        let snapshot = self.snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .ok_or(EditorDocumentError::Visual(VisualTextError::OffsetOverflow))?;
        // 先把块序列取成一个 `Vec<Block>`：`document_set` 要
        // `&mut self.decorations`，同时要读 `self.markdown`，而方法调用借的
        // 是整个 `self`。`Block` 是 `Copy` 的小结构，这一份拷贝比克隆整个
        // `MarkdownDocument` 便宜得多——后者每次查询都要复制一遍块存储。
        let blocks: Vec<_> = self.markdown.blocks().iter().collect();
        let set = self
            .decorations
            .document_set(&self.markdown, &blocks, active)?;
        Ok(VisualText::new(&snapshot, range, set)?)
    }

    #[must_use]
    pub fn decoration_cache_stats(&self) -> DecorationCacheStats {
        self.decorations.stats()
    }

    /// 一个块的规范装饰（无光标露出）。
    ///
    /// # Errors
    ///
    /// 块下标越界，或装饰产出失败。
    pub fn block_decorations(
        &mut self,
        index: usize,
    ) -> Result<&BlockDecorations, EditorDocumentError> {
        let block = self.block_at(index)?;
        Ok(self.decorations.get_or_build_block(&self.markdown, block)?)
    }

    /// 换一份查询，立刻在这一版源码上扫出全部匹配。
    ///
    /// 空查询也留下一份状态（0 个匹配），面板要靠它显示「没有结果」；要连
    /// 高亮一起收掉用 [`Self::clear_search`]。
    pub fn set_search_query(&mut self, query: &str) {
        let snapshot = self.snapshot();
        self.search = Some(Arc::new(SearchState::new(&snapshot, query)));
        self.search_generation = self.search_generation.wrapping_add(1);
    }

    /// 收掉搜索：不再有匹配，也不再有高亮。
    pub fn clear_search(&mut self) {
        if self.search.is_none() {
            return;
        }
        self.search = None;
        self.search_generation = self.search_generation.wrapping_add(1);
    }

    /// 当前查询的匹配，没有搜索时是 `None`。
    #[must_use]
    pub fn search(&self) -> Option<&SearchState> {
        self.search.as_deref()
    }

    /// 查询换过几次。
    ///
    /// **帧身份必须带上它。** 改查询不推进 Revision、不改几何、也不改选区，
    /// 少了这一项，在搜索框里打字画面一动不动——不报错、不 panic，正是这个
    /// 项目最危险的失败模式。
    #[must_use]
    pub const fn search_generation(&self) -> u64 {
        self.search_generation
    }

    /// 焦点块那一份：光标碰到的行内语法露出来。
    ///
    /// 有意**不**进缓存——移动光标不推进 Revision，进了缓存别的块也会看见
    /// 一份只对焦点块成立的产出。
    ///
    /// # Errors
    ///
    /// 块下标越界，或装饰产出失败。
    pub fn block_decorations_with_selection_reveal(
        &mut self,
        index: usize,
    ) -> Result<BlockDecorations, EditorDocumentError> {
        let block = self.block_at(index)?;
        let active = self
            .selections
            .table_columns()
            .is_none()
            .then(|| self.selection().ordered_range());
        Ok(self.decorations.decorate(&self.markdown, block, active)?)
    }

    /// 一个块的视觉字节流。
    ///
    /// # Errors
    ///
    /// 块下标越界，或装饰产出失败。
    pub fn block_visual_text(&mut self, index: usize) -> Result<VisualText, EditorDocumentError> {
        let snapshot = self.snapshot();
        let decorations = self.block_decorations(index)?.clone();
        Ok(VisualText::new(
            &snapshot,
            decorations.range(),
            decorations.set().clone(),
        )?)
    }

    pub(super) fn block_at(&self, index: usize) -> Result<yu_markdown::Block, EditorDocumentError> {
        self.markdown
            .blocks()
            .get(index)
            .ok_or(EditorDocumentError::BlockOutOfBounds {
                index,
                blocks: self.markdown.blocks().len(),
            })
    }

    /// 当前光标可能让哪个块露出行内语法。
    ///
    /// 判据是**露出来的那份藏得更少**。composition 期间没有露出：preedit
    /// 已经占着这一段的视觉状态。
    #[must_use]
    pub fn selection_reveal_block_index(&mut self) -> Option<usize> {
        if self.composition.is_some() || self.selections.table_columns().is_some() {
            return None;
        }
        let index = self.block_index_for_source(self.selection().focus())?;
        let block = self.markdown.blocks().get(index)?;
        let active = self.selection().ordered_range();
        let canonical = hidden_bytes(
            self.decorations
                .get_or_build_block(&self.markdown, block)
                .ok()?,
        );
        let revealed = hidden_bytes(
            &self
                .decorations
                .decorate(&self.markdown, block, Some(active))
                .ok()?,
        );
        (revealed < canonical).then_some(index)
    }

    pub(super) fn selection_reveal_range(&mut self) -> TextRange {
        let selection = self.selection().ordered_range();
        let Some(index) = self.selection_reveal_block_index() else {
            return TextRange::empty(self.selection().focus());
        };
        let Some(block) = self.markdown.blocks().get(index) else {
            return TextRange::empty(self.selection().focus());
        };
        if selection.is_empty() {
            return selection;
        }
        TextRange::new(
            selection.start().max(block.range().start()),
            selection.end().min(block.range().end()),
        )
        .unwrap_or_else(|| TextRange::empty(self.selection().focus()))
    }

    /// Returns the parser-owned block containing a canonical source offset.
    ///
    /// The boundary rule matches vertical caret movement: an offset at the
    /// end of a block stays with that block unless a later block contains the
    /// same offset. Native adapters can use this to select a block-local
    /// projection without duplicating Markdown range traversal.
    #[must_use]
    pub fn block_index_for_source(&self, offset: ByteOffset) -> Option<usize> {
        self.markdown.blocks().block_index_for_offset(offset)
    }

    /// Returns the parser block that can host the active composition without
    /// crossing a block boundary.  Composition layout is intentionally
    /// block-local: a marked-text replacement spanning multiple Markdown
    /// blocks has no single block-local index and must use the span-aware
    /// transient viewport path.
    #[must_use]
    pub fn composition_block_index(&self) -> Option<usize> {
        let span = self.composition_block_range()?;
        (span.len() == 1).then_some(span.start)
    }

    /// Returns the half-open parser block-index span touched by the active
    /// composition replacement. The span is source-range based and includes
    /// blank/container blocks crossed by the native selection. A caller can
    /// therefore build one transient projection per affected block without
    /// rescanning Markdown or inventing a second block traversal.
    #[must_use]
    pub fn composition_block_range(&self) -> Option<Range<usize>> {
        let composition = self.composition.as_ref()?;
        self.markdown
            .blocks()
            .block_index_range_for_source_range(composition.replacement_range())
    }

    /// Returns a revision-bound block layout snapshot from the current
    /// projection. The snapshot is owned by a cache keyed by block range,
    /// block kind and layout configuration; source edits remap unaffected
    /// entries and invalidate entries whose projection was touched.
    pub fn block_layout(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<&BlockView, EditorDocumentError> {
        self.block_layout_with_images(index, config, &[])
    }

    /// [`Self::block_layout`] 加上已经解码到位的图片尺寸。
    ///
    /// 没列进来的图片画 placeholder（不变量 D7），所以不关心图片的调用方
    /// 传一张空表即可——那不会把缓存里带尺寸的那一份挤掉，判据见
    /// [`BlockView::needs_widget_rebuild`]。
    pub fn block_layout_with_images(
        &mut self,
        index: usize,
        config: LayoutConfig,
        sizes: &[ImageSize],
    ) -> Result<&BlockView, EditorDocumentError> {
        let block = self.block_at(index)?;
        let snapshot = self.snapshot();
        let decorations = self
            .decorations
            .get_or_build_block(&self.markdown, block)?
            .clone();
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        self.layouts
            .get_or_build_block(
                &self.markdown,
                block,
                config,
                BlockLayoutSource::new(&visual, &decorations, sizes),
            )
            .map_err(EditorDocumentError::Layout)
    }

    /// Returns a revision-bound block layout using a caller-provided shaper.
    ///
    /// Shaped and metrics layouts use separate cache keys. The provider itself
    /// is not stored in the document, so callers can keep platform font state
    /// outside the canonical editor model.
    pub fn block_layout_with_shaper<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<&BlockView, EditorDocumentError> {
        self.block_layout_with_shaper_and_images(index, config, shaper, &[])
    }

    /// 一个块上已经解码到位的图片。
    ///
    /// 装饰先建出来才知道这个块上有哪几张图，所以它与排版是两步。只看这个
    /// 块，不扫整篇文档——viewport 查询不该因为要问图片而变成一次全文扫描。
    pub fn block_image_sizes<F>(
        &mut self,
        index: usize,
        image_resolver: &F,
    ) -> Result<Vec<ImageSize>, EditorDocumentError>
    where
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
    {
        let block = self.block_at(index)?;
        let decorations = self.decorations.get_or_build_block(&self.markdown, block)?;
        Ok(image_sizes(decorations, image_resolver))
    }

    /// [`Self::block_layout_with_shaper`] 加上已经解码到位的图片尺寸。
    pub fn block_layout_with_shaper_and_images<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
        sizes: &[ImageSize],
    ) -> Result<&BlockView, EditorDocumentError> {
        let block = self.block_at(index)?;
        let snapshot = self.snapshot();
        let decorations = self
            .decorations
            .get_or_build_block(&self.markdown, block)?
            .clone();
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        self.layouts
            .get_or_build_block_with_shaper(
                &self.markdown,
                block,
                config,
                BlockLayoutSource::new(&visual, &decorations, sizes),
                shaper,
            )
            .map_err(EditorDocumentError::Layout)
    }

    /// Builds a transient metrics layout for the focus block's currently
    /// revealed inline syntax.
    pub fn block_layout_with_selection_reveal(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<BlockView, EditorDocumentError> {
        let kind = self.block_at(index)?.kind();
        let snapshot = self.snapshot();
        let decorations = self.block_decorations_with_selection_reveal(index)?;
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        BlockView::build(
            kind,
            &visual,
            &decorations,
            config,
            &yu_layout::MonospaceMetrics::new(config.default_advance()),
        )
        .map_err(EditorDocumentError::Layout)
    }

    /// [`Self::block_layout_with_selection_reveal_and_shaper`] 加上图片尺寸。
    pub fn block_layout_with_selection_reveal_and_shaper_and_images<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
        sizes: &[ImageSize],
    ) -> Result<BlockView, EditorDocumentError> {
        let block = self.block_at(index)?;
        let snapshot = self.snapshot();
        let decorations = self.block_decorations_with_selection_reveal(index)?;
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        self.layouts
            .get_or_build_block_with_shaper(
                &self.markdown,
                block,
                config,
                crate::layout::BlockLayoutSource::new(&visual, &decorations, sizes),
                shaper,
            )
            .cloned()
            .map_err(EditorDocumentError::Layout)
    }

    /// Shaping-aware selection reveal layout. The result is intentionally
    /// transient because moving a caret does not change source Revision.
    pub fn block_layout_with_selection_reveal_and_shaper<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<BlockView, EditorDocumentError> {
        self.block_layout_with_selection_reveal_and_shaper_and_images(index, config, shaper, &[])
    }

    /// Returns an owned layout for the current transient visual state.
    /// Composition takes priority, selection reveal applies only to its focus
    /// block, and unaffected blocks clone the canonical cached layout.
    pub fn block_layout_for_visual_state_with_shaper<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<BlockView, EditorDocumentError> {
        self.block_layout_for_visual_state_with_shaper_and_images(index, config, shaper, &[])
    }

    /// [`Self::block_layout_for_visual_state_with_shaper`] 加上图片尺寸。
    pub fn block_layout_for_visual_state_with_shaper_and_images<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
        sizes: &[ImageSize],
    ) -> Result<BlockView, EditorDocumentError> {
        let mut layout = if self
            .composition_block_range()
            .as_ref()
            .is_some_and(|span| span.contains(&index))
        {
            self.block_layout_with_composition_and_shaper_and_images(index, config, shaper, sizes)
        } else if self.selection_reveal_block_index() == Some(index) {
            self.block_layout_with_selection_reveal_and_shaper_and_images(
                index, config, shaper, sizes,
            )
        } else {
            self.block_layout_with_shaper_and_images(index, config, shaper, sizes)
                .cloned()
        }?;
        if let Some(widths) = self.saved_table_widths(index, &layout) {
            layout.set_table_widths_with_shaper(&widths, shaper)?;
        }
        Ok(layout)
    }

    /// Metrics counterpart of
    /// [`Self::block_layout_for_visual_state_with_shaper`].
    pub fn block_layout_for_visual_state(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<BlockView, EditorDocumentError> {
        let mut layout = if self
            .composition_block_range()
            .as_ref()
            .is_some_and(|span| span.contains(&index))
        {
            self.block_layout_with_composition(index, config)
        } else if self.selection_reveal_block_index() == Some(index) {
            self.block_layout_with_selection_reveal(index, config)
        } else {
            self.block_layout(index, config).cloned()
        }?;
        if let Some(widths) = self.saved_table_widths(index, &layout) {
            layout.set_table_widths(&widths)?;
        }
        Ok(layout)
    }

    /// Builds a transient metrics layout with a session-only table column
    /// resize. The normal layout cache remains canonical and the Markdown
    /// source is not changed; callers should discard the returned snapshot
    /// when the visual override ends or the document Revision changes.
    pub fn block_layout_with_table_resize(
        &mut self,
        index: usize,
        config: LayoutConfig,
        commit: TableResizeCommit,
    ) -> Result<BlockView, EditorDocumentError> {
        self.validate_table_resize_commit(index, commit)?;
        let mut layout = self.block_layout(index, config)?.clone();
        layout
            .apply_table_resize(commit)
            .map_err(EditorDocumentError::Layout)?;
        Ok(layout)
    }

    /// Builds a transient shaped layout with a session-only table column
    /// resize. Shaping state stays owned by the caller and the override is
    /// never inserted into the document's layout cache.
    pub fn block_layout_with_table_resize_and_shaper<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
        commit: TableResizeCommit,
    ) -> Result<BlockView, EditorDocumentError> {
        self.validate_table_resize_commit(index, commit)?;
        let mut layout = self
            .block_layout_with_shaper(index, config, shaper)?
            .clone();
        layout
            .apply_table_resize_with_shaper(commit, shaper)
            .map_err(EditorDocumentError::Layout)?;
        Ok(layout)
    }

    pub(super) fn validate_table_resize_commit(
        &self,
        index: usize,
        commit: TableResizeCommit,
    ) -> Result<(), EditorDocumentError> {
        if commit.block_index() != index {
            return Err(EditorDocumentError::Layout(LayoutError::Upstream(
                "table resize commit and block index differ".into(),
            )));
        }
        if commit.revision() != self.revision() {
            return Err(EditorDocumentError::Layout(LayoutError::Upstream(
                "table resize commit and document revisions differ".into(),
            )));
        }
        Ok(())
    }

    /// Builds a transient metrics layout with the active IME preedit
    /// projected over this block. The result is intentionally not inserted in
    /// `LayoutCache`: composition updates do not advance the canonical
    /// Revision, so caching them would make stale preedit geometry observable.
    pub fn block_layout_with_composition(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<BlockView, EditorDocumentError> {
        let kind = self.block_at(index)?.kind();
        let (visual, decorations) = self.block_visual_for_composition(index)?;
        BlockView::build(
            kind,
            &visual,
            &decorations,
            config,
            &yu_layout::MonospaceMetrics::new(config.default_advance()),
        )
        .map_err(EditorDocumentError::Layout)
    }

    /// Builds a transient shaped layout with the active IME preedit projected
    /// over this block. Font/shaping state remains owned by the caller.
    pub fn block_layout_with_composition_and_shaper<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<BlockView, EditorDocumentError> {
        self.block_layout_with_composition_and_shaper_and_images(index, config, shaper, &[])
    }

    /// [`Self::block_layout_with_composition_and_shaper`] 加上图片尺寸。
    pub fn block_layout_with_composition_and_shaper_and_images<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
        sizes: &[ImageSize],
    ) -> Result<BlockView, EditorDocumentError> {
        let block = self.block_at(index)?;
        let (visual, decorations) = self.block_visual_for_composition(index)?;
        self.layouts
            .get_or_build_block_with_shaper(
                &self.markdown,
                block,
                config,
                crate::layout::BlockLayoutSource::new(&visual, &decorations, sizes),
                shaper,
            )
            .cloned()
            .map_err(EditorDocumentError::Layout)
    }

    /// 把 preedit 叠在这个块的规范投影上。
    ///
    /// 一段 marked text 可能横跨几个块。跨块时第一个块吃掉 preedit 的全部
    /// 文字（从替换起点到块末），后面的块只是「这一段没了」——它们的替换
    /// 文本是空的。装饰本身不动：preedit 不是装饰（不变量 H1）。
    pub(super) fn block_visual_for_composition(
        &mut self,
        index: usize,
    ) -> Result<(VisualText, BlockDecorations), EditorDocumentError> {
        let composition = self
            .composition
            .as_ref()
            .ok_or(EditorDocumentError::CompositionNotActive)?;
        let block = self.block_at(index)?;
        let span = self
            .composition_block_range()
            .ok_or(EditorDocumentError::CompositionNotActive)?;
        let (replacement, text, selection) = if span.len() == 1 {
            (
                composition.replacement_range(),
                Arc::from(composition.text()),
                composition.selection_bytes(),
            )
        } else if index == span.start {
            let replacement = TextRange::new(
                composition
                    .replacement_range()
                    .start()
                    .max(block.range().start()),
                block.range().end(),
            )
            .ok_or(EditorDocumentError::CompositionNotActive)?;
            (
                replacement,
                Arc::from(composition.text()),
                composition.selection_bytes(),
            )
        } else if span.contains(&index) {
            let replacement = TextRange::new(
                block.range().start(),
                composition
                    .replacement_range()
                    .end()
                    .min(block.range().end()),
            )
            .ok_or(EditorDocumentError::CompositionNotActive)?;
            (
                replacement,
                Arc::<str>::from(""),
                TextRange::empty(ByteOffset::ZERO),
            )
        } else {
            return Err(EditorDocumentError::CompositionNotActive);
        };
        let snapshot = self.snapshot();
        let decorations = self
            .decorations
            .get_or_build_block(&self.markdown, block)?
            .clone();
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?
            .with_composition(replacement, text, selection)?;
        Ok((visual, decorations))
    }

    #[must_use]
    pub fn layout_cache_stats(&self) -> LayoutCacheStats {
        self.layouts.stats()
    }

    /// Drops all revision-bound layouts and viewport measurements.
    ///
    /// Callers should use this when replacing the font/shaping configuration
    /// behind an existing `LayoutBackend::Shaped` provider. The canonical
    /// source, Markdown document, projections and selection remain intact.
    pub fn clear_layout_state(&mut self) {
        self.layouts.clear();
        self.viewport.clear();
    }

    /// Replaces the pure Rust viewport policy and drops its block estimates.
    pub fn set_viewport_config(&mut self, config: ViewportConfig) -> Result<(), ViewportError> {
        self.viewport.reconfigure(config)?;
        Ok(())
    }

    /// Overscan changes which blocks are requested, not their measured heights.
    pub fn set_viewport_overscan(&mut self, overscan: f32) -> Result<(), ViewportError> {
        self.viewport.set_overscan(overscan)
    }

    #[must_use]
    pub fn viewport_config(&self) -> ViewportConfig {
        self.viewport.config()
    }

    #[must_use]
    pub fn viewport_stats(&self) -> ViewportStats {
        self.viewport.stats()
    }

    /// Measures only the estimated/visible block window and returns block
    /// metadata for a future scene or renderer.
    pub fn visible_blocks(
        &mut self,
        viewport: ViewportSpan,
    ) -> Result<ViewportSnapshot, EditorDocumentError> {
        let mut layout = std::mem::take(&mut self.viewport);
        let result = self.measure_visible_blocks(&mut layout, viewport);
        self.viewport = layout;
        result
    }

    /// Measures the visible block window with a caller-provided shaping
    /// provider. The viewport resets previously measured metrics heights when
    /// switching backend, while estimates for off-screen blocks remain cheap.
    pub fn visible_blocks_with_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
    ) -> Result<ViewportSnapshot, EditorDocumentError> {
        self.visible_blocks_with_shaper_and_image_resolver(viewport, shaper, |_| None)
    }

    /// Measures the visible window with ready image dimensions supplied by a
    /// caller-owned resolver. Only selected blocks are inspected, so the
    /// resolver does not turn a viewport query into a full-document image
    /// scan. Image geometry remains transient to the layout snapshot while
    /// the resulting block height is retained in the viewport HeightIndex.
    pub fn visible_blocks_with_shaper_and_image_resolver<S, F>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: F,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
    {
        let mut layout = std::mem::take(&mut self.viewport);
        let result = self.measure_visible_blocks_with_shaper_and_images(
            &mut layout,
            viewport,
            shaper,
            &image_resolver,
        );
        self.viewport = layout;
        result
    }

    /// Measures the visible window with the active IME overlay projected into
    /// every affected Markdown block. Canonical viewport heights remain the
    /// cache/HeightIndex source when no composition is active; transient
    /// composition heights are applied only to the working viewport state and
    /// are never inserted into `LayoutCache`.
    pub fn visible_blocks_with_composition_and_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
    ) -> Result<ViewportSnapshot, EditorDocumentError> {
        self.visible_blocks_with_composition_and_shaper_and_image_resolver(viewport, shaper, |_| {
            None
        })
    }

    /// Composition-aware variant of
    /// [`Self::visible_blocks_with_shaper_and_image_resolver`]. Ready image
    /// dimensions are applied to transient composition layouts as well, while
    /// the canonical source and layout cache remain untouched.
    pub fn visible_blocks_with_composition_and_shaper_and_image_resolver<S, F>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: F,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
    {
        if self.composition.is_none() {
            return self.visible_blocks_with_shaper_and_image_resolver(
                viewport,
                shaper,
                image_resolver,
            );
        }
        let mut layout = std::mem::take(&mut self.viewport);
        let result = self.measure_visible_blocks_with_composition_and_images(
            &mut layout,
            viewport,
            shaper,
            &image_resolver,
            &mut || false,
        );
        self.viewport = layout;
        result
    }

    /// Measures the viewport using the document's complete transient visual
    /// state. IME composition wins while active; otherwise the focus block is
    /// measured with selection-driven inline syntax reveal.
    pub fn visible_blocks_with_visual_state_and_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
    ) -> Result<ViewportSnapshot, EditorDocumentError> {
        self.visible_blocks_with_visual_state_and_shaper_and_image_resolver(
            viewport,
            shaper,
            |_| None,
        )
    }

    /// Cancellation is checked between paragraphs, including active syntax and
    /// composition projections. No partial geometry is published.
    pub fn visible_blocks_with_visual_state_and_shaper_cancelable<S, C>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
        mut should_cancel: C,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        C: FnMut() -> bool,
    {
        self.visible_blocks_with_visual_state_and_images_cancelable(
            viewport,
            shaper,
            &|_| None,
            &mut should_cancel,
        )
    }

    /// Image-aware variant of the complete visual projection.
    pub fn visible_blocks_with_visual_state_and_shaper_and_image_resolver<S, F>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: F,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
    {
        self.visible_blocks_with_visual_state_and_images_cancelable(
            viewport,
            shaper,
            &image_resolver,
            &mut || false,
        )
    }

    pub(super) fn visible_blocks_with_visual_state_and_images_cancelable<S, F, C>(
        &mut self,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: &F,
        should_cancel: &mut C,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
        C: FnMut() -> bool,
    {
        if should_cancel() {
            return Err(EditorDocumentError::Cancelled);
        }
        let mut layout = std::mem::take(&mut self.viewport);
        let result = if self.composition.is_some() {
            self.measure_visible_blocks_with_composition_and_images(
                &mut layout,
                viewport,
                shaper,
                image_resolver,
                should_cancel,
            )
        } else if self.selection_reveal_block_index().is_some() {
            self.measure_visible_blocks_with_selection_reveal_and_images(
                &mut layout,
                viewport,
                shaper,
                image_resolver,
                should_cancel,
            )
        } else {
            self.measure_visible_blocks_with_shaper_and_images_cancelable(
                &mut layout,
                viewport,
                shaper,
                image_resolver,
                should_cancel,
            )
        };
        self.viewport = layout;
        result
    }

    /// Resolves the current focus caret into a revision-bound scroll request.
    ///
    /// The returned target is document-space `scroll_y`; the platform only
    /// needs to apply it to its native viewport when `needs_scroll()` is true.
    /// Unmeasured blocks keep their configured estimate, while the caret's
    /// block is measured before its document-space y is calculated.
    pub fn caret_scroll_request(
        &mut self,
        viewport: ViewportSpan,
        margin: f32,
    ) -> Result<CaretScrollRequest, EditorDocumentError> {
        let mut layout = std::mem::take(&mut self.viewport);
        let result = self.measure_caret_scroll_request_metrics(&mut layout, viewport, margin);
        self.viewport = layout;
        result
    }

    /// Shaping-aware variant of [`Self::caret_scroll_request`]. Its measured
    /// block height uses the same provider as the caller's visible viewport.
    pub fn caret_scroll_request_with_shaper<S: ShapingProvider>(
        &mut self,
        viewport: ViewportSpan,
        margin: f32,
        shaper: &S,
    ) -> Result<CaretScrollRequest, EditorDocumentError> {
        viewport.validate()?;
        validate_caret_margin(margin)?;
        let focus = self
            .composition()
            .map_or(self.selection().focus(), |overlay| {
                overlay.replacement_range().start()
            });
        if self.block_index_for_source(focus).is_none() {
            return Ok(self.empty_caret_scroll_request(viewport, margin));
        }
        let snapshot = self.layout_snapshot_for_source(focus, shaper)?;
        let block =
            snapshot
                .block_for_source(focus)
                .ok_or(EditorDocumentError::BlockOutOfBounds {
                    index: 0,
                    blocks: self.markdown.blocks().len(),
                })?;
        let layout = block.layout();
        let caret = if let Some(selection) = layout.visual().composition_selection_visual() {
            layout.caret_for_visual(selection.end(), Bias::After)?
        } else {
            layout.caret_for_source(focus, self.selection_projection_bias())?
        };
        snapshot.caret_scroll_request(block, caret, viewport, margin)
    }

    /// 把块级盒模型折进这一块的高度贡献：上内边距 + 内容高 + 下内边距 + 折给
    /// 下一块的间距。
    ///
    /// 间距按 `collapsed_block_gap` 取 `max(after(本块), before(下一块))`，
    /// **整条缝折在本块（缝的上块）的贡献里**，而不是上下各一半：折一半需要
    /// 每个消费方都按块给内容加一个原点偏移，而这条路的约定是「高度索引的
    /// 结构不动、调用方零感知」——折在上块，下一块的内容正好顶到自己的盒顶，
    /// 前缀和自动把缝算进每一块的原点，光标、滚动、AX、绘制的数学全部照旧
    /// 一致（Revision 也不推进）。
    ///
    /// 代码块的上、下内边距分别取自当前主题和缩放，绘制与几何查询
    /// 通过 content_origin_y 使用同一上边距；背景和高度索引加同一下边距。
    ///
    /// 顶层首标题保留自己的段前距，页面顶/底留白仍由视口负责。
    /// 相邻块的间距取较大值；末块保留自身及结束容器的段后距。
    /// The first top-level heading retains its own margin inside the reading
    /// column. This is document geometry, not a global viewport correction.
    pub(super) fn block_content_origin(&self, index: usize, config: LayoutConfig) -> f32 {
        if self.markdown.source_mode() {
            return 0.0;
        }
        let Some(block) = self.markdown.blocks().get(index) else {
            return 0.0;
        };
        let base = content_origin_y(block.kind(), config);
        if self
            .markdown
            .blocks()
            .iter()
            .position(|b| b.kind() != BlockKind::BlankLine)
            != Some(index)
        {
            return base;
        }
        let tree = self.markdown.presentation();
        let quote_margin = tree
            .path_for_range(block.range())
            .iter()
            .filter(|id| tree.nodes()[**id].kind == yu_markdown::PresentationKind::Quote)
            .map(|id| crate::layout_tokens::quote_margin(tree, *id, config.theme(), false))
            .reduce(f32::max);
        if let Some(margin) = quote_margin {
            return base + margin * config.line_height() / config.theme().spec().body_size;
        }
        let is_table = yu_markdown::table_for_block(&self.markdown, block).is_some();
        if !matches!(block.kind(), BlockKind::Heading { .. }) && !is_table {
            return base;
        }
        let direct = tree
            .leaf_for_range(block.range())
            .and_then(|id| tree.nodes()[id].parent)
            .is_some_and(|parent| {
                tree.nodes()[parent].kind == yu_markdown::PresentationKind::Document
            });
        if !direct {
            return base;
        }
        let margin = if is_table {
            yu_core::ThemeSpec::TABLE_MARGIN_EM
        } else {
            crate::layout_tokens::block_spacing(block.kind(), config.theme()).before
                * config.theme().spec().body_line_ratio
        };
        base + margin * config.line_height()
    }

    pub(super) fn block_box_height(
        &self,
        index: usize,
        content_height: f32,
        config: LayoutConfig,
    ) -> f32 {
        let blocks = self.markdown.blocks();
        let Some(kind) = blocks.get(index).map(|block| block.kind()) else {
            return content_height;
        };
        let composition_span = self.composition_block_range();
        if composition_span
            .as_ref()
            .is_some_and(|span| index > span.start && span.contains(&index))
        {
            return 0.0;
        }
        let active_blank = self
            .block_index_for_offset(
                self.composition
                    .as_ref()
                    .map_or(self.selection().focus(), |composition| {
                        composition.replacement_range().start()
                    }),
            )
            // A selection ending on a separator is not an editing caret.
            // Giving it a line box would move following text during a drag.
            .filter(|_| self.composition.is_some() || self.selection().is_empty())
            .filter(|index| {
                blocks
                    .get(*index)
                    .is_some_and(|block| block.kind() == BlockKind::BlankLine)
            });
        let is_empty_paragraph = |index: usize| {
            blocks
                .get(index)
                .and_then(|block| self.markdown.presentation().leaf_for_range(block.range()))
                .is_some_and(|leaf| {
                    self.markdown.presentation().nodes()[leaf].kind
                        == yu_markdown::PresentationKind::EmptyParagraph
                })
        };
        let semantic_empty = kind == BlockKind::BlankLine && is_empty_paragraph(index);
        if kind == BlockKind::BlankLine && !semantic_empty {
            return if active_blank == Some(index) {
                content_height.max(crate::layout_tokens::resolved_line_height(
                    config,
                    config.theme().spec().body_line_ratio,
                ))
            } else {
                0.0
            };
        }
        let following = composition_span
            .filter(|span| span.start == index)
            .map_or(index + 1, |span| span.end);
        let next = (following..blocks.len())
            .filter(|next| {
                blocks.get(*next).is_some_and(|block| {
                    block.kind() != BlockKind::BlankLine
                        || active_blank == Some(*next)
                        || is_empty_paragraph(*next)
                })
            })
            .find_map(|next| blocks.get(next));
        let current = blocks.get(index).expect("validated block");
        let gap_below = self.structural_block_gap(current, next, config);
        let origin = self.block_content_origin(index, config);
        let content_height = if semantic_empty {
            crate::layout_tokens::resolved_line_height(
                config,
                config.theme().spec().body_line_ratio,
            )
        } else {
            content_height
        };
        // 间距表以「一行正文高」（line_height × 正文行高倍率）为单位，乘回
        // 实际行高得到这份配置下的间距。
        content_height
            + origin
            + content_bottom_inset(kind, config)
            + gap_below * config.theme().spec().body_line_ratio * config.line_height()
    }

    /// Collapse margins along the two branches meeting at the common
    /// container. Tightness belongs to direct item paragraphs, not every
    /// descendant or every pair of adjacent list-shaped blocks.
    fn structural_block_gap(
        &self,
        upper: yu_markdown::Block,
        lower: Option<yu_markdown::Block>,
        config: LayoutConfig,
    ) -> f32 {
        use yu_markdown::PresentationKind;
        if self.markdown.source_mode() {
            return 0.0;
        }
        let tree = self.markdown.presentation();
        let upper_path = tree.path_for_range(upper.range());
        let lower_path = lower
            .map(|block| tree.path_for_range(block.range()))
            .unwrap_or_default();
        if upper_path.is_empty() || (lower.is_some() && lower_path.is_empty()) {
            let upper_kind = if upper_path
                .last()
                .is_some_and(|id| tree.nodes()[*id].kind == PresentationKind::EmptyParagraph)
            {
                BlockKind::Paragraph
            } else {
                upper.kind()
            };
            return lower.map_or_else(
                || crate::layout_tokens::block_spacing(upper_kind, config.theme()).after,
                |lower| collapsed_block_gap(upper_kind, lower.kind(), config.theme()),
            );
        }
        let common = upper_path
            .iter()
            .zip(&lower_path)
            .take_while(|(a, b)| a == b)
            .count();
        let spec = config.theme().spec();
        let margin = |path: &[usize], block: yu_markdown::Block, after: bool| {
            let is_table = yu_markdown::table_for_block(&self.markdown, block).is_some();
            let leaf = *path.last().expect("nonempty path");
            let node = &tree.nodes()[leaf];
            let kind = if node.kind == PresentationKind::EmptyParagraph {
                BlockKind::Paragraph
            } else {
                block.kind()
            };
            let spacing = crate::layout_tokens::block_spacing(kind, config.theme());
            let item = node
                .parent
                .filter(|id| tree.nodes()[*id].kind == PresentationKind::ListItem);
            let direct_item_paragraph =
                node.kind == PresentationKind::Paragraph && item.is_some() && !is_table;
            let mut margin = if is_table {
                let last_in_item =
                    after && item.is_some_and(|id| tree.nodes()[id].children.last() == Some(&leaf));
                (if last_in_item {
                    yu_core::ThemeSpec::LIST_PARAGRAPH_MARGIN
                } else {
                    yu_core::ThemeSpec::TABLE_MARGIN_EM
                }) / spec.body_line_ratio
            } else if direct_item_paragraph {
                let first_child =
                    item.is_some_and(|id| tree.nodes()[id].children.first() == Some(&leaf));
                if !after && first_child {
                    0.0
                } else {
                    yu_core::ThemeSpec::LIST_PARAGRAPH_MARGIN / spec.body_line_ratio
                }
            } else if after {
                spacing.after
            } else {
                spacing.before
            };
            if let Some(parent) = node.parent.map(|id| &tree.nodes()[id])
                && parent.kind == PresentationKind::Quote
                && if after {
                    parent.children.last() == Some(&leaf)
                } else {
                    parent.children.first() == Some(&leaf)
                }
            {
                margin = 0.0;
            }
            for id in &path[common.min(path.len() - 1)..path.len() - 1] {
                let node = &tree.nodes()[*id];
                if node.kind == PresentationKind::Quote {
                    let points =
                        crate::layout_tokens::quote_margin(tree, *id, config.theme(), after);
                    margin = margin.max(points / spec.body_size / spec.body_line_ratio);
                    continue;
                }
                let has_margin = match node.kind {
                    PresentationKind::List { .. } => !node.parent.is_some_and(|parent| {
                        tree.nodes()[parent].kind == PresentationKind::ListItem
                    }),
                    _ => false,
                };
                if has_margin {
                    margin = margin.max(
                        if after {
                            spec.paragraph_margin
                        } else {
                            spec.paragraph_margin_before
                        } / spec.body_line_ratio,
                    );
                }
            }
            margin
        };
        let after = margin(&upper_path, upper, true);
        lower.map_or(after, |lower| after.max(margin(&lower_path, lower, false)))
    }

    pub(super) fn measure_visible_blocks(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportSpan,
    ) -> Result<ViewportSnapshot, EditorDocumentError> {
        layout
            .set_backend(LayoutBackend::Metrics)
            .map_err(EditorDocumentError::Viewport)?;
        let mut range = layout
            .visible_range(&self.markdown, viewport)
            .map_err(EditorDocumentError::Viewport)?;
        let config = layout.config().layout();
        for _ in 0..8 {
            let mut changed = false;
            for index in range.start()..range.end() {
                let content_height = if self.has_saved_table_widths(index) {
                    self.block_layout_for_visual_state(index, config)?.height()
                } else {
                    config.line_height() * self.block_layout(index, config)?.lines().len() as f32
                };
                let height = self.block_box_height(index, content_height, config);
                changed |= layout
                    .set_block_height(index, height)
                    .map_err(EditorDocumentError::Viewport)?;
            }
            let next = layout
                .visible_range(&self.markdown, viewport)
                .map_err(EditorDocumentError::Viewport)?;
            if next == range || !changed {
                break;
            }
            range = next;
        }
        layout
            .snapshot(&self.markdown, range)
            .map_err(EditorDocumentError::Viewport)
    }

    pub(super) fn measure_visible_blocks_with_shaper_and_images<S, F>(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: &F,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
    {
        self.measure_visible_blocks_with_shaper_and_images_cancelable(
            layout,
            viewport,
            shaper,
            image_resolver,
            &mut || false,
        )
    }

    pub(super) fn measure_visible_blocks_with_shaper_and_images_cancelable<S, F, C>(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: &F,
        should_cancel: &mut C,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
        C: FnMut() -> bool,
    {
        layout
            .set_backend(LayoutBackend::Shaped)
            .map_err(EditorDocumentError::Viewport)?;
        let mut range = layout
            .visible_range(&self.markdown, viewport)
            .map_err(EditorDocumentError::Viewport)?;
        let config = layout.config().layout();
        for _ in 0..8 {
            let mut changed = false;
            for index in range.start()..range.end() {
                if should_cancel() {
                    return Err(EditorDocumentError::Cancelled);
                }
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let content_height =
                    self.measured_table_aware_height(index, config, shaper, &sizes)?;
                let height = self.block_box_height(index, content_height, config);
                changed |= layout
                    .set_block_height(index, height)
                    .map_err(EditorDocumentError::Viewport)?;
            }
            let next = layout
                .visible_range(&self.markdown, viewport)
                .map_err(EditorDocumentError::Viewport)?;
            if next == range || !changed {
                break;
            }
            range = next;
        }
        layout
            .snapshot(&self.markdown, range)
            .map_err(EditorDocumentError::Viewport)
    }

    pub(super) fn measure_visible_blocks_with_selection_reveal_and_images<S, F, C>(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: &F,
        should_cancel: &mut C,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
        C: FnMut() -> bool,
    {
        layout
            .set_backend(LayoutBackend::Shaped)
            .map_err(EditorDocumentError::Viewport)?;
        let mut range = layout
            .visible_range(&self.markdown, viewport)
            .map_err(EditorDocumentError::Viewport)?;
        let config = layout.config().layout();
        let reveal_block = self.selection_reveal_block_index();
        for _ in 0..8 {
            let mut changed = false;

            // The focus block can sit above the visible window. Its revealed
            // syntax may rewrap, so measure it first to keep every later block
            // y-coordinate consistent with the retained scene.
            if let Some(index) = reveal_block {
                if should_cancel() {
                    return Err(EditorDocumentError::Cancelled);
                }
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let block_layout = self.block_layout_for_visual_state_with_shaper_and_images(
                    index, config, shaper, &sizes,
                )?;
                let height = self.block_box_height(index, block_layout.height(), config);
                changed |= layout
                    .set_block_height(index, height)
                    .map_err(EditorDocumentError::Viewport)?;
            }

            for index in range.start()..range.end() {
                if reveal_block == Some(index) {
                    continue;
                }
                if should_cancel() {
                    return Err(EditorDocumentError::Cancelled);
                }
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let content_height =
                    self.measured_table_aware_height(index, config, shaper, &sizes)?;
                let height = self.block_box_height(index, content_height, config);
                changed |= layout
                    .set_block_height(index, height)
                    .map_err(EditorDocumentError::Viewport)?;
            }
            let next = layout
                .visible_range(&self.markdown, viewport)
                .map_err(EditorDocumentError::Viewport)?;
            if next == range || !changed {
                break;
            }
            range = next;
        }
        layout
            .snapshot(&self.markdown, range)
            .map_err(EditorDocumentError::Viewport)
    }

    pub(super) fn measure_visible_blocks_with_composition_and_images<S, F, C>(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportSpan,
        shaper: &S,
        image_resolver: &F,
        should_cancel: &mut C,
    ) -> Result<ViewportSnapshot, EditorDocumentError>
    where
        S: ShapingProvider,
        F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
        C: FnMut() -> bool,
    {
        layout
            .set_backend(LayoutBackend::Shaped)
            .map_err(EditorDocumentError::Viewport)?;
        let mut range = layout
            .visible_range(&self.markdown, viewport)
            .map_err(EditorDocumentError::Viewport)?;
        let config = layout.config().layout();
        let composition_span = self.composition_block_range();
        for _ in 0..8 {
            let mut changed = false;

            // A transient block may be above the current scroll window. Its
            // height still contributes to every later document-space y, so
            // measure the affected span before measuring the visible window.
            if let Some(span) = composition_span.as_ref() {
                for index in span.clone() {
                    if should_cancel() {
                        return Err(EditorDocumentError::Cancelled);
                    }
                    let sizes = self.block_image_sizes(index, image_resolver)?;
                    let content_height = self
                        .block_layout_for_visual_state_with_shaper_and_images(
                            index, config, shaper, &sizes,
                        )?
                        .height();
                    let height = self.block_box_height(index, content_height, config);
                    changed |= layout
                        .set_block_height(index, height)
                        .map_err(EditorDocumentError::Viewport)?;
                }
            }

            for index in range.start()..range.end() {
                if composition_span
                    .as_ref()
                    .is_some_and(|span| span.contains(&index))
                {
                    continue;
                }
                if should_cancel() {
                    return Err(EditorDocumentError::Cancelled);
                }
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let content_height =
                    self.measured_table_aware_height(index, config, shaper, &sizes)?;
                let height = self.block_box_height(index, content_height, config);
                changed |= layout
                    .set_block_height(index, height)
                    .map_err(EditorDocumentError::Viewport)?;
            }
            let next = layout
                .visible_range(&self.markdown, viewport)
                .map_err(EditorDocumentError::Viewport)?;
            if next == range || !changed {
                break;
            }
            range = next;
        }
        layout
            .snapshot(&self.markdown, range)
            .map_err(EditorDocumentError::Viewport)
    }

    pub(super) fn measure_caret_scroll_request_metrics(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportSpan,
        margin: f32,
    ) -> Result<CaretScrollRequest, EditorDocumentError> {
        viewport.validate().map_err(EditorDocumentError::Viewport)?;
        validate_caret_margin(margin).map_err(EditorDocumentError::Viewport)?;
        layout
            .set_backend(LayoutBackend::Metrics)
            .map_err(EditorDocumentError::Viewport)?;
        layout
            .sync(&self.markdown)
            .map_err(EditorDocumentError::Viewport)?;
        let focus = self.selection().focus();
        let Some(block_index) = self.block_index_for_offset(focus) else {
            return Ok(self.empty_caret_scroll_request(viewport, margin));
        };
        let config = layout.config().layout();
        let projection_bias = self.selection_projection_bias();
        let (caret_x, caret_y, line_count) = {
            let block_layout = self.block_layout_for_visual_state(block_index, config)?;
            let caret = block_layout.caret_for_source(focus, projection_bias)?;
            (
                caret.point().x(),
                caret.point().y(),
                block_layout.lines().len(),
            )
        };
        let height = self.block_box_height(
            block_index,
            config.line_height() * line_count.max(1) as f32,
            config,
        );
        layout
            .set_block_height(block_index, height)
            .map_err(EditorDocumentError::Viewport)?;
        self.finish_caret_scroll_request(
            layout,
            viewport,
            margin,
            CaretLayoutPosition {
                source: focus,
                block: block_index,
                x: caret_x,
                y: caret_y,
                height: config.line_height(),
            },
        )
    }

    pub(super) fn finish_caret_scroll_request(
        &self,
        layout: &ViewportLayout,
        viewport: ViewportSpan,
        margin: f32,
        position: CaretLayoutPosition,
    ) -> Result<CaretScrollRequest, EditorDocumentError> {
        let effective_margin = margin.min(viewport.height() / 2.0);
        // `position.y` 是内容局部坐标；代码块的内容在盒里从上内边距起排
        // （`block_box_height` 折块高时把那 5pt 加在了内容上方），换算成文档
        // 坐标要补回同一个起点，否则代码块里的光标/滚动目标整体上移 5pt。
        let origin = self.block_content_origin(position.block, layout.config().layout());
        let document_y = layout.height_index().prefix_height(position.block) + origin + position.y;
        let caret_bottom = document_y + position.height;
        let visible_top = viewport.scroll_y() + effective_margin;
        let visible_bottom = viewport.scroll_y() + viewport.height() - effective_margin;
        let mut target = viewport.scroll_y();
        if document_y < visible_top {
            target = document_y - effective_margin;
        } else if caret_bottom > visible_bottom {
            target = caret_bottom + effective_margin - viewport.height();
        }
        let max_scroll = (layout.height_index().total_height() - viewport.height()).max(0.0);
        target = target.clamp(0.0, max_scroll);
        let needs_scroll = (target - viewport.scroll_y()).abs() > f32::EPSILON;
        let caret = ViewportCaret::new(
            position.source,
            position.block,
            position.x,
            document_y,
            0.0,
            position.height,
        )?;
        Ok(CaretScrollRequest::new(
            self.revision(),
            caret,
            viewport.scroll_y(),
            if needs_scroll {
                target
            } else {
                viewport.scroll_y()
            },
            effective_margin,
            needs_scroll,
        ))
    }

    pub(super) fn empty_caret_scroll_request(
        &self,
        viewport: ViewportSpan,
        margin: f32,
    ) -> CaretScrollRequest {
        CaretScrollRequest::new(
            self.revision(),
            ViewportCaret::new(ByteOffset::ZERO, 0, 0.0, 0.0, 0.0, 0.0)
                .expect("an all-zero caret box is always valid"),
            viewport.scroll_y(),
            viewport.scroll_y(),
            margin.min(viewport.height() / 2.0),
            false,
        )
    }

    pub(super) fn block_index_for_offset(&self, offset: ByteOffset) -> Option<usize> {
        self.markdown.blocks().block_index_for_offset(offset)
    }

    pub(super) fn selection_projection_bias(&self) -> Bias {
        match self.selection().affinity() {
            crate::CaretAffinity::Upstream => Bias::Before,
            crate::CaretAffinity::Downstream => Bias::After,
        }
    }
}
