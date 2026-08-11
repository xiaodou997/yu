use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::{LineIndex, Revision, TextRange, Utf16Range};
use yu_layout::{LayoutConfig, LayoutError, LayoutSnapshot, ShapingProvider};
use yu_markdown::{BlockKind, IncrementalParseError, MarkdownDocument, TaskState};
use yu_text::{
    AppliedTransaction, EditError, TextBuffer, TextPositionError, TextSnapshot, Transaction,
};

use crate::{
    BlockProjection, CommandResult, CompositionError, CompositionOverlay, EditorCommand,
    EditorSelection, LayoutBackend, LayoutCache, LayoutCacheStats, Projection, ProjectionCache,
    ProjectionCacheStats, ProjectionError, SelectionError, ViewportConfig, ViewportError,
    ViewportLayout, ViewportRect, ViewportSnapshot, ViewportStats,
    command::{next_grapheme_boundary, previous_grapheme_boundary},
    history::{EditorHistory, HistoryEntry, HistoryGroup, HistoryStats},
    list::ListLinePrefix,
};

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
    history: EditorHistory,
    projections: ProjectionCache,
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
            snapshot.len_bytes(),
            crate::CaretAffinity::Downstream,
        )
        .expect("the end of a newly created source is a valid caret");
        Self {
            buffer,
            markdown,
            composition: None,
            selection,
            history: EditorHistory::default(),
            projections: ProjectionCache::default(),
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

    /// Returns the source-backed projection for a range in the current
    /// revision, building it on first use and reusing it on later queries.
    pub fn projection(&mut self, range: TextRange) -> Result<&Projection, EditorDocumentError> {
        let snapshot = self.snapshot();
        self.projections
            .get_or_build_with_definitions(&snapshot, range, self.markdown.reference_definitions())
            .map_err(EditorDocumentError::Projection)
    }

    #[must_use]
    pub fn projection_cache_stats(&self) -> ProjectionCacheStats {
        self.projections.stats()
    }

    /// Returns the projection for one parser-owned Markdown block.
    pub fn block_projection(
        &mut self,
        index: usize,
    ) -> Result<&BlockProjection, EditorDocumentError> {
        let block =
            self.markdown
                .blocks()
                .get(index)
                .ok_or(EditorDocumentError::BlockOutOfBounds {
                    index,
                    blocks: self.markdown.blocks().len(),
                })?;
        let snapshot = self.snapshot();
        self.projections
            .get_or_build_block_with_definitions(
                &snapshot,
                block,
                self.markdown.reference_definitions(),
            )
            .map_err(EditorDocumentError::Projection)
    }

    /// Returns a revision-bound block layout snapshot from the current
    /// projection. The snapshot is owned by a cache keyed by block range,
    /// block kind and layout configuration; source edits remap unaffected
    /// entries and invalidate entries whose projection was touched.
    pub fn block_layout(
        &mut self,
        index: usize,
        config: LayoutConfig,
    ) -> Result<&LayoutSnapshot, EditorDocumentError> {
        let block =
            self.markdown
                .blocks()
                .get(index)
                .ok_or(EditorDocumentError::BlockOutOfBounds {
                    index,
                    blocks: self.markdown.blocks().len(),
                })?;
        let snapshot = self.snapshot();
        let projection = self
            .projections
            .get_or_build_block_with_definitions(
                &snapshot,
                block,
                self.markdown.reference_definitions(),
            )
            .map_err(EditorDocumentError::Projection)?;
        self.layouts
            .get_or_build_block(&snapshot, block, config, projection)
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
    ) -> Result<&LayoutSnapshot, EditorDocumentError> {
        let block =
            self.markdown
                .blocks()
                .get(index)
                .ok_or(EditorDocumentError::BlockOutOfBounds {
                    index,
                    blocks: self.markdown.blocks().len(),
                })?;
        let snapshot = self.snapshot();
        let projection = self
            .projections
            .get_or_build_block_with_definitions(
                &snapshot,
                block,
                self.markdown.reference_definitions(),
            )
            .map_err(EditorDocumentError::Projection)?;
        self.layouts
            .get_or_build_block_with_shaper(&snapshot, block, config, projection, shaper)
            .map_err(EditorDocumentError::Layout)
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
        viewport: ViewportRect,
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
        viewport: ViewportRect,
        shaper: &S,
    ) -> Result<ViewportSnapshot, EditorDocumentError> {
        let mut layout = std::mem::take(&mut self.viewport);
        let result = self.measure_visible_blocks_with_shaper(&mut layout, viewport, shaper);
        self.viewport = layout;
        result
    }

    fn measure_visible_blocks(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportRect,
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

    fn measure_visible_blocks_with_shaper<S: ShapingProvider>(
        &mut self,
        layout: &mut ViewportLayout,
        viewport: ViewportRect,
        shaper: &S,
    ) -> Result<ViewportSnapshot, EditorDocumentError> {
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
                let line_count = self
                    .block_layout_with_shaper(index, config, shaper)?
                    .lines()
                    .len();
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

    /// Replaces the selection after checking that it belongs to this revision.
    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), SelectionError> {
        selection.utf16_range(&self.snapshot())?;
        self.selection = selection;
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
        let applied = self.buffer.apply(transaction)?;
        let incremental = yu_markdown::parse_incremental(
            &self.markdown,
            applied.result_snapshot(),
            applied.change_set(),
        )?;
        let definitions_changed = self.markdown.reference_definitions().fingerprint()
            != incremental.document().reference_definitions().fingerprint();
        self.selection = self
            .selection
            .map_through(applied.change_set(), applied.result_snapshot())?;
        if definitions_changed {
            self.projections.clear();
            self.layouts.clear();
            self.viewport.clear();
        } else {
            self.projections
                .map_through(applied.change_set(), applied.result_snapshot())
                .map_err(EditorDocumentError::Projection)?;
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
        }
        self.projections.retain_blocks(incremental.document());
        self.layouts.retain_blocks(incremental.document());
        self.markdown = incremental.into_document();
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
        self.history.break_group();
        Ok(applied)
    }

    /// Drops the active overlay without changing source or revision.
    #[must_use]
    pub fn cancel_composition(&mut self) -> bool {
        let cancelled = self.composition.take().is_some();
        if cancelled {
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
        self.projections.clear();
        self.layouts.clear();
        self.viewport.clear();
        self.history.clear();
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
        match command {
            EditorCommand::InsertText(text) => self.insert_text(text),
            EditorCommand::DeleteBackward => self.delete_backward(),
            EditorCommand::DeleteForward => self.delete_forward(),
            EditorCommand::MoveLeft => self.move_left(),
            EditorCommand::MoveRight => self.move_right(),
            EditorCommand::InsertNewline => self.insert_newline(),
            EditorCommand::IndentList => self.indent_list(),
            EditorCommand::OutdentList => self.outdent_list(),
            EditorCommand::Undo => self.undo(),
            EditorCommand::Redo => self.redo(),
            EditorCommand::ToggleTask { block } => self.toggle_task(block),
        }
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
        Ok(self.command_result(true))
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
        Ok(self.command_result(true))
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

    fn command_result(&self, changed: bool) -> CommandResult {
        CommandResult::new(self.revision(), self.selection, changed)
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
}

struct SourceLine {
    start: yu_core::ByteOffset,
    content_end: yu_core::ByteOffset,
    content: String,
    terminator: String,
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
    Projection(ProjectionError),
    Selection(SelectionError),
    Viewport(ViewportError),
    BlockOutOfBounds { index: usize, blocks: usize },
    BlockNotTaskList { index: usize },
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
            Self::Projection(error) => error.fmt(formatter),
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
            Self::Projection(error) => Some(error),
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

impl From<ProjectionError> for EditorDocumentError {
    fn from(error: ProjectionError) -> Self {
        Self::Projection(error)
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
    use crate::VisualRunKind;
    use unicode_segmentation::UnicodeSegmentation;
    use yu_core::{ByteOffset, Utf16Offset};
    use yu_layout::{
        FontFaceId, Glyph, GlyphId, GlyphRun, Script, ShapedText, ShapingProvider, TextDirection,
    };
    use yu_markdown::BlockKind;
    use yu_projection::VisualRunStyle;
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

    #[derive(Clone, Copy, Debug)]
    struct WideShaper;

    impl ShapingProvider for WideShaper {
        type Error = &'static str;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: VisualRunStyle,
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
        assert_eq!(document.snapshot().as_str(), "羽e\u{301}x");
        assert_eq!(document.selection().focus().get(), "羽".len() as u64);

        document
            .execute(EditorCommand::DeleteBackward)
            .expect("backspace should remove one grapheme");
        assert_eq!(document.snapshot().as_str(), "e\u{301}x");
        assert_eq!(document.revision(), Revision::new(2));

        document
            .execute(EditorCommand::MoveRight)
            .expect("right should move over one grapheme");
        document
            .execute(EditorCommand::DeleteForward)
            .expect("forward delete should remove x");
        assert_eq!(document.snapshot().as_str(), "e\u{301}");
        assert_eq!(document.revision(), Revision::new(3));
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

    #[test]
    fn projection_cache_reuses_and_remaps_unaffected_ranges() {
        let source = "prefix **羽🙂** suffix";
        let mut document = EditorDocument::new(source);
        let start = source.find("**").expect("strong delimiter should exist");
        let end = start + "**羽🙂**".len();
        let range = source_range(start as u64, end as u64);

        {
            let projection = document.projection(range).expect("projection should build");
            assert_eq!(projection.visual_len().get(), "羽🙂".len() as u64);
        }
        {
            let projection = document
                .projection(range)
                .expect("projection should be cached");
            assert_eq!(projection.revision(), document.revision());
        }
        let stats = document.projection_cache_stats();
        assert_eq!(stats.entries(), 1);
        assert_eq!(stats.builds(), 1);
        assert_eq!(stats.hits(), 1);

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("prefix edit should apply");
        let shifted = source_range((start + "前".len()) as u64, (end + "前".len()) as u64);
        let projection = document
            .projection(shifted)
            .expect("unaffected projection should be remapped");
        assert_eq!(projection.visual_len().get(), "羽🙂".len() as u64);
        let stats = document.projection_cache_stats();
        assert_eq!(stats.remapped(), 1);
        assert_eq!(stats.builds(), 1);
        assert_eq!(stats.entries(), 1);
    }

    #[test]
    fn projection_cache_invalidates_intersecting_ranges() {
        let source = "prefix **羽🙂** suffix";
        let mut document = EditorDocument::new(source);
        let start = source.find("**").expect("strong delimiter should exist");
        let end = start + "**羽🙂**".len();
        let range = source_range(start as u64, end as u64);
        document.projection(range).expect("projection should build");

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(
                TextRange::empty(ByteOffset::new((start + 2) as u64)),
                "x",
            )],
        );
        document
            .apply_transaction(&transaction)
            .expect("inside edit should apply");
        let stats = document.projection_cache_stats();
        assert_eq!(stats.entries(), 0);
        assert_eq!(stats.invalidated(), 1);

        document
            .projection(range)
            .expect("projection should rebuild for the new revision");
        assert_eq!(document.projection_cache_stats().builds(), 2);
    }

    #[test]
    fn definition_index_changes_invalidate_nonlocal_projections() {
        let source = "[id]: /docs\n\n[id]\n";
        let mut document = EditorDocument::new(source);
        let paragraph = document
            .markdown()
            .blocks()
            .get(2)
            .expect("paragraph should exist")
            .range();
        document
            .block_projection(2)
            .expect("shortcut projection should build");
        assert_eq!(document.projection_cache_stats().entries(), 1);

        let label_start = source.find("id").expect("definition label should exist");
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(
                source_range(label_start as u64, (label_start + 2) as u64),
                "new",
            )],
        );
        document
            .apply_transaction(&transaction)
            .expect("definition edit should apply");

        assert_eq!(
            document
                .markdown()
                .reference_definitions()
                .definitions()
                .len(),
            1
        );
        assert_eq!(document.projection_cache_stats().entries(), 0);
        let new_paragraph = document
            .markdown()
            .blocks()
            .get(2)
            .expect("paragraph should remain")
            .range();
        assert_eq!(
            new_paragraph,
            source_range(paragraph.start().get() + 1, paragraph.end().get() + 1)
        );
        document
            .block_projection(2)
            .expect("unresolved shortcut should rebuild as literal inline text");
        assert_eq!(document.projection_cache_stats().builds(), 2);
    }

    #[test]
    fn definition_index_fingerprint_allows_prefix_remapping() {
        let source = "intro\n\n[id]: /docs\n\n[id]\n";
        let mut document = EditorDocument::new(source);
        let old_range = document
            .markdown()
            .blocks()
            .get(4)
            .expect("paragraph should exist")
            .range();
        document
            .block_projection(4)
            .expect("shortcut projection should build");

        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("prefix edit should apply");
        let shifted_range = document
            .markdown()
            .blocks()
            .get(4)
            .expect("shifted paragraph should exist")
            .range();
        assert_eq!(shifted_range.start().get(), old_range.start().get() + 3);
        document
            .block_projection(4)
            .expect("prefix shift should reuse shortcut projection");
        assert_eq!(document.projection_cache_stats().builds(), 1);
        assert_eq!(document.projection_cache_stats().remapped(), 1);
    }

    #[test]
    fn toggle_task_is_a_source_transaction_and_rebuilds_task_projection() {
        let mut document = EditorDocument::new("- [ ] todo\n");
        let projection = document
            .block_projection(0)
            .expect("task projection should build");
        assert_eq!(projection.kind(), crate::BlockProjectionKind::TaskList);
        assert_eq!(document.projection_cache_stats().builds(), 1);

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
        assert_eq!(document.projection_cache_stats().entries(), 0);

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

    #[test]
    fn projection_query_rejects_non_utf8_source_boundaries() {
        let mut document = EditorDocument::new("羽");
        let invalid = source_range(1, 3);
        assert!(matches!(
            document.projection(invalid),
            Err(EditorDocumentError::Projection(
                ProjectionError::InlineParse(_)
            ))
        ));
        assert_eq!(document.projection_cache_stats().entries(), 0);
    }

    #[test]
    fn block_projection_uses_incremental_markdown_ranges_and_remaps_prefix_edits() {
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
            let projection = document
                .block_projection(paragraph_index)
                .expect("paragraph projection should build");
            assert_eq!(projection.source_range(), old_range);
            assert_eq!(
                projection
                    .visual()
                    .runs()
                    .iter()
                    .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                    .count(),
                2
            );
        }
        assert_eq!(document.markdown().revision(), document.revision());
        assert_eq!(document.projection_cache_stats().builds(), 1);

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
        let projection = document
            .block_projection(paragraph_index)
            .expect("remapped paragraph projection should be reusable");
        assert_eq!(projection.source_range(), new_range);
        assert_eq!(document.projection_cache_stats().remapped(), 1);
        assert_eq!(document.projection_cache_stats().builds(), 1);
    }

    #[test]
    fn fenced_code_blocks_use_the_independent_code_projection() {
        let mut document = EditorDocument::new("```rust\n**code**\n```\n");
        {
            let projection = document
                .block_projection(0)
                .expect("fenced code projection should build");
            match projection {
                BlockProjection::FencedCode(code) => {
                    assert_eq!(code.marker(), '`');
                    assert!(code.closed());
                    assert_eq!(code.visual().visual_len().get(), code.content().len());
                    assert!(code.visual().runs().iter().any(|run| {
                        run.kind() == VisualRunKind::Visible
                            && run.style() == crate::VisualRunStyle::Code
                    }));
                }
                BlockProjection::Inline(_)
                | BlockProjection::ReferenceDefinition(_)
                | BlockProjection::TaskList(_) => {
                    panic!("fenced code must not use inline projection")
                }
            }
        }
        assert_eq!(
            document.projection_cache_stats().entries(),
            1,
            "code projection should be cached by block key"
        );
        assert!(matches!(
            document.block_projection(1),
            Err(EditorDocumentError::BlockOutOfBounds { index: 1, .. })
        ));
    }

    #[test]
    fn cached_code_projection_remaps_when_a_prefix_edit_shifts_the_block() {
        let mut document = EditorDocument::new("intro\n\n```rust\n**code**\n```\n");
        let old_content = match document
            .block_projection(2)
            .expect("fenced code projection should build")
        {
            BlockProjection::FencedCode(code) => code.content(),
            BlockProjection::Inline(_)
            | BlockProjection::ReferenceDefinition(_)
            | BlockProjection::TaskList(_) => {
                panic!("fenced code must use code projection")
            }
        };
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        document
            .apply_transaction(&transaction)
            .expect("prefix edit should apply");

        let projection = document
            .block_projection(2)
            .expect("shifted code projection should be reusable");
        let new_content = match projection {
            BlockProjection::FencedCode(code) => code.content(),
            BlockProjection::Inline(_)
            | BlockProjection::ReferenceDefinition(_)
            | BlockProjection::TaskList(_) => {
                panic!("fenced code must use code projection")
            }
        };
        assert_eq!(new_content.start().get(), old_content.start().get() + 3);
        assert_eq!(new_content.end().get(), old_content.end().get() + 3);
        assert_eq!(document.projection_cache_stats().builds(), 1);
        assert_eq!(document.projection_cache_stats().remapped(), 1);
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
            .visible_blocks(ViewportRect::new(0.0, 0.5))
            .expect("first viewport should measure");
        assert_eq!(first.revision(), document.revision());
        assert_eq!(first.blocks().len(), 1);
        assert_eq!(first.blocks()[0].index(), 0);
        assert!(first.blocks()[0].is_measured());
        assert!(document.viewport_stats().measured() < document.markdown().blocks().len());
        assert_eq!(document.layout_cache_stats().builds(), 1);

        document
            .visible_blocks(ViewportRect::new(0.0, 0.5))
            .expect("repeated viewport should hit layout cache");
        assert_eq!(document.layout_cache_stats().builds(), 1);
        assert!(document.layout_cache_stats().hits() >= 1);

        let last = document
            .visible_blocks(ViewportRect::new(100.0, 0.5))
            .expect("far viewport should measure only its block");
        assert!(last.blocks().iter().all(|block| block.index() > 0));
        assert!(document.layout_cache_stats().builds() < document.markdown().blocks().len() as u64);
    }

    #[test]
    fn viewport_remeasures_when_switching_to_shaped_backend() {
        let mut document = EditorDocument::new("ab");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(3.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");

        let metrics = document
            .visible_blocks(ViewportRect::new(0.0, 2.0))
            .expect("metrics viewport should measure");
        assert_eq!(metrics.blocks()[0].height(), 1.0);

        let shaped = document
            .visible_blocks_with_shaper(ViewportRect::new(0.0, 2.0), &WideShaper)
            .expect("shaped viewport should measure");
        assert_eq!(shaped.blocks()[0].height(), 2.0);
        assert_eq!(shaped.content_height(), 2.0);

        let metrics_again = document
            .visible_blocks(ViewportRect::new(0.0, 2.0))
            .expect("metrics viewport should remeasure after backend switch");
        assert_eq!(metrics_again.blocks()[0].height(), 1.0);
    }

    #[test]
    fn viewport_preserves_unaffected_measurements_through_prefix_edits() {
        let mut document = EditorDocument::new("a\n\nb\n\nc\n\nd");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");
        document
            .visible_blocks(ViewportRect::new(100.0, 0.5))
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
            .visible_blocks(ViewportRect::new(100.0, 0.5))
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
            .visible_blocks(ViewportRect::new(0.0, 0.5))
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
            .visible_blocks(ViewportRect::new(0.0, 0.5))
            .expect("new heading block should be queryable");
        assert_eq!(visible.revision(), document.revision());
    }

    #[test]
    fn block_projection_is_dropped_when_block_kind_changes() {
        let mut document = EditorDocument::new("paragraph **羽**\n");
        document
            .block_projection(0)
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
        assert_eq!(document.projection_cache_stats().entries(), 0);
        document
            .block_projection(0)
            .expect("heading projection should build independently");
        assert_eq!(document.projection_cache_stats().builds(), 2);
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
