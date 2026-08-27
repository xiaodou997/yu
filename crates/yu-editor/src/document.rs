use std::error::Error;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use yu_core::{ByteOffset, LineIndex, Revision, ShapingProvider, TextRange, Utf16Range};
use yu_layout::{ImageIntrinsicSize, LayoutConfig, LayoutError};

use crate::blockview::BlockView;
use crate::table::TableResizeCommit;
use yu_markdown::{BlockKind, IncrementalParseError, MarkdownDocument, TaskState};
use yu_state::{EditorHistory, HistoryEntry, HistoryGroup, HistoryStats};
use yu_text::{
    AppliedTransaction, EditError, TextBuffer, TextPositionError, TextSnapshot, Transaction,
};

use crate::widget::ImageSize;
use crate::{
    BlockLayoutSource, CaretScrollRequest, CommandResult, CompositionError, CompositionOverlay,
    DecorationCache, DecorationCacheStats, DecorationError, EditorCommand, EditorSelection,
    KeyEvent, KeyRouteResult, LayoutBackend, LayoutCache, LayoutCacheStats, LayoutPoint,
    SelectionError, SourceChange, ViewportCaret, ViewportConfig, ViewportError, ViewportLayout,
    ViewportSnapshot, ViewportSpan, ViewportStats, VisualText, VisualTextError,
    command::{
        next_grapheme_boundary, next_word_boundary, previous_grapheme_boundary,
        previous_word_boundary,
    },
    decorations::hidden_bytes,
    keymap::command_for_key,
    list::ListLinePrefix,
};
use yu_decoration::Bias;
use yu_markdown::{BlockDecorations, BlockWidget, ImageSpan};

/// The canonical source and transient composition state owned by one editor.
///
/// `TextBuffer` remains the only persistent source of truth. The optional
/// `CompositionOverlay` is deliberately kept beside it so platform adapters
/// cannot accidentally commit preedit text through a separate shadow buffer.
#[derive(Debug)]
pub struct EditorDocument {
    buffer: TextBuffer,
    markdown: MarkdownDocument,
    composition: Option<CompositionOverlay>,
    selection: EditorSelection,
    preferred_x: Option<PreferredCaretX>,
    last_source_change: Option<SourceChange>,
    history: EditorHistory,
    decorations: DecorationCache,
    layouts: LayoutCache,
    viewport: ViewportLayout,
}

impl EditorDocument {
    /// Creates a document at the initial revision.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let selection = EditorSelection::cursor(
            &snapshot,
            yu_core::ByteOffset::ZERO,
            crate::CaretAffinity::Downstream,
        )
        .expect("offset zero is always a valid caret");
        Self {
            buffer,
            markdown,
            composition: None,
            selection,
            preferred_x: None,
            last_source_change: None,
            history: EditorHistory::default(),
            decorations: DecorationCache::default(),
            layouts: LayoutCache::default(),
            viewport: ViewportLayout::default(),
        }
    }

    /// Returns the current canonical source revision.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.buffer.revision()
    }

    /// Returns an immutable source snapshot for parser/layout/platform work.
    #[must_use]
    pub fn snapshot(&self) -> TextSnapshot {
        self.buffer.snapshot()
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

    /// Returns the current source selection and caret endpoints.
    #[must_use]
    pub const fn selection(&self) -> EditorSelection {
        self.selection
    }

    /// Returns the bounded undo/redo depth for the current editor session.
    #[must_use]
    pub fn history_stats(&self) -> HistoryStats {
        self.history.stats()
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
        if self.composition.is_some() {
            return self.visual_text_with_reveal(None);
        }
        let active = self.selection_reveal_range();
        self.visual_text_with_reveal(Some(active))
    }

    fn visual_text_with_reveal(
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
        let set = self.decorations.document_set(&snapshot, &blocks, active)?;
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
        let snapshot = self.snapshot();
        Ok(self.decorations.get_or_build_block(&snapshot, block)?)
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
        let snapshot = self.snapshot();
        let active = self.selection.ordered_range();
        Ok(self.decorations.decorate(&snapshot, block, Some(active))?)
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

    fn block_at(&self, index: usize) -> Result<yu_markdown::Block, EditorDocumentError> {
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
        if self.composition.is_some() {
            return None;
        }
        let index = self.block_index_for_source(self.selection.focus())?;
        let block = self.markdown.blocks().get(index)?;
        let snapshot = self.snapshot();
        let active = self.selection.ordered_range();
        let canonical = hidden_bytes(self.decorations.get_or_build_block(&snapshot, block).ok()?);
        let revealed = hidden_bytes(
            &self
                .decorations
                .decorate(&snapshot, block, Some(active))
                .ok()?,
        );
        (revealed < canonical).then_some(index)
    }

    fn selection_reveal_range(&mut self) -> TextRange {
        let selection = self.selection.ordered_range();
        let Some(index) = self.selection_reveal_block_index() else {
            return TextRange::empty(self.selection.focus());
        };
        let Some(block) = self.markdown.blocks().get(index) else {
            return TextRange::empty(self.selection.focus());
        };
        if selection.is_empty() {
            return selection;
        }
        TextRange::new(
            selection.start().max(block.range().start()),
            selection.end().min(block.range().end()),
        )
        .unwrap_or_else(|| TextRange::empty(self.selection.focus()))
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
            .get_or_build_block(&snapshot, block)?
            .clone();
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        self.layouts
            .get_or_build_block(
                &snapshot,
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
        let snapshot = self.snapshot();
        let decorations = self.decorations.get_or_build_block(&snapshot, block)?;
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
            .get_or_build_block(&snapshot, block)?
            .clone();
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        self.layouts
            .get_or_build_block_with_shaper(
                &snapshot,
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
        let snapshot = self.snapshot();
        let decorations = self.block_decorations_with_selection_reveal(index)?;
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        BlockView::build(
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
        let snapshot = self.snapshot();
        let decorations = self.block_decorations_with_selection_reveal(index)?;
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())?;
        BlockView::build_shaped_with_images(&visual, &decorations, config, shaper, sizes)
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
        if self
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
        }
    }

    /// Metrics counterpart of
    /// [`Self::block_layout_for_visual_state_with_shaper`].
    pub fn block_layout_for_visual_state(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<BlockView, EditorDocumentError> {
        if self
            .composition_block_range()
            .as_ref()
            .is_some_and(|span| span.contains(&index))
        {
            self.block_layout_with_composition(index, config)
        } else if self.selection_reveal_block_index() == Some(index) {
            self.block_layout_with_selection_reveal(index, config)
        } else {
            self.block_layout(index, config).cloned()
        }
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
            .apply_table_resize(commit)
            .map_err(EditorDocumentError::Layout)?;
        Ok(layout)
    }

    fn validate_table_resize_commit(
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
        let (visual, decorations) = self.block_visual_for_composition(index)?;
        BlockView::build(
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
        let (visual, decorations) = self.block_visual_for_composition(index)?;
        BlockView::build_shaped_with_images(&visual, &decorations, config, shaper, sizes)
            .map_err(EditorDocumentError::Layout)
    }

    /// 把 preedit 叠在这个块的规范投影上。
    ///
    /// 一段 marked text 可能横跨几个块。跨块时第一个块吃掉 preedit 的全部
    /// 文字（从替换起点到块末），后面的块只是「这一段没了」——它们的替换
    /// 文本是空的。装饰本身不动：preedit 不是装饰（不变量 H1）。
    fn block_visual_for_composition(
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
            .get_or_build_block(&snapshot, block)?
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
        self.viewport = ViewportLayout::new(config)?;
        Ok(())
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

    /// Image-aware variant of
    /// [`Self::visible_blocks_with_visual_state_and_shaper`].
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
        if self.composition.is_some() {
            return self.visible_blocks_with_composition_and_shaper_and_image_resolver(
                viewport,
                shaper,
                image_resolver,
            );
        }
        if self.selection_reveal_block_index().is_none() {
            return self.visible_blocks_with_shaper_and_image_resolver(
                viewport,
                shaper,
                image_resolver,
            );
        }
        let mut layout = std::mem::take(&mut self.viewport);
        let result = self.measure_visible_blocks_with_selection_reveal_and_images(
            &mut layout,
            viewport,
            shaper,
            &image_resolver,
        );
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
        let mut layout = std::mem::take(&mut self.viewport);
        let result =
            self.measure_caret_scroll_request_shaped(&mut layout, viewport, margin, shaper);
        self.viewport = layout;
        result
    }

    fn measure_visible_blocks(
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
                let line_count = self.block_layout(index, config)?.lines().len();
                let height = config.line_height() * (line_count as f32);
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

    fn measure_visible_blocks_with_shaper_and_images<S, F>(
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
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let height = self
                    .block_layout_with_shaper_and_images(index, config, shaper, &sizes)?
                    .height();
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

    fn measure_visible_blocks_with_selection_reveal_and_images<S, F>(
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
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let block_layout = self.block_layout_with_selection_reveal_and_shaper_and_images(
                    index, config, shaper, &sizes,
                )?;
                changed |= layout
                    .set_block_height(index, block_layout.height())
                    .map_err(EditorDocumentError::Viewport)?;
            }

            for index in range.start()..range.end() {
                if reveal_block == Some(index) {
                    continue;
                }
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let height = self
                    .block_layout_with_shaper_and_images(index, config, shaper, &sizes)?
                    .height();
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

    fn measure_visible_blocks_with_composition_and_images<S, F>(
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
                    let sizes = self.block_image_sizes(index, image_resolver)?;
                    let height = self
                        .block_layout_with_composition_and_shaper_and_images(
                            index, config, shaper, &sizes,
                        )?
                        .height();
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
                let sizes = self.block_image_sizes(index, image_resolver)?;
                let height = self
                    .block_layout_with_shaper_and_images(index, config, shaper, &sizes)?
                    .height();
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

    fn measure_caret_scroll_request_metrics(
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
        let focus = self.selection.focus();
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
        let height = config.line_height() * line_count.max(1) as f32;
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

    fn measure_caret_scroll_request_shaped<S: ShapingProvider>(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportSpan,
        margin: f32,
        shaper: &S,
    ) -> Result<CaretScrollRequest, EditorDocumentError> {
        viewport.validate().map_err(EditorDocumentError::Viewport)?;
        validate_caret_margin(margin).map_err(EditorDocumentError::Viewport)?;
        layout
            .set_backend(LayoutBackend::Shaped)
            .map_err(EditorDocumentError::Viewport)?;
        layout
            .sync(&self.markdown)
            .map_err(EditorDocumentError::Viewport)?;
        let focus = self.selection.focus();
        let Some(block_index) = self.block_index_for_offset(focus) else {
            return Ok(self.empty_caret_scroll_request(viewport, margin));
        };
        let config = layout.config().layout();
        let projection_bias = self.selection_projection_bias();
        let (caret_x, caret_y, line_count) = {
            let block_layout =
                self.block_layout_for_visual_state_with_shaper(block_index, config, shaper)?;
            let caret = block_layout.caret_for_source(focus, projection_bias)?;
            (
                caret.point().x(),
                caret.point().y(),
                block_layout.lines().len(),
            )
        };
        let height = config.line_height() * line_count.max(1) as f32;
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

    fn finish_caret_scroll_request(
        &self,
        layout: &ViewportLayout,
        viewport: ViewportSpan,
        margin: f32,
        position: CaretLayoutPosition,
    ) -> Result<CaretScrollRequest, EditorDocumentError> {
        let effective_margin = margin.min(viewport.height() / 2.0);
        let document_y = layout.height_index().prefix_height(position.block) + position.y;
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

    fn empty_caret_scroll_request(
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

    /// Replaces the selection after checking that it belongs to this revision.
    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), SelectionError> {
        selection.utf16_range(&self.snapshot())?;
        self.selection = selection;
        self.preferred_x = None;
        self.history.break_group();
        Ok(())
    }

    /// Applies a permanent transaction to the canonical source.
    ///
    /// An active composition is not implicitly rewritten or committed. If the
    /// transaction advances the revision, a later composition commit will
    /// return a stale-revision error and the platform can cancel/restart it.
    pub fn apply_transaction(
        &mut self,
        transaction: &Transaction,
    ) -> Result<AppliedTransaction, EditorDocumentError> {
        self.apply_transaction_with_group(transaction, HistoryGroup::External)
    }

    fn apply_transaction_with_group(
        &mut self,
        transaction: &Transaction,
        group: HistoryGroup,
    ) -> Result<AppliedTransaction, EditorDocumentError> {
        let applied = self.apply_transaction_core(transaction)?;
        self.history.record(&applied, group);
        Ok(applied)
    }

    fn apply_transaction_core(
        &mut self,
        transaction: &Transaction,
    ) -> Result<AppliedTransaction, EditorDocumentError> {
        self.preferred_x = None;
        let before_snapshot = self.snapshot();
        let applied = self.buffer.apply(transaction)?;
        let incremental = yu_markdown::parse_incremental(
            &self.markdown,
            applied.result_snapshot(),
            applied.change_set(),
        )?;
        self.selection = self
            .selection
            .map_through(applied.change_set(), applied.result_snapshot())?;
        // 改一条 reference definition 曾经要把所有缓存整表作废：v1 的投影
        // 先查表才知道 `[id]` 是不是一个链接。换成语法树之后 `[id]` 的
        // `LinkLabel` 是树给的结构，隐藏区间不再依赖索引（不变量 C6 说的
        // 「解析目标」才需要），所以没有东西要作废了。
        self.decorations
            .shift_through(applied.change_set(), applied.result_snapshot());
        self.layouts
            .map_through(applied.change_set(), applied.result_snapshot())
            .map_err(EditorDocumentError::Layout)?;
        self.viewport
            .map_through(
                applied.change_set(),
                applied.result_snapshot(),
                incremental.document(),
            )
            .map_err(EditorDocumentError::Viewport)?;
        self.decorations.retain_blocks(incremental.document());
        self.layouts.retain_blocks(incremental.document());
        self.markdown = incremental.into_document();
        self.last_source_change = source_change_from_applied(&before_snapshot, &applied)?;
        Ok(applied)
    }

    /// Starts or replaces the transient composition overlay.
    pub fn begin_composition(
        &mut self,
        replacement_range: TextRange,
        text: impl Into<Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), EditorDocumentError> {
        self.validate_source_range(replacement_range)?;
        self.history.break_group();
        self.preferred_x = None;
        self.composition = Some(CompositionOverlay::new(
            self.revision(),
            replacement_range,
            text,
            selection_utf16,
        )?);
        Ok(())
    }

    fn validate_source_range(&self, range: TextRange) -> Result<(), EditorDocumentError> {
        let snapshot = self.snapshot();
        snapshot.utf16_offset(range.start())?;
        snapshot.utf16_offset(range.end())?;
        Ok(())
    }

    /// Updates preedit and selection without mutating the canonical source.
    pub fn update_composition(
        &mut self,
        text: impl Into<Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), EditorDocumentError> {
        let composition = self
            .composition
            .as_mut()
            .ok_or(EditorDocumentError::CompositionNotActive)?;
        composition.update(text, selection_utf16)?;
        Ok(())
    }

    /// Commits the active overlay as one transaction.
    ///
    /// The overlay is cleared only after the transaction succeeds. A stale or
    /// otherwise invalid commit therefore leaves the overlay available for a
    /// caller to inspect and cancel explicitly.
    pub fn commit_composition(
        &mut self,
        committed_text: impl Into<Arc<str>>,
    ) -> Result<AppliedTransaction, EditorDocumentError> {
        let composition = self
            .composition
            .as_ref()
            .ok_or(EditorDocumentError::CompositionNotActive)?;
        let replacement_range = composition.replacement_range();
        let committed_text: Arc<str> = committed_text.into();
        let transaction = composition.clone().commit(Arc::clone(&committed_text));
        let applied = self.apply_transaction_with_group(&transaction, HistoryGroup::Composition)?;
        let cursor_offset = replacement_range
            .start()
            .checked_add(
                u64::try_from(committed_text.len())
                    .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?,
            )
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        self.selection = EditorSelection::cursor(
            applied.result_snapshot(),
            cursor_offset,
            crate::CaretAffinity::Downstream,
        )?;
        self.composition = None;
        self.preferred_x = None;
        self.last_source_change = None;
        self.history.break_group();
        Ok(applied)
    }

    /// Drops the active overlay without changing source or revision.
    #[must_use]
    pub fn cancel_composition(&mut self) -> bool {
        let cancelled = self.composition.take().is_some();
        if cancelled {
            self.preferred_x = None;
            self.history.break_group();
        }
        cancelled
    }

    /// Replaces the source for a newly opened document and resets its revision.
    pub fn reset_source(&mut self, source: impl Into<String>) -> Result<(), EditorDocumentError> {
        if self.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        self.buffer = TextBuffer::new(source);
        self.markdown = yu_markdown::parse(&self.buffer.snapshot());
        self.decorations.clear();
        self.layouts.clear();
        self.viewport.clear();
        self.history.clear();
        self.preferred_x = None;
        self.last_source_change = None;
        let snapshot = self.snapshot();
        self.selection = EditorSelection::cursor(
            &snapshot,
            snapshot.len_bytes(),
            crate::CaretAffinity::Downstream,
        )
        .expect("the end of a reset source is a valid caret");
        Ok(())
    }

    /// Executes a small revision-bound editing command set.
    pub fn execute(
        &mut self,
        command: EditorCommand,
    ) -> Result<CommandResult, EditorDocumentError> {
        self.last_source_change = None;
        // A native text input client owns the transient marked-text lifecycle
        // while a composition is active.  Keep the same invariant at the
        // platform-independent editor boundary so a caller cannot bypass the
        // FFI/menu availability guard and accidentally create a permanent
        // transaction over the composition's fixed replacement range.
        if self.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if !matches!(
            command,
            EditorCommand::MoveUp
                | EditorCommand::MoveDown
                | EditorCommand::MoveUpExtend
                | EditorCommand::MoveDownExtend
        ) {
            self.preferred_x = None;
        }
        match command {
            EditorCommand::InsertText(text) => self.insert_text(text),
            EditorCommand::DeleteBackward => self.delete_backward(),
            EditorCommand::DeleteForward => self.delete_forward(),
            EditorCommand::MoveLeft => self.move_left(),
            EditorCommand::MoveRight => self.move_right(),
            EditorCommand::MoveWordLeft => self.move_word_left(),
            EditorCommand::MoveWordRight => self.move_word_right(),
            EditorCommand::MoveUp => self.move_up(false),
            EditorCommand::MoveDown => self.move_down(false),
            EditorCommand::MoveUpExtend => self.move_up(true),
            EditorCommand::MoveDownExtend => self.move_down(true),
            EditorCommand::MoveTableCellNext => self.move_table_cell(false),
            EditorCommand::MoveTableCellPrevious => self.move_table_cell(true),
            EditorCommand::InsertNewline => self.insert_newline(),
            EditorCommand::IndentList => self.indent_list(),
            EditorCommand::OutdentList => self.outdent_list(),
            EditorCommand::Undo => self.undo(),
            EditorCommand::Redo => self.redo(),
            EditorCommand::ToggleTask { block } => self.toggle_task(block),
        }
    }

    /// Reports whether a command can currently make a meaningful editor
    /// transition. This is a read-only query for native menu/selector
    /// validation; executing a command remains the authoritative operation.
    #[must_use]
    pub fn command_available(&self, command: &EditorCommand) -> bool {
        if self.composition.is_some() {
            return false;
        }
        let snapshot = self.snapshot();
        match command {
            EditorCommand::InsertText(text) => !text.is_empty(),
            EditorCommand::DeleteBackward | EditorCommand::MoveLeft => {
                !self.selection.is_empty() || self.selection.focus() > ByteOffset::ZERO
            }
            EditorCommand::MoveWordLeft => {
                !self.selection.is_empty() || self.selection.focus() > ByteOffset::ZERO
            }
            EditorCommand::DeleteForward | EditorCommand::MoveRight => {
                !self.selection.is_empty() || self.selection.focus() < snapshot.len_bytes()
            }
            EditorCommand::MoveWordRight => {
                !self.selection.is_empty() || self.selection.focus() < snapshot.len_bytes()
            }
            EditorCommand::MoveUp | EditorCommand::MoveUpExtend => {
                self.vertical_command_available(VerticalDirection::Up)
            }
            EditorCommand::MoveDown | EditorCommand::MoveDownExtend => {
                self.vertical_command_available(VerticalDirection::Down)
            }
            EditorCommand::MoveTableCellNext => self.table_cell_navigation_target(false).is_some(),
            EditorCommand::MoveTableCellPrevious => {
                self.table_cell_navigation_target(true).is_some()
            }
            EditorCommand::InsertNewline => true,
            EditorCommand::IndentList => self
                .current_list_line()
                .is_some_and(|line| self.list_prefix(&line).is_some()),
            EditorCommand::OutdentList => self.current_list_line().is_some_and(|line| {
                self.list_prefix(&line).is_some_and(|_| {
                    line.content
                        .as_bytes()
                        .iter()
                        .take_while(|byte| **byte == b' ')
                        .next()
                        .is_some()
                })
            }),
            EditorCommand::Undo => self.history.stats().undo_entries() > 0,
            EditorCommand::Redo => self.history.stats().redo_entries() > 0,
            EditorCommand::ToggleTask { block } => self
                .markdown
                .blocks()
                .get(*block)
                .is_some_and(|block| matches!(block.kind(), BlockKind::TaskListItem { .. })),
        }
    }

    /// Resolves and executes a native key command against the current document
    /// context. Tab and Shift-Tab are only consumed when they actually edit a
    /// list item; in a paragraph they remain available for native focus or
    /// text-input policy.
    pub fn route_key(&mut self, event: KeyEvent) -> Result<KeyRouteResult, EditorDocumentError> {
        let Some(command) = self.command_for_key(event) else {
            return Ok(KeyRouteResult::Unhandled);
        };
        if self.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        let list_command = matches!(
            command,
            EditorCommand::IndentList | EditorCommand::OutdentList
        );
        let result = self.execute(command)?;
        if list_command && !result.changed() {
            return Ok(KeyRouteResult::Unhandled);
        }
        Ok(KeyRouteResult::Executed(result))
    }

    fn command_for_key(&mut self, event: KeyEvent) -> Option<EditorCommand> {
        if event.key() == crate::EditorKey::Tab {
            let previous = event.modifiers() == crate::KeyModifiers::SHIFT;
            let plain = event.modifiers() == crate::KeyModifiers::NONE;
            if (plain || previous) && self.table_cell_navigation_target(previous).is_some() {
                return Some(if previous {
                    EditorCommand::move_table_cell_previous()
                } else {
                    EditorCommand::move_table_cell_next()
                });
            }
        }
        command_for_key(event)
    }

    /// Toggles the source-backed `[ ]`/`[x]` marker of one task-list block.
    /// The edit is a normal transaction, so undo/history and projection cache
    /// invalidation follow the same path as keyboard input.
    pub fn toggle_task(&mut self, index: usize) -> Result<CommandResult, EditorDocumentError> {
        let block =
            self.markdown
                .blocks()
                .get(index)
                .ok_or(EditorDocumentError::BlockOutOfBounds {
                    index,
                    blocks: self.markdown.blocks().len(),
                })?;
        let state = match block.kind() {
            BlockKind::TaskListItem { state, .. } => state,
            _ => return Err(EditorDocumentError::BlockNotTaskList { index }),
        };
        let marker = yu_markdown::task_marker(&self.snapshot(), block)
            .ok_or(EditorDocumentError::BlockNotTaskList { index })?;
        let state_start = marker
            .range()
            .start()
            .checked_add(1)
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let state_end = state_start
            .checked_add(1)
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let replacement = match state {
            TaskState::Todo => "x",
            TaskState::Done => " ",
        };
        let transaction = Transaction::new(
            self.revision(),
            [yu_text::Edit::new(
                TextRange::new(state_start, state_end)
                    .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?,
                replacement,
            )],
        );
        self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
        Ok(self.command_result(true))
    }

    /// Replays one grouped set of inverse transactions without recording the
    /// replay itself as a new edit. The inverse of each replay becomes the
    /// corresponding redo transaction.
    pub fn undo(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let Some(entries) = self.history.pop_undo_group() else {
            self.history.break_group();
            return Ok(self.command_result(false));
        };
        let mut redo = Vec::with_capacity(entries.len());
        let mut rollback = Vec::with_capacity(entries.len());
        for entry in &entries {
            let transaction = entry.transaction_for(self.revision());
            match self.apply_transaction_core(&transaction) {
                Ok(applied) => {
                    rollback.push(applied.inverse().clone());
                    redo.push(HistoryEntry::new(applied.inverse().clone(), entry.group()));
                }
                Err(error) => {
                    for transaction in rollback.iter().rev() {
                        let _ = self.apply_transaction_core(transaction);
                    }
                    self.history.restore_undo_group(&entries);
                    return Err(error);
                }
            }
        }
        self.history.push_redo_group(redo);
        Ok(self.command_result(true).requiring_full_source_sync())
    }

    /// Replays one grouped set of forward transactions without recording the
    /// replay itself as a new edit. The inverse of each replay is restored to
    /// the undo stack in the original stack order.
    pub fn redo(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let Some(entries) = self.history.pop_redo_group() else {
            self.history.break_group();
            return Ok(self.command_result(false));
        };
        let mut undo = Vec::with_capacity(entries.len());
        let mut rollback = Vec::with_capacity(entries.len());
        for entry in &entries {
            let transaction = entry.transaction_for(self.revision());
            match self.apply_transaction_core(&transaction) {
                Ok(applied) => {
                    rollback.push(applied.inverse().clone());
                    undo.push(HistoryEntry::new(applied.inverse().clone(), entry.group()));
                }
                Err(error) => {
                    for transaction in rollback.iter().rev() {
                        let _ = self.apply_transaction_core(transaction);
                    }
                    self.history.restore_redo_group(&entries);
                    return Err(error);
                }
            }
        }
        self.history.push_undo_group(undo);
        Ok(self.command_result(true).requiring_full_source_sync())
    }

    /// Inserts a line ending and, when the caret is in a list item, continues
    /// its source prefix. A completed task always starts the next item as
    /// unchecked. Pressing Enter on an empty list item exits the list by
    /// removing that line's prefix while preserving its line ending.
    pub fn insert_newline(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let selection_range = self.selection.ordered_range();
        let line = source_line(&snapshot, selection_range.start())?;
        if self.selection.is_empty() {
            let caret = self.selection.focus();
            let relative = byte_distance(line.start, caret)?;
            if relative <= line.content.len()
                && let Some(prefix) = self.list_prefix(&line)
            {
                if relative >= prefix.content_start
                    && prefix.is_empty_item(&line.content)
                    && line
                        .content
                        .get(relative..)
                        .is_some_and(|tail| tail.trim().is_empty())
                {
                    let transaction = Transaction::new(
                        self.revision(),
                        [yu_text::Edit::new(line.content_range(), "")],
                    );
                    let applied =
                        self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
                    self.selection = EditorSelection::cursor(
                        applied.result_snapshot(),
                        line.start,
                        crate::CaretAffinity::Downstream,
                    )?;
                    return Ok(self.command_result(true));
                }

                let mut insertion = String::from(line.insertion_terminator());
                insertion.push_str(&prefix.continuation(&line.content));
                let offset = caret
                    .checked_add(u64::try_from(insertion.len()).map_err(|_| {
                        EditorDocumentError::Selection(SelectionError::InvalidRange)
                    })?)
                    .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
                let transaction = Transaction::new(
                    self.revision(),
                    [yu_text::Edit::new(
                        TextRange::empty(caret),
                        insertion.as_str(),
                    )],
                );
                let applied =
                    self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
                self.selection = EditorSelection::cursor(
                    applied.result_snapshot(),
                    offset,
                    crate::CaretAffinity::Downstream,
                )?;
                return Ok(self.command_result(true));
            }
        }

        let insertion = String::from(line.insertion_terminator());
        let offset = selection_range
            .start()
            .checked_add(
                u64::try_from(insertion.len())
                    .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?,
            )
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let transaction = Transaction::new(
            self.revision(),
            [yu_text::Edit::new(selection_range, insertion.as_str())],
        );
        let applied = self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
        self.selection = EditorSelection::cursor(
            applied.result_snapshot(),
            offset,
            crate::CaretAffinity::Downstream,
        )?;
        Ok(self.command_result(true))
    }

    /// Indents the current list item by two source spaces.
    pub fn indent_list(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let line = source_line(&snapshot, self.selection.focus())?;
        if self.list_prefix(&line).is_none() {
            return Ok(self.command_result(false));
        }
        let transaction = Transaction::new(
            self.revision(),
            [yu_text::Edit::new(TextRange::empty(line.start), "  ")],
        );
        self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
        Ok(self.command_result(true))
    }

    /// Removes up to two leading source spaces from the current list item.
    pub fn outdent_list(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let line = source_line(&snapshot, self.selection.focus())?;
        if self.list_prefix(&line).is_none() {
            return Ok(self.command_result(false));
        }
        let leading = line
            .content
            .as_bytes()
            .iter()
            .take_while(|byte| **byte == b' ')
            .count();
        if leading == 0 {
            return Ok(self.command_result(false));
        }
        let remove = leading.min(2);
        let end = line
            .start
            .checked_add(
                u64::try_from(remove)
                    .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?,
            )
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let range = TextRange::new(line.start, end)
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let transaction = Transaction::new(self.revision(), [yu_text::Edit::new(range, "")]);
        self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
        Ok(self.command_result(true))
    }

    fn insert_text(&mut self, text: Arc<str>) -> Result<CommandResult, EditorDocumentError> {
        if text.is_empty() {
            return Ok(self.command_result(false));
        }
        let range = self.selection.ordered_range();
        let offset = range
            .start()
            .checked_add(
                u64::try_from(text.len())
                    .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?,
            )
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let transaction = Transaction::new(
            self.revision(),
            [yu_text::Edit::new(range, Arc::clone(&text))],
        );
        let applied = self.apply_transaction_with_group(&transaction, HistoryGroup::Typing)?;
        self.selection = EditorSelection::cursor(
            applied.result_snapshot(),
            offset,
            crate::CaretAffinity::Downstream,
        )?;
        Ok(self.command_result(true))
    }

    fn delete_backward(&mut self) -> Result<CommandResult, EditorDocumentError> {
        if self.selection.is_empty()
            && let Some(result) = self.delete_empty_list_prefix()?
        {
            return Ok(result);
        }
        let range = if self.selection.is_empty() {
            let start = previous_grapheme_boundary(&self.snapshot(), self.selection.focus())?;
            TextRange::new(start, self.selection.focus())
                .expect("previous grapheme boundary must precede caret")
        } else {
            self.selection.ordered_range()
        };
        self.delete_range(range, HistoryGroup::Deletion)
    }

    fn delete_empty_list_prefix(&mut self) -> Result<Option<CommandResult>, EditorDocumentError> {
        let snapshot = self.snapshot();
        let line = source_line(&snapshot, self.selection.focus())?;
        let Some(prefix) = self.list_prefix(&line) else {
            return Ok(None);
        };
        let relative = byte_distance(line.start, self.selection.focus())?;
        if relative < prefix.content_start
            || !prefix.is_empty_item(&line.content)
            || !line
                .content
                .get(relative..)
                .is_some_and(|tail| tail.trim().is_empty())
        {
            return Ok(None);
        }
        let transaction = Transaction::new(
            self.revision(),
            [yu_text::Edit::new(line.content_range(), "")],
        );
        let applied = self.apply_transaction_with_group(&transaction, HistoryGroup::ListEditing)?;
        self.selection = EditorSelection::cursor(
            applied.result_snapshot(),
            line.start,
            crate::CaretAffinity::Downstream,
        )?;
        Ok(Some(self.command_result(true)))
    }

    fn delete_forward(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let range = if self.selection.is_empty() {
            let end = next_grapheme_boundary(&self.snapshot(), self.selection.focus())?;
            TextRange::new(self.selection.focus(), end)
                .expect("next grapheme boundary must follow caret")
        } else {
            self.selection.ordered_range()
        };
        self.delete_range(range, HistoryGroup::Deletion)
    }

    fn delete_range(
        &mut self,
        range: TextRange,
        group: HistoryGroup,
    ) -> Result<CommandResult, EditorDocumentError> {
        if range.is_empty() {
            return Ok(self.command_result(false));
        }
        let transaction = Transaction::new(
            self.revision(),
            [yu_text::Edit::new(range, Arc::<str>::from(""))],
        );
        let applied = self.apply_transaction_with_group(&transaction, group)?;
        self.selection = EditorSelection::cursor(
            applied.result_snapshot(),
            range.start(),
            crate::CaretAffinity::Downstream,
        )?;
        Ok(self.command_result(true))
    }

    fn move_left(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.history.break_group();
        let target = if self.selection.is_empty() {
            previous_grapheme_boundary(&self.snapshot(), self.selection.focus())?
        } else {
            self.selection.ordered_range().start()
        };
        self.selection =
            EditorSelection::cursor(&self.snapshot(), target, crate::CaretAffinity::Downstream)?;
        Ok(self.command_result(false))
    }

    fn move_table_cell(&mut self, previous: bool) -> Result<CommandResult, EditorDocumentError> {
        self.history.break_group();
        let Some(target) = self.table_cell_navigation_target(previous) else {
            return Ok(self.command_result(false));
        };
        self.selection =
            EditorSelection::cursor(&self.snapshot(), target, crate::CaretAffinity::Downstream)?;
        self.preferred_x = None;
        Ok(self.command_result(false))
    }

    fn table_cell_navigation_target(&self, previous: bool) -> Option<ByteOffset> {
        let focus = self.selection.focus();
        let block_index = self.block_index_for_offset(focus)?;
        let block = self.markdown.blocks().get(block_index)?;
        let snapshot = self.snapshot();
        let table = yu_markdown::parse_table_in_snapshot(&snapshot, block.range())?;
        let offset = usize::try_from(focus.get()).ok()?;
        let current = table.visible_cell_for_source(offset)?;
        let (_, target) = if previous {
            table.previous_visible_cell(current)?
        } else {
            table.next_visible_cell(current)?
        };
        ByteOffset::try_from(target.start()).ok()
    }

    fn move_right(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.history.break_group();
        let target = if self.selection.is_empty() {
            next_grapheme_boundary(&self.snapshot(), self.selection.focus())?
        } else {
            self.selection.ordered_range().end()
        };
        self.selection =
            EditorSelection::cursor(&self.snapshot(), target, crate::CaretAffinity::Downstream)?;
        Ok(self.command_result(false))
    }

    fn move_word_left(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.history.break_group();
        if !self.selection.is_empty() {
            let target = self.selection.ordered_range().start();
            self.selection = EditorSelection::cursor(
                &self.snapshot(),
                target,
                crate::CaretAffinity::Downstream,
            )?;
            return Ok(self.command_result(false));
        }

        let snapshot = self.snapshot();
        let line_index = snapshot.line_index(self.selection.focus())?;
        let line = source_line(&snapshot, self.selection.focus())?;
        let relative = byte_distance(line.start, self.selection.focus())?.min(line.content.len());
        let local_target = previous_word_boundary(&line.content, relative);
        if local_target < relative {
            let target =
                line.start
                    .checked_add(u64::try_from(local_target).map_err(|_| {
                        EditorDocumentError::Selection(SelectionError::InvalidRange)
                    })?)
                    .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
            self.selection =
                EditorSelection::cursor(&snapshot, target, crate::CaretAffinity::Downstream)?;
            return Ok(self.command_result(false));
        }

        let target =
            if line_index.get() == 0 {
                line.start
            } else {
                let previous_line = source_line(
                    &snapshot,
                    snapshot.line_start(LineIndex::new(line_index.get() - 1))?,
                )?;
                let local_target =
                    previous_word_boundary(&previous_line.content, previous_line.content.len());
                previous_line
                    .start
                    .checked_add(u64::try_from(local_target).map_err(|_| {
                        EditorDocumentError::Selection(SelectionError::InvalidRange)
                    })?)
                    .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?
            };
        self.selection =
            EditorSelection::cursor(&snapshot, target, crate::CaretAffinity::Downstream)?;
        Ok(self.command_result(false))
    }

    fn move_word_right(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.history.break_group();
        if !self.selection.is_empty() {
            let target = self.selection.ordered_range().end();
            self.selection = EditorSelection::cursor(
                &self.snapshot(),
                target,
                crate::CaretAffinity::Downstream,
            )?;
            return Ok(self.command_result(false));
        }

        let snapshot = self.snapshot();
        let line_index = snapshot.line_index(self.selection.focus())?;
        let line = source_line(&snapshot, self.selection.focus())?;
        let relative = byte_distance(line.start, self.selection.focus())?.min(line.content.len());
        let local_target = next_word_boundary(&line.content, relative);
        if local_target > relative {
            let target =
                line.start
                    .checked_add(u64::try_from(local_target).map_err(|_| {
                        EditorDocumentError::Selection(SelectionError::InvalidRange)
                    })?)
                    .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
            self.selection =
                EditorSelection::cursor(&snapshot, target, crate::CaretAffinity::Downstream)?;
            return Ok(self.command_result(false));
        }

        let next_index = line_index.get().saturating_add(1);
        let target =
            if next_index < snapshot.summary().line_count() {
                let next_line =
                    source_line(&snapshot, snapshot.line_start(LineIndex::new(next_index))?)?;
                let local_target = next_word_boundary(&next_line.content, 0);
                next_line
                    .start
                    .checked_add(u64::try_from(local_target).map_err(|_| {
                        EditorDocumentError::Selection(SelectionError::InvalidRange)
                    })?)
                    .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?
            } else {
                snapshot.len_bytes()
            };
        self.selection =
            EditorSelection::cursor(&snapshot, target, crate::CaretAffinity::Downstream)?;
        Ok(self.command_result(false))
    }

    fn move_up(&mut self, extend: bool) -> Result<CommandResult, EditorDocumentError> {
        self.move_vertical(VerticalDirection::Up, extend)
    }

    fn move_down(&mut self, extend: bool) -> Result<CommandResult, EditorDocumentError> {
        self.move_vertical(VerticalDirection::Down, extend)
    }

    fn move_vertical(
        &mut self,
        direction: VerticalDirection,
        extend: bool,
    ) -> Result<CommandResult, EditorDocumentError> {
        let config = self.viewport_config().layout();
        self.move_vertical_with_loader(direction, extend, config, |document, index, config| {
            document.block_layout_for_visual_state(index, config)
        })
    }

    /// Executes a vertical movement against a caller-owned shaped layout
    /// provider. The source selection/history contract is identical to the
    /// regular command path; only the block layout used for line/caret hit
    /// testing changes. The shaper remains outside the canonical editor.
    pub fn move_vertical_with_shaper<S: ShapingProvider>(
        &mut self,
        up: bool,
        extend: bool,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<CommandResult, EditorDocumentError> {
        self.last_source_change = None;
        let direction = if up {
            VerticalDirection::Up
        } else {
            VerticalDirection::Down
        };
        self.move_vertical_with_loader(direction, extend, config, |document, index, config| {
            document.block_layout_for_visual_state_with_shaper(index, config, shaper)
        })
    }

    fn move_vertical_with_loader<F>(
        &mut self,
        direction: VerticalDirection,
        extend: bool,
        config: LayoutConfig,
        mut load_layout: F,
    ) -> Result<CommandResult, EditorDocumentError>
    where
        F: FnMut(&mut Self, usize, LayoutConfig) -> Result<BlockView, EditorDocumentError>,
    {
        self.history.break_group();

        if !extend && !self.selection.is_empty() {
            let target = match direction {
                VerticalDirection::Up => self.selection.ordered_range().start(),
                VerticalDirection::Down => self.selection.ordered_range().end(),
            };
            self.selection = EditorSelection::cursor(
                &self.snapshot(),
                target,
                crate::CaretAffinity::Downstream,
            )?;
            self.preferred_x = None;
            return Ok(self.command_result(false));
        }

        let source = self.snapshot();
        let focus = self.selection.focus();
        let anchor = self.selection.anchor();
        let Some(block_index) = self.block_index_for_offset(focus) else {
            self.preferred_x = None;
            return Ok(self.command_result(false));
        };
        let projection_bias = match self.selection.affinity() {
            crate::CaretAffinity::Upstream => Bias::Before,
            crate::CaretAffinity::Downstream => Bias::After,
        };
        let preferred_x = self.preferred_x.map(PreferredCaretX::value);
        let block_count = self.markdown.blocks().len();
        let (current_x, target_block) = {
            let layout = load_layout(self, block_index, config)?;
            let caret = layout.caret_for_source(focus, projection_bias)?;
            let line_count = navigable_line_count(&layout, block_index + 1 < block_count);
            let next_line = caret.line().checked_add(1);
            let target_block = match direction {
                VerticalDirection::Up if caret.line() > 0 && caret.line() < line_count => {
                    Some((block_index, Some(caret.line() - 1)))
                }
                VerticalDirection::Up => block_index.checked_sub(1).map(|index| (index, None)),
                VerticalDirection::Down => {
                    if let Some(next_line) = next_line.filter(|line| *line < line_count) {
                        Some((block_index, Some(next_line)))
                    } else {
                        let next = block_index.saturating_add(1);
                        (next < block_count).then_some((next, Some(0)))
                    }
                }
            };
            (caret.point().x(), target_block)
        };
        let Some((target_block, target_line)) = target_block else {
            return Ok(self.command_result(false));
        };
        let desired_x = preferred_x.unwrap_or(current_x);
        let (target, target_width) = {
            let layout = load_layout(self, target_block, config)?;
            let target_line_index = target_line.unwrap_or_else(|| {
                navigable_line_count(&layout, target_block + 1 < block_count).saturating_sub(1)
            });
            let Some(target_line) = layout.lines().get(target_line_index) else {
                return Ok(self.command_result(false));
            };
            let hit = layout.hit_test(LayoutPoint::new(desired_x, target_line.y()))?;
            (hit.source(), target_line.width())
        };

        let affinity = vertical_hit_affinity(desired_x, target_width);
        self.selection = if extend {
            EditorSelection::range(&source, anchor, target, affinity)?
        } else {
            EditorSelection::cursor(&source, target, affinity)?
        };
        self.preferred_x = Some(PreferredCaretX::new(desired_x));
        Ok(self.command_result(false))
    }

    fn block_index_for_offset(&self, offset: ByteOffset) -> Option<usize> {
        let mut ending_at_offset = None;
        for (index, block) in self.markdown.blocks().iter().enumerate() {
            let range = block.range();
            if range.contains(offset) {
                return Some(index);
            }
            if range.end() == offset {
                ending_at_offset = Some(index);
            }
            if range.is_empty() && range.start() == offset {
                return Some(index);
            }
        }
        ending_at_offset
    }

    fn selection_projection_bias(&self) -> Bias {
        match self.selection.affinity() {
            crate::CaretAffinity::Upstream => Bias::Before,
            crate::CaretAffinity::Downstream => Bias::After,
        }
    }

    fn vertical_command_available(&self, direction: VerticalDirection) -> bool {
        if !self.selection.is_empty() {
            return true;
        }
        let Some(block_index) = self.block_index_for_offset(self.selection.focus()) else {
            return false;
        };
        let Some(block) = self.markdown.blocks().get(block_index) else {
            return false;
        };
        match direction {
            VerticalDirection::Up => {
                block_index > 0 || self.selection.focus() > block.range().start()
            }
            VerticalDirection::Down => {
                block_index.saturating_add(1) < self.markdown.blocks().len()
                    || self.selection.focus() < block.range().end()
            }
        }
    }

    fn command_result(&self, changed: bool) -> CommandResult {
        CommandResult::with_source_change(
            self.revision(),
            self.selection,
            changed,
            self.last_source_change,
        )
    }

    fn list_prefix(&self, line: &SourceLine) -> Option<ListLinePrefix> {
        let blocks = self.markdown.blocks();
        let mut low = 0_usize;
        let mut high = blocks.len();
        while low < high {
            let middle = low + (high - low) / 2;
            let block = blocks.get(middle)?;
            if block.range().end() <= line.start {
                low = middle.saturating_add(1);
            } else {
                high = middle;
            }
        }
        let block = blocks.get(low)?;
        if block.range().start() > line.start {
            return None;
        }
        if !matches!(
            block.kind(),
            BlockKind::ListItem { .. } | BlockKind::TaskListItem { .. }
        ) {
            return None;
        }
        ListLinePrefix::parse(&line.content)
    }

    fn current_list_line(&self) -> Option<SourceLine> {
        source_line(&self.snapshot(), self.selection.focus()).ok()
    }
}

/// 这个块上已经解码到位的图片。
///
/// 没解出来的不进表——widget 会给它一个 placeholder 盒子（不变量 D7），
/// 而不是让整块排版失败。
pub fn image_sizes<F>(decorations: &BlockDecorations, image_resolver: &F) -> Vec<ImageSize>
where
    F: Fn(ImageSpan) -> Option<ImageIntrinsicSize>,
{
    decorations
        .widgets()
        .iter()
        .copied()
        .map(|BlockWidget::Image(image)| image)
        .filter_map(|image| image_resolver(image).map(|size| (image.source(), size)))
        .collect()
}

fn source_change_from_applied(
    before: &TextSnapshot,
    applied: &AppliedTransaction,
) -> Result<Option<SourceChange>, EditorDocumentError> {
    let changes = applied.change_set().changes();
    let Some(first) = changes.first() else {
        return Ok(None);
    };
    let mut old_start = first.old_range().start();
    let mut old_end = first.old_range().end();
    let mut new_start = first.new_range().start();
    let mut new_end = first.new_range().end();
    for change in &changes[1..] {
        old_start = ByteOffset::new(old_start.get().min(change.old_range().start().get()));
        old_end = ByteOffset::new(old_end.get().max(change.old_range().end().get()));
        new_start = ByteOffset::new(new_start.get().min(change.new_range().start().get()));
        new_end = ByteOffset::new(new_end.get().max(change.new_range().end().get()));
    }
    let after = applied.result_snapshot();
    let old_range = Utf16Range::new(
        before.utf16_offset(old_start)?,
        before.utf16_offset(old_end)?,
    )
    .expect("change set old UTF-16 range must be ordered");
    let new_range = Utf16Range::new(after.utf16_offset(new_start)?, after.utf16_offset(new_end)?)
        .expect("change set new UTF-16 range must be ordered");
    Ok(Some(SourceChange::new(old_range, new_range)))
}

struct SourceLine {
    start: yu_core::ByteOffset,
    content_end: yu_core::ByteOffset,
    content: String,
    terminator: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerticalDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PreferredCaretX(f32);

#[derive(Clone, Copy, Debug, PartialEq)]
struct CaretLayoutPosition {
    source: ByteOffset,
    block: usize,
    x: f32,
    y: f32,
    height: f32,
}

impl PreferredCaretX {
    fn new(value: f32) -> Self {
        debug_assert!(value.is_finite() && value >= 0.0);
        Self(value.max(0.0))
    }

    fn value(self) -> f32 {
        self.0
    }
}

fn vertical_hit_affinity(x: f32, line_width: f32) -> crate::CaretAffinity {
    if line_width > 0.0 && x >= line_width {
        crate::CaretAffinity::Upstream
    } else {
        crate::CaretAffinity::Downstream
    }
}

fn validate_caret_margin(margin: f32) -> Result<(), ViewportError> {
    if margin.is_finite() && margin >= 0.0 {
        Ok(())
    } else {
        Err(ViewportError::InvalidMargin)
    }
}

fn navigable_line_count(layout: &BlockView, has_following_block: bool) -> usize {
    let line_count = layout.lines().len();
    let has_synthetic_trailing_line = layout.lines().last().is_some_and(|line| {
        line.source().is_empty() && line.source().start() == layout.source_range().end()
    });
    if has_following_block && line_count > 1 && has_synthetic_trailing_line {
        line_count - 1
    } else {
        line_count
    }
}

impl SourceLine {
    fn content_range(&self) -> TextRange {
        TextRange::new(self.start, self.content_end)
            .expect("source line content range must be ordered")
    }

    fn insertion_terminator(&self) -> &str {
        if self.terminator.is_empty() {
            "\n"
        } else {
            &self.terminator
        }
    }
}

fn source_line(
    snapshot: &TextSnapshot,
    offset: yu_core::ByteOffset,
) -> Result<SourceLine, EditorDocumentError> {
    let line = snapshot.line_index(offset)?;
    let line_count = snapshot.summary().line_count();
    let start = snapshot.line_start(line)?;
    let next_line = line
        .get()
        .checked_add(1)
        .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
    let end = if next_line < line_count {
        snapshot.line_start(LineIndex::new(next_line))?
    } else {
        snapshot.len_bytes()
    };
    let range = TextRange::new(start, end)
        .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
    let text = read_source_range(snapshot, range)?;
    let terminator_len = if text.ends_with("\r\n") {
        2
    } else if text.ends_with('\n') {
        1
    } else {
        0
    };
    let content_len = text.len().saturating_sub(terminator_len);
    let content_end = start
        .checked_add(
            u64::try_from(content_len)
                .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?,
        )
        .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
    Ok(SourceLine {
        start,
        content_end,
        content: text[..content_len].to_owned(),
        terminator: text[content_len..].to_owned(),
    })
}

fn read_source_range(
    snapshot: &TextSnapshot,
    range: TextRange,
) -> Result<String, EditorDocumentError> {
    let start = usize::try_from(range.start())
        .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?;
    let end = usize::try_from(range.end())
        .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?;
    let mut text = String::with_capacity(end.saturating_sub(start));
    for chunk in snapshot.chunk_cursor(range.start())? {
        let chunk_start = usize::try_from(chunk.start())
            .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let chunk_end = chunk_start
            .checked_add(chunk.text().len())
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        if chunk_start >= end {
            break;
        }
        let local_start = start.saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            text.push_str(&chunk.text()[local_start..local_end]);
        }
    }
    Ok(text)
}

fn byte_distance(
    start: yu_core::ByteOffset,
    end: yu_core::ByteOffset,
) -> Result<usize, EditorDocumentError> {
    usize::try_from(
        end.get()
            .checked_sub(start.get())
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?,
    )
    .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))
}

/// Errors raised while coordinating canonical edits and composition state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorDocumentError {
    Composition(CompositionError),
    Edit(EditError),
    Layout(LayoutError),
    Markdown(IncrementalParseError),
    Position(TextPositionError),
    /// 解析或装饰产出失败。
    Decoration(DecorationError),
    /// 视觉字节流出错：偏移越界、落在字符中间、preedit 区间不合法。
    Visual(VisualTextError),
    Selection(SelectionError),
    Viewport(ViewportError),
    BlockOutOfBounds {
        index: usize,
        blocks: usize,
    },
    BlockNotTaskList {
        index: usize,
    },
    CompositionNotActive,
    CompositionActive,
}

impl fmt::Display for EditorDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Composition(error) => error.fmt(formatter),
            Self::Edit(error) => error.fmt(formatter),
            Self::Layout(error) => error.fmt(formatter),
            Self::Markdown(error) => error.fmt(formatter),
            Self::Position(error) => error.fmt(formatter),
            Self::Decoration(error) => error.fmt(formatter),
            Self::Visual(error) => error.fmt(formatter),
            Self::Selection(error) => error.fmt(formatter),
            Self::Viewport(error) => error.fmt(formatter),
            Self::BlockOutOfBounds { index, blocks } => {
                write!(
                    formatter,
                    "Markdown block index {index} is outside {blocks} blocks"
                )
            }
            Self::BlockNotTaskList { index } => {
                write!(
                    formatter,
                    "Markdown block index {index} is not a task-list item"
                )
            }
            Self::CompositionNotActive => formatter.write_str("no active composition"),
            Self::CompositionActive => formatter.write_str("composition is already active"),
        }
    }
}

impl Error for EditorDocumentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Composition(error) => Some(error),
            Self::Edit(error) => Some(error),
            Self::Layout(error) => Some(error),
            Self::Markdown(error) => Some(error),
            Self::Position(error) => Some(error),
            Self::Decoration(error) => Some(error),
            Self::Visual(error) => Some(error),
            Self::Selection(error) => Some(error),
            Self::Viewport(error) => Some(error),
            Self::BlockOutOfBounds { .. }
            | Self::BlockNotTaskList { .. }
            | Self::CompositionNotActive
            | Self::CompositionActive => None,
        }
    }
}

impl From<CompositionError> for EditorDocumentError {
    fn from(error: CompositionError) -> Self {
        Self::Composition(error)
    }
}

impl From<EditError> for EditorDocumentError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

impl From<IncrementalParseError> for EditorDocumentError {
    fn from(error: IncrementalParseError) -> Self {
        Self::Markdown(error)
    }
}

impl From<LayoutError> for EditorDocumentError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<ViewportError> for EditorDocumentError {
    fn from(error: ViewportError) -> Self {
        Self::Viewport(error)
    }
}

impl From<TextPositionError> for EditorDocumentError {
    fn from(error: TextPositionError) -> Self {
        Self::Position(error)
    }
}

impl From<DecorationError> for EditorDocumentError {
    fn from(error: DecorationError) -> Self {
        Self::Decoration(error)
    }
}

impl From<VisualTextError> for EditorDocumentError {
    fn from(error: VisualTextError) -> Self {
        Self::Visual(error)
    }
}

impl From<SelectionError> for EditorDocumentError {
    fn from(error: SelectionError) -> Self {
        Self::Selection(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::{TableResizeGesture, TableResizeTarget};
    use crate::{EditorKey, KeyModifiers, SourceSync};
    use unicode_segmentation::UnicodeSegmentation;
    use yu_core::{
        ByteOffset, FontFaceId, Glyph, GlyphId, GlyphRun, Script, ShapedText, ShapingProvider,
        TextDirection, TextStyle, Utf16Offset,
    };
    use yu_markdown::{BlockKind, BlockOrnament};
    use yu_text::Edit;

    fn source_range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test source range should be ordered")
    }

    fn utf16_range(start: u64, end: u64) -> Utf16Range {
        Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end))
            .expect("test UTF-16 range should be ordered")
    }

    fn set_caret(document: &mut EditorDocument, offset: usize) {
        let selection = EditorSelection::cursor(
            &document.snapshot(),
            ByteOffset::try_from(offset).expect("test offset fits"),
            crate::CaretAffinity::Downstream,
        )
        .expect("test caret should be valid");
        document
            .set_selection(selection)
            .expect("test caret should belong to document");
    }

    #[test]
    fn table_tab_navigation_uses_visible_source_cells() {
        let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find('A').expect("header A"));

        let next = document
            .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
            .expect("tab route");
        assert!(matches!(next, KeyRouteResult::Executed(result) if !result.changed()));
        assert_eq!(
            document.selection().focus(),
            ByteOffset::new(source.find('B').expect("header B") as u64)
        );

        let previous = document
            .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::SHIFT))
            .expect("shift-tab route");
        assert!(matches!(previous, KeyRouteResult::Executed(result) if !result.changed()));
        assert_eq!(
            document.selection().focus(),
            ByteOffset::new(source.find('A').expect("header A") as u64)
        );

        set_caret(&mut document, source.rfind('2').expect("last cell"));
        assert_eq!(
            document
                .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
                .expect("last-cell tab route"),
            KeyRouteResult::Unhandled
        );
        assert_eq!(
            document.selection().focus(),
            ByteOffset::new(source.rfind('2').expect("last cell") as u64)
        );
    }

    #[test]
    fn table_column_resize_layout_is_transient_and_revision_bound() {
        let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let mut document = EditorDocument::new(source);
        let config = LayoutConfig::new(20.0, 2.0);
        let hit = document
            .block_layout(0, config)
            .expect("table layout")
            .table()
            .expect("table metadata")
            .resize_hit_test(LayoutPoint::new(3.0, 0.5), 0.0)
            .expect("resize hit-test")
            .expect("column divider");
        assert_eq!(hit.target(), TableResizeTarget::Column { index: 0 });
        let revision = document.revision();
        let mut gesture = TableResizeGesture::begin(revision, 0, hit, 3.0).expect("gesture begin");
        gesture.update(revision, 4.0).expect("gesture update");
        let commit = gesture.finish(revision).expect("gesture finish");
        let resized = document
            .block_layout_with_table_resize(0, config, commit)
            .expect("transient table resize");

        assert_eq!(
            resized.table().expect("resized table").column_widths(),
            &[4.0, 2.0]
        );
        assert_eq!(
            resized.table().expect("resized table").cells()[1]
                .bounds()
                .x(),
            4.0
        );
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.layout_cache_stats().entries(), 1);
        assert_eq!(
            document
                .block_layout(0, config)
                .expect("canonical table layout")
                .table()
                .expect("canonical table metadata")
                .column_widths(),
            &[3.0, 3.0]
        );

        document
            .apply_transaction(&Transaction::new(
                revision,
                [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
            ))
            .expect("source edit");
        assert_eq!(
            document
                .block_layout_with_table_resize(0, config, commit)
                .err(),
            Some(EditorDocumentError::Layout(LayoutError::Upstream(
                "table resize commit and document revisions differ".into()
            )))
        );
    }

    #[derive(Clone, Copy, Debug)]
    struct WideShaper;

    impl ShapingProvider for WideShaper {
        type Error = &'static str;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: TextStyle,
        ) -> Result<ShapedText, Self::Error> {
            let glyphs = text
                .grapheme_indices(true)
                .map(|(start, cluster)| {
                    let end = start + cluster.len();
                    let source_start = source
                        .start()
                        .checked_add(u64::try_from(start).expect("test offset fits"))
                        .expect("source offset fits");
                    let source_end = source
                        .start()
                        .checked_add(u64::try_from(end).expect("test offset fits"))
                        .expect("source offset fits");
                    let glyph_source = TextRange::new(source_start, source_end)
                        .expect("glyph source range should be ordered");
                    Glyph::new(GlyphId::from_raw(1), glyph_source, 2.0, 0.0, 0.0)
                })
                .collect();
            Ok(ShapedText::new(
                source,
                vec![GlyphRun::new(
                    FontFaceId::from_raw(1),
                    source,
                    style,
                    TextDirection::Ltr,
                    Script::Latin,
                    glyphs,
                )],
            ))
        }
    }

    #[test]
    fn composition_lives_in_document_and_commits_once() {
        let mut document = EditorDocument::new("输入: ");
        document
            .begin_composition(source_range(8, 8), "にほんご", utf16_range(4, 4))
            .expect("Japanese composition should begin");
        document
            .update_composition("にほんご", utf16_range(4, 4))
            .expect("Japanese composition should update");

        assert_eq!(document.snapshot().as_str(), "输入: ");
        assert_eq!(document.revision(), Revision::INITIAL);
        assert_eq!(
            document.composition().map(CompositionOverlay::text),
            Some("にほんご")
        );

        document
            .commit_composition("日本語")
            .expect("Japanese composition should commit");
        assert_eq!(document.snapshot().as_str(), "输入: 日本語");
        assert_eq!(document.revision(), Revision::new(1));
        assert_eq!(
            document.selection().focus().get(),
            "输入: 日本語".len() as u64
        );
        assert!(document.composition().is_none());
    }

    #[test]
    fn commands_edit_unicode_graphemes_and_share_document_revision() {
        let mut document = EditorDocument::new("e\u{301}x");
        let start = EditorSelection::cursor(
            &document.snapshot(),
            yu_core::ByteOffset::ZERO,
            crate::CaretAffinity::Downstream,
        )
        .expect("start caret should be valid");
        document
            .set_selection(start)
            .expect("selection should belong to document");

        let inserted = document
            .execute(EditorCommand::insert_text("羽"))
            .expect("insert should succeed");
        assert!(inserted.changed());
        assert_eq!(inserted.revision(), Revision::new(1));
        assert_eq!(
            inserted.source_sync(),
            SourceSync::Range(SourceChange::new(utf16_range(0, 0), utf16_range(0, 1)))
        );
        assert_eq!(document.snapshot().as_str(), "羽e\u{301}x");
        assert_eq!(document.selection().focus().get(), "羽".len() as u64);

        let deleted = document
            .execute(EditorCommand::DeleteBackward)
            .expect("backspace should remove one grapheme");
        assert_eq!(
            deleted.source_sync(),
            SourceSync::Range(SourceChange::new(utf16_range(0, 1), utf16_range(0, 0)))
        );
        assert_eq!(document.snapshot().as_str(), "e\u{301}x");
        assert_eq!(document.revision(), Revision::new(2));

        let moved = document
            .execute(EditorCommand::MoveRight)
            .expect("right should move over one grapheme");
        assert_eq!(moved.source_sync(), SourceSync::None);
        let deleted = document
            .execute(EditorCommand::DeleteForward)
            .expect("forward delete should remove x");
        assert_eq!(
            deleted.source_sync(),
            SourceSync::Range(SourceChange::new(utf16_range(2, 3), utf16_range(2, 2)))
        );
        assert_eq!(document.snapshot().as_str(), "e\u{301}");
        assert_eq!(document.revision(), Revision::new(3));
    }

    #[test]
    fn key_route_only_consumes_tab_for_list_contexts() {
        let mut plain = EditorDocument::new("paragraph");
        assert_eq!(
            plain
                .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
                .expect("plain tab route should succeed"),
            KeyRouteResult::Unhandled
        );
        assert_eq!(plain.snapshot().as_str(), "paragraph");
        assert_eq!(plain.revision(), Revision::INITIAL);

        let mut list = EditorDocument::new("- item");
        let KeyRouteResult::Executed(result) = list
            .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
            .expect("list tab route should succeed")
        else {
            panic!("list tab should be consumed");
        };
        assert!(result.changed());
        let source_change = result
            .source_change()
            .expect("list tab should expose a source change");
        assert_eq!(source_change.old_range(), utf16_range(0, 0));
        assert_eq!(source_change.new_range(), utf16_range(0, 2));
        assert_eq!(list.snapshot().as_str(), "  - item");
    }

    #[test]
    fn command_availability_tracks_context_without_mutating_document() {
        let mut document = EditorDocument::new("");
        assert!(!document.command_available(&EditorCommand::undo()));
        assert!(!document.command_available(&EditorCommand::redo()));
        assert!(!document.command_available(&EditorCommand::MoveLeft));
        assert!(!document.command_available(&EditorCommand::DeleteBackward));
        assert!(document.command_available(&EditorCommand::insert_newline()));

        let revision = document.revision();
        document
            .execute(EditorCommand::insert_text("羽"))
            .expect("insert should succeed");
        assert_eq!(
            document.revision(),
            revision.next().expect("revision should advance")
        );
        assert!(document.command_available(&EditorCommand::undo()));
        assert!(document.command_available(&EditorCommand::MoveLeft));
        assert!(!document.command_available(&EditorCommand::MoveRight));
        assert!(document.command_available(&EditorCommand::DeleteBackward));

        let mut list = EditorDocument::new("- item");
        assert!(list.command_available(&EditorCommand::indent_list()));
        assert!(!list.command_available(&EditorCommand::outdent_list()));
        list.execute(EditorCommand::indent_list())
            .expect("indent should succeed");
        assert!(list.command_available(&EditorCommand::outdent_list()));

        let task = EditorDocument::new("- [ ] item");
        assert!(task.command_available(&EditorCommand::toggle_task(0)));
        assert!(!task.command_available(&EditorCommand::toggle_task(1)));

        let mut composing = EditorDocument::new("text");
        composing
            .begin_composition(source_range(4, 4), "x", utf16_range(0, 0))
            .expect("composition should begin");
        assert!(!composing.command_available(&EditorCommand::DeleteBackward));
        assert!(!composing.command_available(&EditorCommand::insert_newline()));
    }

    #[test]
    fn word_commands_move_by_unicode_segments_without_editing_source() {
        let source = "hello  世界🙂!\nnext";
        let mut document = EditorDocument::new(source);
        let first_line_end = source.find('\n').expect("line ending");
        set_caret(&mut document, first_line_end);
        let revision = document.revision();

        document
            .execute(EditorCommand::move_word_left())
            .expect("word left should succeed");
        assert_eq!(
            document.selection().focus().get() as usize,
            "hello  世界🙂".len()
        );
        document
            .execute(EditorCommand::move_word_left())
            .expect("word left should reach emoji");
        assert_eq!(
            document.selection().focus().get() as usize,
            "hello  世界".len()
        );

        document
            .execute(EditorCommand::move_word_right())
            .expect("word right should reach emoji end");
        assert_eq!(
            document.selection().focus().get() as usize,
            "hello  世界🙂".len()
        );
        document
            .execute(EditorCommand::move_word_right())
            .expect("word right should reach line ending");
        assert_eq!(document.selection().focus().get() as usize, first_line_end);
        document
            .execute(EditorCommand::move_word_right())
            .expect("word right should cross the line");
        assert_eq!(
            document.selection().focus().get() as usize,
            first_line_end + 1 + "next".len()
        );

        assert_eq!(document.revision(), revision);
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn vertical_commands_use_layout_lines_and_retain_preferred_x() {
        let source = "abcdefghij\nxy\n1234567890";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, "abcdefghij".len());
        let revision = document.revision();

        document
            .execute(EditorCommand::move_down())
            .expect("first vertical move should succeed");
        assert_eq!(document.selection().focus().get(), 13);
        assert_eq!(document.preferred_x, Some(PreferredCaretX::new(10.0)));

        document
            .execute(EditorCommand::move_down())
            .expect("second vertical move should preserve preferred x");
        assert_eq!(document.selection().focus().get(), 24);
        assert_eq!(document.preferred_x, Some(PreferredCaretX::new(10.0)));
        assert_eq!(document.revision(), revision);
        assert_eq!(document.snapshot().as_str(), source);

        document
            .execute(EditorCommand::move_up())
            .expect("up should return to the short line");
        assert_eq!(document.selection().focus().get(), 13);
        document
            .execute(EditorCommand::MoveLeft)
            .expect("horizontal movement should clear preferred x");
        assert_eq!(document.preferred_x, None);
    }

    #[test]
    fn shaped_vertical_commands_use_caller_shaper_and_keep_revision() {
        let source = "abcdefghij\nxy\n1234567890";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, "abcdefghij".len());
        let revision = document.revision();
        let config = LayoutConfig::new(80.0, 12.0).with_default_advance(2.0);

        document
            .move_vertical_with_shaper(false, false, config, &WideShaper)
            .expect("shaped down should succeed");
        assert_eq!(document.selection().focus().get(), 13);
        assert_eq!(document.preferred_x, Some(PreferredCaretX::new(20.0)));

        document
            .move_vertical_with_shaper(false, false, config, &WideShaper)
            .expect("second shaped down should preserve preferred x");
        assert_eq!(document.selection().focus().get(), 24);
        assert_eq!(document.preferred_x, Some(PreferredCaretX::new(20.0)));
        assert_eq!(document.revision(), revision);
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn vertical_command_collapses_selection_and_stops_at_block_boundary() {
        let source = "one\ntwo";
        let mut document = EditorDocument::new(source);
        let selection = EditorSelection::range(
            &document.snapshot(),
            ByteOffset::new(0),
            ByteOffset::new(7),
            crate::CaretAffinity::Downstream,
        )
        .expect("selection should be valid");
        document
            .set_selection(selection)
            .expect("selection should belong to the document");
        assert!(document.command_available(&EditorCommand::move_up()));
        assert!(document.command_available(&EditorCommand::move_down()));

        document
            .execute(EditorCommand::move_up())
            .expect("up should collapse to the ordered start");
        assert_eq!(document.selection().focus(), ByteOffset::ZERO);
        assert!(document.selection().is_empty());

        document
            .execute(EditorCommand::move_down())
            .expect("down should move within the block");
        assert_eq!(document.selection().focus().get(), 4);
        document
            .execute(EditorCommand::move_down())
            .expect("down at the block boundary should be a no-op");
        assert_eq!(document.selection().focus().get(), 4);
    }

    #[test]
    fn vertical_commands_cross_adjacent_markdown_blocks() {
        let source = "# title\ntext";
        let mut document = EditorDocument::new(source);
        set_caret(
            &mut document,
            source.find("text").expect("paragraph should exist"),
        );
        assert!(document.command_available(&EditorCommand::move_up()));

        document
            .execute(EditorCommand::move_up())
            .expect("up should enter the preceding heading block");
        assert_eq!(document.selection().focus().get(), 0);

        document
            .execute(EditorCommand::move_down())
            .expect("down should return to the following paragraph block");
        assert_eq!(document.selection().focus().get(), 8);
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn vertical_extend_commands_preserve_anchor_and_preferred_x() {
        let source = "one\ntwo\nthree";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, 0);

        document
            .execute(EditorCommand::move_down_extend())
            .expect("shift-down should extend to the second line");
        assert_eq!(document.selection().anchor().get(), 0);
        assert_eq!(document.selection().focus().get(), 4);
        assert!(!document.selection().is_empty());

        document
            .execute(EditorCommand::move_down_extend())
            .expect("repeated shift-down should preserve the anchor");
        assert_eq!(document.selection().anchor().get(), 0);
        assert_eq!(document.selection().focus().get(), 8);

        document
            .execute(EditorCommand::move_up_extend())
            .expect("shift-up should contract toward the anchor");
        assert_eq!(document.selection().anchor().get(), 0);
        assert_eq!(document.selection().focus().get(), 4);

        document
            .execute(EditorCommand::move_up_extend())
            .expect("shift-up should collapse at the anchor");
        assert!(document.selection().is_empty());
        assert_eq!(document.selection().focus().get(), 0);
    }

    #[test]
    fn newline_continues_task_as_unchecked_and_increments_ordered_lists() {
        let source = "- [x] done\n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find('\n').expect("line ending"));
        document
            .execute(EditorCommand::InsertNewline)
            .expect("task newline should apply");
        assert_eq!(document.snapshot().as_str(), "- [x] done\n- [ ] \n");
        assert_eq!(
            document.selection().focus().get() as usize,
            "- [x] done\n- [ ] ".len()
        );

        let source = "9. item\n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find('\n').expect("line ending"));
        document
            .execute(EditorCommand::insert_newline())
            .expect("ordered newline should apply");
        assert_eq!(document.snapshot().as_str(), "9. item\n10. \n");
    }

    #[test]
    fn empty_list_enter_and_backspace_exit_without_losing_line_ending() {
        let source = "- [ ] \n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find('\n').expect("line ending"));
        document
            .execute(EditorCommand::insert_newline())
            .expect("empty task newline should apply");
        assert_eq!(document.snapshot().as_str(), "\n");
        assert_eq!(document.selection().focus(), ByteOffset::ZERO);

        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find('\n').expect("line ending"));
        document
            .execute(EditorCommand::DeleteBackward)
            .expect("empty task backspace should apply");
        assert_eq!(document.snapshot().as_str(), "\n");
        assert_eq!(document.selection().focus(), ByteOffset::ZERO);
    }

    #[test]
    fn list_indent_and_outdent_are_source_transactions() {
        let source = "- [ ] item\n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find('\n').expect("line ending"));
        document
            .execute(EditorCommand::indent_list())
            .expect("list indent should apply");
        assert_eq!(document.snapshot().as_str(), "  - [ ] item\n");
        assert_eq!(
            document.selection().focus().get(),
            (source.find('\n').expect("line ending") + 2) as u64
        );

        document
            .execute(EditorCommand::outdent_list())
            .expect("list outdent should apply");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(
            document.selection().focus().get(),
            source.find('\n').expect("line ending") as u64
        );
    }

    #[test]
    fn newline_on_plain_text_does_not_invent_a_list_prefix() {
        let source = "plain\n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find('\n').expect("line ending"));
        document
            .execute(EditorCommand::insert_newline())
            .expect("plain newline should apply");
        assert_eq!(document.snapshot().as_str(), "plain\n\n");

        let source = "plain";
        let mut document = EditorDocument::new(source);
        // 光标显式放到行尾。此前这里依赖「新文档的光标在文末」这个隐含默认，
        // 而那个默认本身是错的——打开文件应该看到开头。
        set_caret(&mut document, source.len());
        document
            .execute(EditorCommand::insert_newline())
            .expect("unterminated newline should apply");
        assert_eq!(document.snapshot().as_str(), "plain\n");
    }

    #[test]
    fn list_commands_preserve_crlf_and_ignore_fenced_code_lines() {
        let source = "- [ ] item\r\n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find("\r\n").expect("line ending"));
        document
            .execute(EditorCommand::insert_newline())
            .expect("CRLF list newline should apply");
        assert_eq!(document.snapshot().as_str(), "- [ ] item\r\n- [ ] \r\n");

        let source = "```\n- [ ] code\n```\n";
        let mut document = EditorDocument::new(source);
        let code_line = source.find("code").expect("code line");
        set_caret(&mut document, code_line + "code".len());
        document
            .execute(EditorCommand::insert_newline())
            .expect("fenced code newline should apply");
        assert_eq!(document.snapshot().as_str(), "```\n- [ ] code\n\n```\n");
    }

    #[test]
    fn undo_groups_typing_and_redoes_in_forward_order() {
        let mut document = EditorDocument::new("");
        document
            .execute(EditorCommand::insert_text("a"))
            .expect("first insert should apply");
        document
            .execute(EditorCommand::insert_text("b"))
            .expect("second insert should apply");
        assert_eq!(document.history_stats().undo_entries(), 2);

        document
            .execute(EditorCommand::undo())
            .expect("grouped undo should apply");
        assert_eq!(document.snapshot().as_str(), "");
        assert_eq!(document.history_stats().undo_entries(), 0);
        assert_eq!(document.history_stats().redo_entries(), 2);

        document
            .execute(EditorCommand::redo())
            .expect("grouped redo should apply");
        assert_eq!(document.snapshot().as_str(), "ab");
        assert_eq!(document.history_stats().undo_entries(), 2);
        assert_eq!(document.history_stats().redo_entries(), 0);
    }

    #[test]
    fn cursor_motion_breaks_typing_group_and_new_edit_clears_redo() {
        let mut document = EditorDocument::new("");
        document
            .execute(EditorCommand::insert_text("ab"))
            .expect("insert should apply");
        document
            .execute(EditorCommand::MoveLeft)
            .expect("cursor move should apply");
        document
            .execute(EditorCommand::insert_text("x"))
            .expect("second insert should apply");

        document.execute(EditorCommand::undo()).expect("undo x");
        assert_eq!(document.snapshot().as_str(), "ab");
        document.execute(EditorCommand::undo()).expect("undo ab");
        assert_eq!(document.snapshot().as_str(), "");
        assert_eq!(document.history_stats().redo_entries(), 2);

        document
            .execute(EditorCommand::insert_text("new"))
            .expect("new edit should apply");
        assert_eq!(document.history_stats().redo_entries(), 0);
    }

    #[test]
    fn list_and_task_commands_are_undoable_through_the_same_history() {
        let mut document = EditorDocument::new("- [x] item\n");
        set_caret(&mut document, "- [x] item".len());
        document
            .execute(EditorCommand::insert_newline())
            .expect("list continuation should apply");
        assert_eq!(document.snapshot().as_str(), "- [x] item\n- [ ] \n");
        document
            .execute(EditorCommand::undo())
            .expect("undo list continuation");
        assert_eq!(document.snapshot().as_str(), "- [x] item\n");
        document
            .execute(EditorCommand::redo())
            .expect("redo list continuation");
        assert_eq!(document.snapshot().as_str(), "- [x] item\n- [ ] \n");

        let mut document = EditorDocument::new("- [ ] item\n");
        document
            .execute(EditorCommand::toggle_task(0))
            .expect("task toggle should apply");
        document
            .execute(EditorCommand::undo())
            .expect("undo task toggle");
        assert_eq!(document.snapshot().as_str(), "- [ ] item\n");
        document
            .execute(EditorCommand::redo())
            .expect("redo task toggle");
        assert_eq!(document.snapshot().as_str(), "- [x] item\n");

        set_caret(&mut document, "- [x] item".len());
        document
            .execute(EditorCommand::indent_list())
            .expect("indent should apply");
        document
            .execute(EditorCommand::undo())
            .expect("undo indent");
        assert_eq!(document.snapshot().as_str(), "- [x] item\n");
    }

    #[test]
    fn composition_preedit_is_not_history_but_commit_is_undoable() {
        let mut document = EditorDocument::new("before");
        document
            .begin_composition(source_range(6, 6), "にほんご", utf16_range(0, 0))
            .expect("composition should begin");
        document
            .update_composition("日本語", utf16_range(0, 0))
            .expect("composition should update");
        assert_eq!(document.history_stats().undo_entries(), 0);

        document
            .commit_composition("日本語")
            .expect("composition should commit");
        assert_eq!(document.snapshot().as_str(), "before日本語");
        assert_eq!(document.history_stats().undo_entries(), 1);
        document
            .execute(EditorCommand::undo())
            .expect("undo commit");
        assert_eq!(document.snapshot().as_str(), "before");
    }

    #[test]
    fn composition_layout_is_transient_and_uses_preedit_text() {
        let mut document = EditorDocument::new("hello");
        let revision = document.revision();
        assert_eq!(document.layout_cache_stats().entries(), 0);
        document
            .begin_composition(source_range(5, 5), "日本", utf16_range(1, 1))
            .expect("composition should begin");
        let layout = document
            .block_layout_with_composition(0, LayoutConfig::new(80.0, 1.0))
            .expect("composition metrics layout");
        assert_eq!(layout.revision(), revision);
        assert_eq!(layout.visual().composition_text(), Some("日本"));
        assert_eq!(
            layout
                .visual()
                .composition_selection_visual()
                .map(|range| range.start().get()),
            Some(8)
        );
        assert_eq!(document.snapshot().as_str(), "hello");
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_stats().undo_entries(), 0);
        assert_eq!(document.layout_cache_stats().entries(), 0);

        document
            .update_composition("日本語", utf16_range(3, 3))
            .expect("composition update");
        let updated = document
            .block_layout_with_composition(0, LayoutConfig::new(80.0, 1.0))
            .expect("updated composition metrics layout");
        assert_eq!(updated.visual().composition_text(), Some("日本語"));
        assert_eq!(document.revision(), revision);
        assert_eq!(document.layout_cache_stats().entries(), 0);

        assert!(document.cancel_composition());
        let canonical = document
            .block_layout(0, LayoutConfig::new(80.0, 1.0))
            .expect("canonical layout after cancel");
        assert_eq!(canonical.visual().composition_text(), None);
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn composition_shaped_layout_uses_temporary_shape_coordinates() {
        let mut document = EditorDocument::new("hello");
        document
            .begin_composition(source_range(5, 5), "日本", utf16_range(0, 0))
            .expect("composition should begin");
        let layout = document
            .block_layout_with_composition_and_shaper(0, LayoutConfig::new(80.0, 1.0), &WideShaper)
            .expect("composition shaped layout");
        assert_eq!(layout.visual().composition_text(), Some("日本"));
        assert!(layout.glyphs().len() >= 2);
        assert!(
            layout
                .glyphs()
                .iter()
                .filter(|glyph| glyph.visual().start().get() >= 5)
                .filter(|glyph| glyph.source() == source_range(5, 5))
                .count()
                >= 2
        );
        assert_eq!(document.snapshot().as_str(), "hello");
    }

    #[test]
    fn composition_block_index_rejects_cross_block_replacements() {
        let mut document = EditorDocument::new("first\n\nsecond");
        document
            .begin_composition(source_range(2, 10), "x", utf16_range(0, 1))
            .expect("composition should begin");
        assert_eq!(document.composition_block_index(), None);

        let _ = document.cancel_composition();
        document
            .begin_composition(source_range(2, 2), "x", utf16_range(0, 1))
            .expect("composition should begin");
        assert_eq!(document.composition_block_index(), Some(0));
    }

    #[test]
    fn cross_block_composition_projects_first_and_clears_following_blocks() {
        let source = "first **x**\n\nsecond 日本語";
        let mut document = EditorDocument::new(source);
        let start = source.find("x").expect("first block target");
        let end = source.find("日本語").expect("last block target") + "日本".len();
        document
            .begin_composition(
                source_range(start as u64, end as u64),
                "日本🙂",
                utf16_range(2, 2),
            )
            .expect("cross-block composition should begin");

        let span = document
            .composition_block_range()
            .expect("composition should touch blocks");
        assert!(span.len() >= 2);
        assert_eq!(document.composition_block_index(), None);
        let config = LayoutConfig::new(80.0, 1.0);
        for index in span.clone() {
            let layout = document
                .block_layout_with_composition_and_shaper(index, config, &WideShaper)
                .expect("transient cross-block layout");
            if index == span.start {
                assert_eq!(layout.visual().composition_text(), Some("日本🙂"));
                assert!(layout.glyphs().len() >= 2);
            } else {
                assert_eq!(layout.visual().composition_text(), Some(""));
            }
        }

        document
            .set_viewport_config(ViewportConfig::new(config, 1.0, 0.0))
            .expect("viewport config");
        let viewport = document
            .visible_blocks_with_composition_and_shaper(ViewportSpan::new(0.0, 12.0), &WideShaper)
            .expect("transient cross-block viewport");
        assert_eq!(viewport.revision(), document.revision());
        assert!(
            viewport
                .blocks()
                .iter()
                .filter(|block| span.contains(&block.index()))
                .all(|block| block.height() > 0.0)
        );
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.revision(), Revision::INITIAL);
    }

    #[test]
    fn external_transaction_maps_selection_to_the_new_revision() {
        let mut document = EditorDocument::new("abc");
        let selection = EditorSelection::cursor(
            &document.snapshot(),
            yu_core::ByteOffset::new(1),
            crate::CaretAffinity::Downstream,
        )
        .expect("caret should be valid");
        document
            .set_selection(selection)
            .expect("selection should belong to document");
        let transaction =
            Transaction::new(document.revision(), [Edit::new(source_range(0, 0), "羽")]);

        document
            .apply_transaction(&transaction)
            .expect("external transaction should apply");
        assert_eq!(document.revision(), Revision::new(1));
        assert_eq!(document.selection().focus().get(), "羽a".len() as u64);
    }

    /// 一份装饰藏起来的 source 区间，升序、合并重叠。
    ///
    /// 合并是必须的：多个 extension 可以盖在同一段上（`- [x]` 就有三条互相
    /// 重叠的），不合并会把同一段数几遍。
    fn hidden_spans(decorations: &BlockDecorations) -> Vec<(u64, u64)> {
        let mut spans: Vec<(u64, u64)> = decorations
            .set()
            .all()
            .iter()
            .filter(|entry| entry.decoration.hides_source())
            .map(|entry| (entry.range.start().get(), entry.range.end().get()))
            .filter(|(from, to)| from < to)
            .collect();
        spans.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for (from, to) in spans {
            match merged.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        merged
    }

    /// 这个块投影之后的视觉长度。
    fn visual_len_of(decorations: &BlockDecorations) -> u64 {
        let set = decorations.set();
        let range = decorations.range();
        set.source_to_visual(range.end())
            .get()
            .saturating_sub(set.source_to_visual(range.start()).get())
    }

    /// 露出语法不得往规范缓存里加东西。
    ///
    /// 比的是「产了几份、存着几份、作废了几份」而不是整个计数结构：命中数
    /// 会变（判断该不该露出本身就要问一次规范装饰），那不是污染。
    fn assert_no_new_entries(now: DecorationCacheStats, before: DecorationCacheStats) {
        assert_eq!(now.entries(), before.entries(), "缓存条数变了");
        assert_eq!(now.builds(), before.builds(), "多产了一份规范装饰");
        assert_eq!(now.invalidated(), before.invalidated(), "作废了缓存");
    }

    /// 行首标记的替代文字，没有就是 `None`。
    fn marker_text_of(decorations: &BlockDecorations) -> Option<String> {
        decorations
            .line_styles()
            .iter()
            .find_map(|ornament| match ornament {
                BlockOrnament::Marker(marker) => Some(marker.text().to_owned()),
                _ => None,
            })
    }

    /// **J1 走完整条产品链路**：`apply_transaction` 之后再渲染一次，只重扫被
    /// 改的那个块。
    ///
    /// `decorations.rs` 里那几条直接驱动 `DecorationCache`，压的是缓存自己；
    /// 这一条压的是**接线**——`shift_through` 有没有真的把 `ChangeSet` 交给
    /// 复用来源。接线断了不会报错，只会让每次敲键都整篇重解析。
    #[test]
    fn an_edit_through_the_document_rescans_only_the_block_it_touched() {
        /// 一次单字符编辑允许重扫的字节数。实测约 60（就是被改的那个块）。
        /// 判据是它必须小到让「退化成全量」一定越界：这份语料有三万多字节。
        const BUDGET: u64 = 256;

        let mut source = String::new();
        for index in 0..512 {
            source.push_str(&format!(
                "## Section {index}\n\nParagraph {index} with *emphasis*.\n\n"
            ));
        }
        let mut document = EditorDocument::new(source.clone());
        document.visual_text().expect("初次渲染");
        let full = document.decoration_cache_stats().reparsed_bytes();
        assert!(
            full > 10_000,
            "第一次是全量，只读了 {full} 字节——语料太小，证明不了什么"
        );

        // 语料全是 ASCII，文档正中间一定是字符边界。
        let middle = (source.len() / 2) as u64;
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(source_range(middle, middle), "X")],
        );
        document
            .apply_transaction(&transaction)
            .expect("编辑应当成功");
        document.visual_text().expect("编辑后再渲染一次");

        assert_eq!(
            document.decoration_cache_stats().parses(),
            2,
            "两次渲染各解析一次"
        );
        let rescanned = document.decoration_cache_stats().reparsed_bytes() - full;
        assert!(
            rescanned <= BUDGET,
            "改一个字符重扫了 {rescanned} 字节，超出上界 {BUDGET}（全量是 {full}）"
        );
    }

    #[test]
    fn decoration_cache_reuses_and_remaps_unaffected_blocks() {
        let source = "prefix **羽🙂** suffix";
        let mut document = EditorDocument::new(source);

        {
            let decorations = document.block_decorations(0).expect("装饰应当产得出来");
            assert_eq!(hidden_spans(decorations).len(), 2, "两个 `**`");
        }
        {
            let decorations = document.block_decorations(0).expect("第二次该命中缓存");
            assert_eq!(decorations.revision(), document.revision());
        }
        let stats = document.decoration_cache_stats();
        assert_eq!(stats.entries(), 1);
        assert_eq!(stats.builds(), 1);
        assert_eq!(stats.hits(), 1);

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("前缀编辑应当成功");
        // 编辑落在块**内**（块从 0 开始），所以这一份必须重建。
        assert_eq!(document.decoration_cache_stats().invalidated(), 1);
    }

    /// 编辑落在块外时那些块整体平移，不重建。
    #[test]
    fn decoration_cache_shifts_blocks_that_the_edit_did_not_touch() {
        let source = "intro

prefix **羽🙂** suffix
";
        let mut document = EditorDocument::new(source);
        let index = document
            .markdown()
            .blocks()
            .iter()
            .position(|block| block.range().len() > 10 && block.kind() == BlockKind::Paragraph)
            .expect("段落块");
        let old_range = document
            .block_decorations(index)
            .expect("装饰应当产得出来")
            .range();

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("前缀编辑应当成功");

        let decorations = document
            .block_decorations(index)
            .expect("平移过的装饰应当直接可用")
            .clone();
        assert_eq!(
            decorations.range().start().get(),
            old_range.start().get() + 3
        );
        assert_eq!(decorations.revision(), document.revision());
        assert_eq!(hidden_spans(&decorations).len(), 2);
        let stats = document.decoration_cache_stats();
        assert_eq!(stats.builds(), 1, "平移不重建");
        assert_eq!(stats.remapped(), 1);
    }

    /// 编辑碰到块的边界也算碰到。
    ///
    /// 紧贴块首插入的字符会改变块的语法归属——`#` 打进去就是标题了。沿用
    /// 旧装饰的后果是「多打了一个 `#` 但标题级别没变」，不报错。
    #[test]
    fn decoration_cache_invalidates_a_block_touched_at_its_boundary() {
        let mut document = EditorDocument::new(
            "# heading
",
        );
        document.block_decorations(0).expect("装饰应当产得出来");
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "#")],
        );
        document
            .apply_transaction(&transaction)
            .expect("编辑应当成功");
        assert_eq!(document.decoration_cache_stats().entries(), 0);
        let decorations = document.block_decorations(0).expect("重建");
        assert_eq!(hidden_spans(decorations), vec![(0, 3)], "`## ` 整个隐藏");
    }

    /// 引用式链接的装饰不再依赖 definition 索引。
    ///
    /// `yu-syntax` 给的是结构：`[id]` 的 `LinkLabel` 是树上的节点，隐藏区间
    /// 不需要先查表判断它是不是一个真链接（不变量 C6 说那件事发生在装饰
    /// 阶段，而「解析目标」才需要索引）。所以改一条 definition 不会让别的块
    /// 的装饰作废——它只是把块整体挪了一个字节。
    #[test]
    fn a_definition_edit_only_shifts_unrelated_blocks() {
        let source = "[id]: /docs

[id]
";
        let mut document = EditorDocument::new(source);
        let paragraph = document
            .block_decorations(2)
            .expect("引用式段落的装饰")
            .range();
        assert_eq!(document.decoration_cache_stats().entries(), 1);

        let label_start = source.find("id").expect("definition 的标签");
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(
                source_range(label_start as u64, (label_start + 2) as u64),
                "new",
            )],
        );
        document
            .apply_transaction(&transaction)
            .expect("definition 编辑应当成功");

        let shifted = document.block_decorations(2).expect("平移过的装饰");
        assert_eq!(
            shifted.range(),
            source_range(paragraph.start().get() + 1, paragraph.end().get() + 1)
        );
        assert_eq!(document.decoration_cache_stats().builds(), 1);
        assert_eq!(document.decoration_cache_stats().remapped(), 1);
    }

    #[test]
    fn toggle_task_is_a_source_transaction_and_rebuilds_task_decorations() {
        let mut document = EditorDocument::new("- [ ] todo\n");
        let decorations = document.block_decorations(0).expect("任务项的装饰");
        assert_eq!(hidden_spans(decorations), vec![(2, 5)], "`[ ]` 整个隐藏");
        assert_eq!(document.decoration_cache_stats().builds(), 1);

        let result = document
            .execute(EditorCommand::toggle_task(0))
            .expect("task toggle should apply");
        assert!(result.changed());
        assert_eq!(document.snapshot().as_str(), "- [x] todo\n");
        assert!(matches!(
            document
                .markdown()
                .blocks()
                .get(0)
                .expect("task block")
                .kind(),
            BlockKind::TaskListItem {
                state: yu_markdown::TaskState::Done,
                ..
            }
        ));
        assert_eq!(document.decoration_cache_stats().entries(), 0);

        document
            .toggle_task(0)
            .expect("second task toggle should apply");
        assert_eq!(document.snapshot().as_str(), "- [ ] todo\n");
    }

    #[test]
    fn toggle_task_rejects_non_task_blocks() {
        let mut document = EditorDocument::new("- ordinary\n");
        assert!(matches!(
            document.toggle_task(0),
            Err(EditorDocumentError::BlockNotTaskList { index: 0 })
        ));
    }

    /// 落在字符中间的源码偏移必须被拒绝，不能静默取整
    /// （`docs/specs/coordinates.md`）。
    ///
    /// 装饰集合自己回答不了这个问题——它不持有源码。校验归持有文本的
    /// `VisualText`。
    #[test]
    fn visual_text_rejects_non_utf8_source_boundaries() {
        let mut document = EditorDocument::new("羽");
        let text = document.visual_text().expect("视觉文本");
        assert!(matches!(
            text.source_to_visual(ByteOffset::new(1), Bias::After),
            Err(VisualTextError::SourceNotCharBoundary { .. })
        ));
    }

    #[test]
    fn selection_reveal_is_transient_and_does_not_pollute_revision_caches() {
        let source = "before **strong** after";
        let mut document = EditorDocument::new(source);
        let strong = source.find("strong").expect("strong content");
        let block_index = document
            .block_index_for_source(ByteOffset::new(strong as u64))
            .expect("focus block");
        let canonical = document
            .block_decorations(block_index)
            .expect("canonical decorations")
            .clone();
        let canonical_hidden = hidden_spans(&canonical).len();
        let cached = document.decoration_cache_stats();

        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new((strong + 2) as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("set selection");
        let revealed = document
            .block_decorations_with_selection_reveal(block_index)
            .expect("revealed decorations");

        assert_eq!(canonical_hidden, 2);
        assert!(hidden_spans(&revealed).is_empty());
        assert_eq!(visual_len_of(&revealed), source.len() as u64);
        assert_eq!(document.revision(), Revision::new(0));
        assert_no_new_entries(document.decoration_cache_stats(), cached);
        assert_eq!(document.selection_reveal_block_index(), Some(block_index));

        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(source.len() as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("outside selection"),
            )
            .expect("set outside selection");
        assert_eq!(document.selection_reveal_block_index(), None);
        let hidden_again = document
            .block_decorations_with_selection_reveal(block_index)
            .expect("hidden decorations");
        assert_eq!(hidden_spans(&hidden_again).len(), 2);
    }

    #[test]
    fn structural_prefix_reveal_is_focus_bound_and_source_neutral() {
        let source = "## heading\n\nplain\n\n- item\n";
        let mut document = EditorDocument::new(source);
        let heading = source.find("heading").expect("heading text");
        let heading_index = document
            .block_index_for_source(ByteOffset::new(heading as u64))
            .expect("heading block");
        let canonical = document
            .block_decorations(heading_index)
            .expect("canonical heading decorations")
            .clone();
        let cached = document.decoration_cache_stats();

        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new((heading + 2) as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("heading selection"),
            )
            .expect("set heading selection");
        let revealed = document
            .block_decorations_with_selection_reveal(heading_index)
            .expect("revealed heading decorations");
        let heading_source_len = source.find('\n').expect("heading line ending") as u64 + 1;

        assert_eq!(document.selection_reveal_block_index(), Some(heading_index));
        assert_eq!(visual_len_of(&canonical) + 3, heading_source_len);
        assert_eq!(visual_len_of(&revealed), heading_source_len);
        assert_eq!(document.revision(), Revision::new(0));
        assert_no_new_entries(document.decoration_cache_stats(), cached);

        let plain = source.find("plain").expect("plain text");
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(plain as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("plain selection"),
            )
            .expect("set plain selection");
        assert_eq!(document.selection_reveal_block_index(), None);
        assert_eq!(document.revision(), Revision::new(0));

        let item = source.find("item").expect("list item");
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(item as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("list selection"),
            )
            .expect("set list selection");
        let list_index = document
            .block_index_for_source(ByteOffset::new(item as u64))
            .expect("list block");
        assert_eq!(document.selection_reveal_block_index(), Some(list_index));
        let canonical_list = document
            .block_decorations(list_index)
            .expect("canonical list decorations")
            .clone();
        let list_cached = document.decoration_cache_stats();
        assert_eq!(marker_text_of(&canonical_list).as_deref(), Some("\u{2022}"));
        let revealed_list = document
            .block_decorations_with_selection_reveal(list_index)
            .expect("revealed list decorations");
        assert!(marker_text_of(&revealed_list).is_none());
        assert_eq!(
            visual_len_of(&revealed_list),
            visual_len_of(&canonical_list) + 2
        );
        assert_eq!(document.revision(), Revision::new(0));
        assert_no_new_entries(document.decoration_cache_stats(), list_cached);
    }

    #[test]
    fn block_decorations_use_incremental_markdown_ranges_and_remap_prefix_edits() {
        let source = "intro\n\nparagraph **羽🙂**\n\n```rust\ncode\n```\n";
        let mut document = EditorDocument::new(source);
        let paragraph_index = document
            .markdown()
            .blocks()
            .iter()
            .position(|block| block.kind() == BlockKind::Paragraph && block.range().len() > 10)
            .expect("paragraph block should exist");
        let old_range = document
            .markdown()
            .blocks()
            .get(paragraph_index)
            .expect("paragraph block should be present")
            .range();

        {
            let decorations = document
                .block_decorations(paragraph_index)
                .expect("paragraph decorations should build");
            assert_eq!(decorations.range(), old_range);
            assert_eq!(hidden_spans(decorations).len(), 2);
        }
        assert_eq!(document.markdown().revision(), document.revision());
        assert_eq!(document.decoration_cache_stats().builds(), 1);

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("prefix edit should apply");
        let new_block = document
            .markdown()
            .blocks()
            .get(paragraph_index)
            .expect("paragraph block should remain at the same index");
        let new_range = new_block.range();
        assert_eq!(new_range.start().get(), old_range.start().get() + 3);
        assert_eq!(new_range.end().get(), old_range.end().get() + 3);
        let decorations = document
            .block_decorations(paragraph_index)
            .expect("remapped paragraph decorations should be reusable");
        assert_eq!(decorations.range(), new_range);
        assert_eq!(document.decoration_cache_stats().remapped(), 1);
        assert_eq!(document.decoration_cache_stats().builds(), 1);
    }

    /// 围栏代码块把围栏那两行整个藏起来，内容按等宽排。
    ///
    /// **内容里的 `**` 不解析**——树里 `FencedCode` 的内容是一个 `CodeText`
    /// 叶子，遍历不到就产不出装饰。v1 需要一个专门的 `CodeProjection` 来
    /// 保证这件事。
    #[test]
    fn a_fenced_code_block_hides_its_fences_and_keeps_its_body_literal() {
        let mut document = EditorDocument::new("```rust\n**code**\n```\n");
        {
            let decorations = document.block_decorations(0).expect("围栏代码块的装饰");
            assert_eq!(
                hidden_spans(decorations),
                vec![(0, 8), (17, 21)],
                "开围栏连语言名与换行符，收尾围栏连它的换行符"
            );
        }
        assert_eq!(document.decoration_cache_stats().entries(), 1);
        assert!(matches!(
            document.block_decorations(1),
            Err(EditorDocumentError::BlockOutOfBounds { index: 1, .. })
        ));
    }

    #[test]
    fn cached_code_decorations_remap_when_a_prefix_edit_shifts_the_block() {
        let mut document = EditorDocument::new("intro\n\n```rust\n**code**\n```\n");
        let old_hidden = hidden_spans(document.block_decorations(2).expect("围栏代码块的装饰"));
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("prefix edit should apply");

        let new_hidden = hidden_spans(
            document
                .block_decorations(2)
                .expect("平移过的装饰应当直接可用"),
        );
        assert_eq!(
            new_hidden,
            old_hidden
                .iter()
                .map(|(from, to)| (from + 3, to + 3))
                .collect::<Vec<_>>()
        );
        assert_eq!(document.decoration_cache_stats().builds(), 1);
        assert_eq!(document.decoration_cache_stats().remapped(), 1);
    }

    #[test]
    fn block_layout_uses_the_current_projection_revision() {
        let mut document = EditorDocument::new("**羽🙂**");
        let revision = document.revision();
        let layout = document
            .block_layout(0, LayoutConfig::new(2.0, 1.25))
            .expect("block layout should build");

        assert_eq!(layout.revision(), revision);
        assert_eq!(layout.lines().len(), 1);
        assert_eq!(layout.lines()[0].width(), 2.0);
        assert_eq!(layout.clusters().len(), 2);
        assert_eq!(document.layout_cache_stats().builds(), 1);
        document
            .block_layout(0, LayoutConfig::new(2.0, 1.25))
            .expect("same layout should hit cache");
        assert_eq!(document.layout_cache_stats().hits(), 1);
    }

    #[test]
    fn block_layout_cache_separates_metrics_and_shaped_backends() {
        let mut document = EditorDocument::new("ab");
        let config = LayoutConfig::new(3.0, 1.0);
        let metrics = document
            .block_layout(0, config)
            .expect("metrics layout should build");
        assert_eq!(metrics.lines().len(), 1);
        assert_eq!(metrics.lines()[0].width(), 2.0);

        let shaper = WideShaper;
        let shaped = document
            .block_layout_with_shaper(0, config, &shaper)
            .expect("shaped layout should build");
        assert_eq!(shaped.lines().len(), 2);
        assert_eq!(shaped.lines()[0].width(), 2.0);
        assert_eq!(document.layout_cache_stats().entries(), 2);

        document
            .block_layout_with_shaper(0, config, &shaper)
            .expect("same shaped layout should hit cache");
        document
            .block_layout(0, config)
            .expect("metrics layout should remain independently cached");
        assert!(document.layout_cache_stats().hits() >= 2);

        document.clear_layout_state();
        assert_eq!(document.layout_cache_stats().entries(), 0);
        assert_eq!(document.viewport_stats().entries(), 0);
    }

    #[test]
    fn layout_cache_remaps_unaffected_blocks_and_keys_config() {
        let mut document = EditorDocument::new("intro\n\n**羽🙂**");
        let config = LayoutConfig::new(2.0, 1.25);
        let old_range = document
            .block_layout(2, config)
            .expect("block layout should build")
            .source_range();
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("prefix edit should apply");

        let mapped_range = document
            .block_layout(2, config)
            .expect("unaffected block layout should remap")
            .source_range();
        assert_eq!(mapped_range.start().get(), old_range.start().get() + 3);
        assert_eq!(document.layout_cache_stats().builds(), 1);
        assert_eq!(document.layout_cache_stats().remapped(), 1);

        document
            .block_layout(2, LayoutConfig::new(4.0, 1.25))
            .expect("different width should build a separate layout");
        assert_eq!(document.layout_cache_stats().builds(), 2);
        assert_eq!(document.layout_cache_stats().entries(), 2);
    }

    #[test]
    fn layout_cache_is_dropped_when_block_kind_changes() {
        let mut document = EditorDocument::new("paragraph **羽**");
        document
            .block_layout(0, LayoutConfig::default())
            .expect("paragraph layout should build");
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "# ")],
        );
        document
            .apply_transaction(&transaction)
            .expect("heading edit should apply");
        assert_eq!(document.layout_cache_stats().entries(), 0);
    }

    #[test]
    fn viewport_measures_only_the_requested_window_and_reuses_layouts() {
        let mut document = EditorDocument::new("a\n\nb\n\nc\n\nd\n\ne\n\nf\n\ng");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");
        let first = document
            .visible_blocks(ViewportSpan::new(0.0, 0.5))
            .expect("first viewport should measure");
        assert_eq!(first.revision(), document.revision());
        assert_eq!(first.blocks().len(), 1);
        assert_eq!(first.blocks()[0].index(), 0);
        assert!(first.blocks()[0].is_measured());
        assert!(document.viewport_stats().measured() < document.markdown().blocks().len());
        assert_eq!(document.layout_cache_stats().builds(), 1);

        document
            .visible_blocks(ViewportSpan::new(0.0, 0.5))
            .expect("repeated viewport should hit layout cache");
        assert_eq!(document.layout_cache_stats().builds(), 1);
        assert!(document.layout_cache_stats().hits() >= 1);

        let last = document
            .visible_blocks(ViewportSpan::new(100.0, 0.5))
            .expect("far viewport should measure only its block");
        assert!(last.blocks().iter().all(|block| block.index() > 0));
        assert!(document.layout_cache_stats().builds() < document.markdown().blocks().len() as u64);
    }

    #[test]
    fn viewport_state_stays_lazy_until_first_query() {
        let source = "paragraph\n\n".repeat(128);
        let mut document = EditorDocument::new(source);
        assert_eq!(document.viewport_stats().entries(), 0);

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "prefix ")],
        );
        document
            .apply_transaction(&transaction)
            .expect("edit should not materialize viewport state");

        assert_eq!(document.viewport_stats().entries(), 0);
        assert_eq!(document.viewport_stats().remapped(), 0);

        let snapshot = document
            .visible_blocks(ViewportSpan::new(0.0, 1.0))
            .expect("first viewport query should materialize block state");
        assert!(!snapshot.blocks().is_empty());
        assert_eq!(
            document.viewport_stats().entries(),
            document.markdown().blocks().len()
        );
    }

    #[test]
    fn viewport_remeasures_when_switching_to_shaped_backend() {
        let mut document = EditorDocument::new("ab");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(3.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");

        let metrics = document
            .visible_blocks(ViewportSpan::new(0.0, 2.0))
            .expect("metrics viewport should measure");
        assert_eq!(metrics.blocks()[0].height(), 1.0);

        let shaped = document
            .visible_blocks_with_shaper(ViewportSpan::new(0.0, 2.0), &WideShaper)
            .expect("shaped viewport should measure");
        assert_eq!(shaped.blocks()[0].height(), 2.0);
        assert_eq!(shaped.content_height(), 2.0);

        let metrics_again = document
            .visible_blocks(ViewportSpan::new(0.0, 2.0))
            .expect("metrics viewport should remeasure after backend switch");
        assert_eq!(metrics_again.blocks()[0].height(), 1.0);
    }

    /// 资源就绪之后受影响的块重排一次（不变量 D7 的后半句）。
    ///
    /// 缓存不按图片建键：图片就绪与否不是块的身份。命中之后由
    /// `needs_widget_rebuild` 判——判错的方向只有一个坏处大：判「不用重排」
    /// 时图片解码完了画面不变，不报错，只是永远看不到图。
    #[test]
    fn a_ready_image_rebuilds_the_cached_layout() {
        let mut document = EditorDocument::new("![alt](image.png)");
        let config = LayoutConfig::new(80.0, 10.0);
        let placeholder = document
            .block_layout(0, config)
            .expect("placeholder layout")
            .images()[0]
            .bounds();
        assert_eq!(document.layout_cache_stats().builds(), 1);

        let intrinsic = ImageIntrinsicSize::new(200, 100).expect("image dimensions");
        let sizes = document
            .block_image_sizes(0, &|_| Some(intrinsic))
            .expect("image sizes");
        let ready = document
            .block_layout_with_images(0, config, &sizes)
            .expect("ready layout")
            .images()[0]
            .bounds();
        assert_eq!(document.layout_cache_stats().builds(), 2, "就绪要重排一次");
        assert_ne!(placeholder.height(), ready.height());
        assert_eq!(ready.height(), 40.0);
    }

    /// 不带尺寸表的调用方不会把带尺寸的那一份挤掉。
    ///
    /// 命中测试、Accessibility 与纯度量排版都不关心图片解码没有，传的是空
    /// 表。按尺寸表建键的话它们每帧都会把就绪的那一份换成 placeholder，
    /// 然后下一帧再换回来——图片一直在闪，而两份都是「对的」。
    #[test]
    fn a_query_without_image_sizes_keeps_the_ready_layout() {
        let mut document = EditorDocument::new("![alt](image.png)");
        let config = LayoutConfig::new(80.0, 10.0);
        let intrinsic = ImageIntrinsicSize::new(200, 100).expect("image dimensions");
        let sizes = document
            .block_image_sizes(0, &|_| Some(intrinsic))
            .expect("image sizes");
        document
            .block_layout_with_images(0, config, &sizes)
            .expect("ready layout");
        let builds = document.layout_cache_stats().builds();

        let again = document
            .block_layout(0, config)
            .expect("layout without sizes")
            .images()[0]
            .bounds();
        assert_eq!(document.layout_cache_stats().builds(), builds);
        assert_eq!(again.height(), 40.0);
    }

    #[test]
    fn ready_image_intrinsic_height_updates_block_index_and_content_height() {
        let mut document = EditorDocument::new("![alt](image.png)\n\ntext");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(80.0, 10.0),
                10.0,
                0.0,
            ))
            .expect("viewport config should be valid");

        let placeholder = document
            .visible_blocks_with_shaper(ViewportSpan::new(0.0, 100.0), &WideShaper)
            .expect("placeholder viewport should measure");
        assert_eq!(placeholder.blocks()[0].height(), 20.0);

        let intrinsic = ImageIntrinsicSize::new(200, 100).expect("image dimensions");
        let ready = document
            .visible_blocks_with_shaper_and_image_resolver(
                ViewportSpan::new(0.0, 100.0),
                &WideShaper,
                |_| Some(intrinsic),
            )
            .expect("ready image viewport should measure");
        // 40 是图片那一行（200×100 缩到 80 宽就是 40 高），10 是块尾那个
        // 换行符自己的行。图片 widget 化之前这里是 40：图片是排完之后另贴
        // 上去的盒子，行不知道它有多高，块高只能取 `max(行盒累加, 图片下
        // 沿)`——于是图片压在块尾那一行上面。
        assert_eq!(ready.blocks()[0].height(), 50.0);
        assert_eq!(ready.blocks()[1].y(), 50.0);
        assert!(ready.content_height() > placeholder.content_height());
        assert!(ready.content_height() >= ready.blocks()[1].y() + ready.blocks()[1].height());
    }

    #[test]
    fn caret_scroll_request_reveals_focus_and_is_noop_when_visible() {
        let source = "one\n\ntwo\n\nthree";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");
        set_caret(&mut document, source.len());

        let request = document
            .caret_scroll_request(ViewportSpan::new(0.0, 1.0), 0.0)
            .expect("caret scroll request should resolve");
        assert_eq!(request.revision(), document.revision());
        assert_eq!(request.caret().source().get(), source.len() as u64);
        assert_eq!(request.caret().block(), 4);
        assert_eq!(request.caret().y(), 4.0);
        assert!(request.needs_scroll());
        assert_eq!(request.target_scroll_y(), 4.0);

        let visible = document
            .caret_scroll_request(ViewportSpan::new(request.target_scroll_y(), 1.0), 0.0)
            .expect("visible caret request should resolve");
        assert!(!visible.needs_scroll());
        assert_eq!(visible.target_scroll_y(), request.target_scroll_y());

        set_caret(&mut document, 0);
        let reveal_top = document
            .caret_scroll_request(ViewportSpan::new(request.target_scroll_y(), 1.0), 0.0)
            .expect("top caret request should resolve");
        assert!(reveal_top.needs_scroll());
        assert_eq!(reveal_top.target_scroll_y(), 0.0);
    }

    #[test]
    fn caret_scroll_request_rejects_invalid_margin() {
        let mut document = EditorDocument::new("text");
        assert_eq!(
            document.caret_scroll_request(ViewportSpan::new(0.0, 1.0), -1.0),
            Err(EditorDocumentError::Viewport(ViewportError::InvalidMargin))
        );
        assert!(matches!(
            document.caret_scroll_request(ViewportSpan::new(0.0, 1.0), f32::NAN),
            Err(EditorDocumentError::Viewport(ViewportError::InvalidMargin))
        ));
    }

    #[test]
    fn viewport_preserves_unaffected_measurements_through_prefix_edits() {
        let mut document = EditorDocument::new("a\n\nb\n\nc\n\nd");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");
        document
            .visible_blocks(ViewportSpan::new(100.0, 0.5))
            .expect("last block should be measured");
        let measured_before = document.viewport_stats().measured();
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("prefix edit should apply");

        assert!(document.viewport_stats().remapped() >= 1);
        assert_eq!(document.viewport_stats().measured(), measured_before);
        let visible = document
            .visible_blocks(ViewportSpan::new(100.0, 0.5))
            .expect("mapped viewport should remain queryable");
        assert_eq!(visible.revision(), document.revision());
        assert!(visible.blocks().iter().all(|block| block.index() > 0));
    }

    #[test]
    fn viewport_invalidates_a_block_when_its_kind_changes() {
        let mut document = EditorDocument::new("paragraph\n\nother");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");
        document
            .visible_blocks(ViewportSpan::new(0.0, 0.5))
            .expect("first block should be measured");
        let invalidated_before = document.viewport_stats().invalidated();
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "# ")],
        );
        document
            .apply_transaction(&transaction)
            .expect("heading edit should apply");
        assert!(document.viewport_stats().invalidated() > invalidated_before);
        let visible = document
            .visible_blocks(ViewportSpan::new(0.0, 0.5))
            .expect("new heading block should be queryable");
        assert_eq!(visible.revision(), document.revision());
    }

    #[test]
    fn block_projection_is_dropped_when_block_kind_changes() {
        let mut document = EditorDocument::new("paragraph **羽**\n");
        document
            .block_decorations(0)
            .expect("paragraph projection should build");

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "# ")],
        );
        document
            .apply_transaction(&transaction)
            .expect("heading edit should apply");
        assert_eq!(
            document
                .markdown()
                .blocks()
                .get(0)
                .expect("block exists")
                .kind(),
            BlockKind::AtxHeading { level: 1 }
        );
        assert_eq!(document.decoration_cache_stats().entries(), 0);
        document
            .block_decorations(0)
            .expect("heading projection should build independently");
        assert_eq!(document.decoration_cache_stats().builds(), 2);
    }

    #[test]
    fn selection_from_an_old_revision_cannot_be_set() {
        let mut document = EditorDocument::new("old");
        let old_selection = document.selection();
        document
            .execute(EditorCommand::insert_text("new"))
            .expect("insert should succeed");

        assert!(matches!(
            document.set_selection(old_selection),
            Err(SelectionError::StaleRevision { .. })
        ));
    }

    #[test]
    fn stale_commit_keeps_overlay_until_platform_cancels() {
        let mut document = EditorDocument::new("hello");
        document
            .begin_composition(source_range(5, 5), "yu", utf16_range(2, 2))
            .expect("composition should begin");
        let transaction =
            Transaction::new(document.revision(), [Edit::new(source_range(0, 0), "!")]);
        document
            .apply_transaction(&transaction)
            .expect("unrelated edit should apply");

        assert!(matches!(
            document.commit_composition("羽"),
            Err(EditorDocumentError::Edit(EditError::StaleRevision { .. }))
        ));
        assert!(document.composition().is_some());
        assert!(document.cancel_composition());
        assert!(!document.cancel_composition());
    }

    #[test]
    fn reset_source_is_rejected_while_composing() {
        let mut document = EditorDocument::new("old");
        document
            .begin_composition(source_range(3, 3), "x", utf16_range(1, 1))
            .expect("composition should begin");
        assert_eq!(
            document.reset_source("new"),
            Err(EditorDocumentError::CompositionActive)
        );
        let _ = document.cancel_composition();
        document
            .reset_source("new")
            .expect("reset should work after cancellation");
        assert_eq!(document.revision(), Revision::INITIAL);
        assert_eq!(document.snapshot().as_str(), "new");
    }
}
