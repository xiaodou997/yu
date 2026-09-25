use std::error::Error;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use yu_core::{ByteOffset, LineIndex, Revision, ShapingProvider, TextRange, Utf16Range};
use yu_layout::{ImageIntrinsicSize, LayoutConfig, LayoutError};

use crate::blockview::BlockView;
use crate::layout_tokens::{collapsed_block_gap, content_bottom_inset, content_origin_y};
use crate::table::TableResizeCommit;
use yu_markdown::{BlockKind, IncrementalParseError, MarkdownDocument, TaskState};
use yu_state::{EditorHistory, HistoryGroup, HistoryStats, Selections};
use yu_text::{
    AppliedTransaction, EditError, TextBuffer, TextPositionError, TextSnapshot, Transaction,
};

use crate::widget::ImageSize;
use crate::{
    BlockLayoutSource, CaretScrollRequest, CommandResult, CompositionError, CompositionOverlay,
    DecorationCache, DecorationCacheStats, DecorationError, EditorCommand, EditorSelection,
    KeyEvent, KeyRouteResult, LayoutBackend, LayoutCache, LayoutCacheStats, LayoutPoint,
    SearchState, SelectionError, SourceChange, ViewportCaret, ViewportConfig, ViewportError,
    ViewportLayout, ViewportSnapshot, ViewportSpan, ViewportStats, VisualText, VisualTextError,
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

mod image_edit;
mod spelling;
pub use image_edit::ImageProperties;
mod html_disclosure;
mod html_list;
mod layout_context;
mod layout_snapshot;
mod table_edit;
mod table_html;
mod table_paste;
mod table_widths;
pub use table_widths::TableColumnWidthRecord;
mod table_words;
mod tabular_text;
use layout_context::LayoutMeasurements;
pub use layout_context::{EditorRenderSnapshot, LayoutContext};
pub use layout_snapshot::{LayoutQuery, LayoutSnapshot, SnapshotBlock, SnapshotContainer};

/// Mutable transaction state. It is never captured by a layout worker.
#[derive(Debug)]
pub struct EditorState {
    buffer: TextBuffer,
    history: EditorHistory,
    width_history: table_widths::WidthHistory,
    preferred_x: Option<PreferredCaretX>,
    last_source_change: Option<SourceChange>,
    retained_image_destinations: std::collections::BTreeSet<String>,
}

/// A live editor owns mutation rights and one foreground layout context.
/// Source snapshots share immutable storage with the canonical buffer.
#[derive(Debug)]
pub struct EditorDocument {
    state: EditorState,
    presentation: LayoutContext,
}

impl std::ops::Deref for EditorDocument {
    type Target = LayoutContext;
    fn deref(&self) -> &Self::Target {
        &self.presentation
    }
}

impl std::ops::DerefMut for EditorDocument {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.presentation
    }
}

impl EditorDocument {
    pub fn source_mode(&self) -> bool {
        self.markdown.source_mode()
    }

    /// Presentation changes never mutate source, selections or undo history.
    pub fn set_source_mode(&mut self, enabled: bool) -> Result<(), EditorDocumentError> {
        if self.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if self.source_mode() == enabled {
            return Ok(());
        }
        if enabled && self.selections.table_columns().is_some() {
            // The same source ranges become text selections in literal mode.
            // Keeping the cell-grid flag would route Delete/Paste to table edits.
            self.presentation.selections = Selections::new(
                &self.snapshot(),
                self.selections.as_slice().iter().copied(),
                self.selections.primary_index(),
            )?;
        }
        Arc::make_mut(&mut self.presentation.markdown).set_source_mode(enabled);
        if !enabled {
            let positions = self
                .selections()
                .as_slice()
                .iter()
                .flat_map(|selection| [selection.anchor(), selection.focus()])
                .collect::<Vec<_>>();
            for position in positions {
                self.reveal_html_range(TextRange::empty(position))?;
            }
        }
        self.presentation.render_identity = Arc::new(());
        self.presentation.layout_snapshot = None;
        self.presentation.decorations.clear();
        self.presentation.layouts.clear();
        self.presentation.viewport.clear();
        self.state.preferred_x = None;
        Ok(())
    }
    /// Creates a document at the initial revision.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = Arc::new(yu_markdown::parse(&snapshot));
        let selection = EditorSelection::cursor(
            &snapshot,
            yu_core::ByteOffset::ZERO,
            crate::CaretAffinity::Downstream,
        )
        .expect("offset zero is always a valid caret");
        let mut document = Self {
            state: EditorState {
                buffer,
                history: EditorHistory::default(),
                width_history: table_widths::WidthHistory::default(),
                preferred_x: None,
                last_source_change: None,
                retained_image_destinations: Default::default(),
            },
            presentation: LayoutContext {
                table_width_generation: 0,
                table_widths: Arc::new(Vec::new()),
                layout_snapshot: None,
                resource_geometry_version: 0,
                embedded_sizes: None,
                render_identity: Arc::new(()),
                source: snapshot,
                markdown,
                composition: None,
                selections: Selections::single(selection),
                decorations: DecorationCache::default(),
                layouts: LayoutCache::default(),
                viewport: ViewportLayout::default(),
                search: None,
                search_generation: 0,
                spelling: None,
                spelling_generation: 0,
            },
        };
        let revision = document.revision();
        document.presentation.decorations.toc.bind(revision);
        document.remember_image_destinations(None);
        document
    }

    /// Resource identities remain available for undo/redo after Save As.
    /// Only reparsed image syntax is visited during ordinary edits; no source
    /// document or layout cache is cloned into history.
    pub fn retained_image_destinations(&self) -> impl Iterator<Item = &str> {
        self.state
            .retained_image_destinations
            .iter()
            .map(String::as_str)
    }

    fn remember_image_destinations(&mut self, range: Option<TextRange>) {
        if let Ok(images) = self.image_references_in(range) {
            self.state
                .retained_image_destinations
                .extend(images.into_iter().map(|image| image.destination_text));
        }
    }

    /// 塌回一条选区。
    ///
    /// 命令层的绝大多数路径走这里：左右移动、列表编辑、composition 提交之后
    /// 的落点，它们的答案本来就只有一个位置。
    fn set_single_selection(&mut self, selection: EditorSelection) {
        let previous = self.presentation.selection_reveal_block_index();
        self.presentation.selections = Selections::single(selection);
        let next = self.presentation.selection_reveal_block_index();
        if previous != next {
            for index in previous.into_iter().chain(next) {
                self.presentation
                    .viewport
                    .invalidate_block_measurement(index);
            }
        }
    }

    /// Returns the bounded undo/redo depth for the current editor session.
    #[must_use]
    pub fn history_stats(&self) -> HistoryStats {
        self.state.history.stats()
    }

    /// 换一条选区（其余的丢掉）。
    ///
    /// 这是「平台送来一个选区」的入口，塌成一条是对的：鼠标单击、AX 赋值、
    /// 面板导航，每一个都只说得出一个位置。多光标走 [`Self::set_selections`]。
    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), SelectionError> {
        selection.utf16_range(&self.snapshot())?;
        self.set_single_selection(selection);
        self.state.preferred_x = None;
        self.state.history.break_group();
        Ok(())
    }

    /// 换一组选区。
    ///
    /// 归一化（排序、合并、定位 primary）由 [`Selections::new`] 一家做，这里
    /// 不重复一遍——两份合并必定分叉，而分叉的表现是「有时候打字没反应」：
    /// 重叠的选区产出的 Transaction 会被 `validate_edits` 拒掉。
    ///
    /// # Errors
    ///
    /// 输入为空、`primary` 越界、某一条不属于当前 revision，或端点不合法。
    pub fn set_selections(
        &mut self,
        ranges: impl IntoIterator<Item = EditorSelection>,
        primary: usize,
    ) -> Result<(), SelectionError> {
        let snapshot = self.snapshot();
        let selections = Selections::new(&snapshot, ranges, primary)?;
        let previous = self.presentation.selection_reveal_block_index();
        self.presentation.selections = selections;
        let next = self.presentation.selection_reveal_block_index();
        if previous != next {
            for index in previous.into_iter().chain(next) {
                self.presentation
                    .viewport
                    .invalidate_block_measurement(index);
            }
        }
        self.state.preferred_x = None;
        self.state.history.break_group();
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
        let before = self.presentation.selections.clone();
        let widths = self.presentation.table_widths.clone();
        let applied = self.apply_transaction_core(transaction)?;
        let token = self.state.width_history.capture(widths);
        self.state.history.record(&applied, group);
        self.state.history.record_presentation(token);
        self.prune_width_history();
        self.state
            .history
            .record_selections(before, self.presentation.selections.clone());
        Ok(applied)
    }

    fn apply_transaction_core(
        &mut self,
        transaction: &Transaction,
    ) -> Result<AppliedTransaction, EditorDocumentError> {
        self.state.preferred_x = None;
        let before_snapshot = self.snapshot();
        let applied = self.state.buffer.apply(transaction)?;
        self.presentation.source = applied.result_snapshot().clone();
        if before_snapshot.revision() != applied.result_snapshot().revision() {
            self.presentation.render_identity = Arc::new(());
        }
        let incremental = yu_markdown::parse_incremental(
            &self.presentation.markdown,
            applied.result_snapshot(),
            applied.change_set(),
        )?;
        // **整组一起映射，然后重新归一化。** 两个不同的偏移可以映射到同一个
        // 偏移（删掉它们之间的文字），不合并就会留下一对重叠的选区，而下一次
        // 插入会被 `validate_edits` 拒掉——用户看到的是「打字突然没反应」。
        // 收敛点只有 `Selections::map_through` 一个。
        self.presentation.selections = self
            .presentation
            .selections
            .map_through(applied.change_set(), applied.result_snapshot())?;
        // 改一条 reference definition 曾经要把所有缓存整表作废：v1 的投影
        // 先查表才知道 `[id]` 是不是一个链接。换成语法树之后 `[id]` 的
        // `LinkLabel` 是树给的结构，隐藏区间不再依赖索引（不变量 C6 说的
        // 「解析目标」才需要），所以没有东西要作废了。
        self.presentation
            .decorations
            .shift_through(applied.change_set(), applied.result_snapshot());
        self.presentation
            .layouts
            .map_through(applied.change_set(), applied.result_snapshot())
            .map_err(EditorDocumentError::Layout)?;
        self.presentation
            .viewport
            .map_through(
                applied.change_set(),
                applied.result_snapshot(),
                incremental.document(),
            )
            .map_err(EditorDocumentError::Viewport)?;
        self.presentation
            .decorations
            .retain_blocks(incremental.document());
        self.presentation
            .layouts
            .retain_blocks(incremental.document());
        self.presentation.map_table_widths(applied.change_set());
        let image_range = (self
            .presentation
            .markdown
            .reference_definitions()
            .fingerprint()
            == incremental.document().reference_definitions().fingerprint())
        .then_some(incremental.reparsed_range());
        self.presentation.markdown = Arc::new(incremental.into_document());
        self.remember_image_destinations(image_range);
        // 匹配是 `(TextSnapshot, query)` 的纯函数，源码变了就得重扫。没有人
        // 在搜索时这里一分钱不花；有人在搜索时，代价是一次全文子串扫描——
        // 装饰与布局是增量的，这一个不是，因为一次编辑可以让任意远处的匹配
        // 出现或消失（`ab` 中间插一个字符）。
        if let Some(state) = self.presentation.search.as_ref() {
            self.presentation.search = Some(Arc::new(SearchState::new(
                applied.result_snapshot(),
                state.query(),
            )));
        }
        self.state.last_source_change = source_change_from_applied(&before_snapshot, &applied)?;
        Ok(applied)
    }

    /// Starts or replaces the transient composition overlay.
    /// 开一段 IME 组字。
    ///
    /// # 多光标塌成一条，这是一笔登记在案的欠账
    ///
    /// `CompositionOverlay` 是**一个** preedit 覆盖**一个** `replacement_range`
    /// （见 `yu_state::CompositionOverlay`），而 `NSTextInputClient` 也只交出
    /// 一个 marked range——单数是平台 ABI 与这个类型共同的形状，不是这里偷懒。
    ///
    /// 所以开始组字时先把选区塌回 primary。**塌是必须的，不是可选的**：留着
    /// N 条选区而只有一条真的在组字，屏幕上会有几根不动的假光标，提交之后它们
    /// 还会被 `map_through` 搬到莫名其妙的位置——不报错、不 panic。
    ///
    /// **还债条件**：要做「三个光标一起打中文」，就得让 commit 把已确定的文字
    /// 在其余每一处也插一遍，而那个 Transaction 覆盖的是 overlay 从没验证过的
    /// N−1 个区间。不变量 H1（overlay 不写 canonical source）、H2（只有 commit
    /// 产生永久 Transaction）、H6（携带 expected Revision 与 generation）三条
    /// 都要重新过一遍。**这一刀不做。**
    pub fn begin_composition(
        &mut self,
        replacement_range: TextRange,
        text: impl Into<Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), EditorDocumentError> {
        self.validate_source_range(replacement_range)?;
        self.state.history.break_group();
        self.state.preferred_x = None;
        if self.presentation.selections.is_multiple() {
            self.presentation.selections = self.presentation.selections.collapsed_to_primary();
        }
        self.presentation.composition = Some(CompositionOverlay::new(
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
            .presentation
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
            .presentation
            .composition
            .as_ref()
            .ok_or(EditorDocumentError::CompositionNotActive)?;
        let replacement_range = composition.replacement_range();
        let committed_text: Arc<str> = committed_text.into();
        let committed_text = self.table_cell_input(replacement_range, committed_text);
        let committed_cursor = self
            .html_table_input_cursor(replacement_range, committed_text.len())
            .unwrap_or(committed_text.len());
        let transaction = composition.clone().commit(Arc::clone(&committed_text));
        let applied = self.apply_transaction_with_group(&transaction, HistoryGroup::Composition)?;
        let cursor_offset = replacement_range
            .start()
            .checked_add(
                u64::try_from(committed_cursor)
                    .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?,
            )
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        self.set_edit_selection(EditorSelection::cursor(
            applied.result_snapshot(),
            cursor_offset,
            crate::CaretAffinity::Downstream,
        )?);
        self.presentation.composition = None;
        self.state.preferred_x = None;
        self.state.last_source_change = None;
        self.state.history.break_group();
        Ok(applied)
    }

    /// Drops the active overlay without changing source or revision.
    #[must_use]
    pub fn cancel_composition(&mut self) -> bool {
        let cancelled = self.presentation.composition.take().is_some();
        if cancelled {
            self.state.preferred_x = None;
            self.state.history.break_group();
        }
        cancelled
    }

    /// Replaces the source for a newly opened document and resets its revision.
    pub fn reset_source(&mut self, source: impl Into<String>) -> Result<(), EditorDocumentError> {
        if self.presentation.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        self.presentation.render_identity = Arc::new(());
        self.state.buffer = TextBuffer::new(source);
        self.presentation.source = self.state.buffer.snapshot();
        let source_mode = self.source_mode();
        let mut markdown = yu_markdown::parse(&self.state.buffer.snapshot());
        markdown.set_source_mode(source_mode);
        self.presentation.markdown = Arc::new(markdown);
        self.presentation.decorations.clear();
        self.presentation.layouts.clear();
        self.presentation.viewport.clear();
        self.state.history.clear();
        self.state.retained_image_destinations.clear();
        self.remember_image_destinations(None);
        self.state.width_history = table_widths::WidthHistory::default();
        self.presentation.table_widths = Arc::new(Vec::new());
        self.presentation.table_width_generation = self.table_width_generation.wrapping_add(1);
        self.state.preferred_x = None;
        self.state.last_source_change = None;
        let snapshot = self.snapshot();
        if let Some(search) = self.presentation.search.as_ref() {
            self.presentation.search = Some(Arc::new(SearchState::new(&snapshot, search.query())));
        }
        self.set_single_selection(
            EditorSelection::cursor(
                &snapshot,
                snapshot.len_bytes(),
                crate::CaretAffinity::Downstream,
            )
            .expect("the end of a reset source is a valid caret"),
        );
        Ok(())
    }

    /// Executes a small revision-bound editing command set.
    pub fn execute(
        &mut self,
        command: EditorCommand,
    ) -> Result<CommandResult, EditorDocumentError> {
        self.state.last_source_change = None;
        // A native text input client owns the transient marked-text lifecycle
        // while a composition is active.  Keep the same invariant at the
        // platform-independent editor boundary so a caller cannot bypass the
        // FFI/menu availability guard and accidentally create a permanent
        // transaction over the composition's fixed replacement range.
        if self.presentation.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if !matches!(
            command,
            EditorCommand::MoveUp
                | EditorCommand::MoveDown
                | EditorCommand::MoveUpExtend
                | EditorCommand::MoveDownExtend
        ) {
            self.state.preferred_x = None;
        }
        match command {
            EditorCommand::InsertText(text) => self.insert_text(text),
            EditorCommand::PasteFragments(fragments) => self.paste_fragments(fragments),
            EditorCommand::PasteTsv(text) => self.paste_tsv(text),
            EditorCommand::PasteClipboardText { text, tabular } => {
                self.paste_clipboard_text(text, tabular)
            }
            EditorCommand::PasteTableGrid { columns, cells } => {
                self.paste_table_grid(columns, cells)
            }
            EditorCommand::PasteHtmlTableGrid { columns, cells } => {
                self.paste_html_table_grid(columns, cells)
            }
            EditorCommand::PasteHtmlTableSource(source) => self.paste_html_table_source(source),
            EditorCommand::SelectTableCells { anchor, focus } => {
                self.select_table_cells(anchor, focus)
            }
            EditorCommand::EditTable(edit) => self.edit_table(edit),
            EditorCommand::DeleteBackward => self.delete_backward(),
            EditorCommand::DeleteForward => self.delete_forward(),
            EditorCommand::DeleteWordBackward => self.delete_word(false),
            EditorCommand::DeleteWordForward => self.delete_word(true),
            EditorCommand::DeleteSelections => {
                if self.presentation.selections.table_columns().is_some() {
                    return self.clear_table_cells();
                }
                self.state.history.break_group();
                let ranges = self
                    .presentation
                    .selections
                    .as_slice()
                    .iter()
                    .map(|s| s.ordered_range())
                    .collect();
                let result = self.delete_ranges(ranges, HistoryGroup::Deletion);
                self.state.history.break_group();
                result
            }
            EditorCommand::MoveLeft => self.move_left(),
            EditorCommand::MoveRight => self.move_right(),
            EditorCommand::MoveWordLeft => self.move_word_left(),
            EditorCommand::MoveWordRight => self.move_word_right(),
            EditorCommand::ExtendHorizontal { forward, word } => {
                self.extend_horizontal(forward, word)
            }
            EditorCommand::MoveDocumentBoundary { end, extend } => {
                self.move_document_boundary(end, extend)
            }
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
            EditorCommand::ToggleHtmlDetails { source } => self.toggle_html_details(source),
            EditorCommand::ToggleTask { block } => self.toggle_task(block),
        }
    }

    /// Reports whether a command can currently make a meaningful editor
    /// transition. This is a read-only query for native menu/selector
    /// validation; executing a command remains the authoritative operation.
    #[must_use]
    pub fn command_available(&self, command: &EditorCommand) -> bool {
        if self.presentation.composition.is_some() {
            return false;
        }
        let snapshot = self.snapshot();
        match command {
            EditorCommand::InsertText(text) => !text.is_empty(),
            EditorCommand::PasteFragments(fragments) => !fragments.is_empty(),
            EditorCommand::PasteClipboardText { text, .. } => !text.is_empty(),
            EditorCommand::PasteTsv(text) => {
                self.tsv_grid_for_target(text)
                    .is_ok_and(|(columns, cells)| {
                        self.command_available(&EditorCommand::PasteTableGrid { columns, cells })
                    })
            }
            EditorCommand::PasteTableGrid { columns, cells } => {
                if self.grid_paste_is_outside_table() {
                    Self::serialize_grid(*columns, cells).is_ok()
                } else {
                    self.table_grid_plan(*columns, cells).is_ok()
                }
            }
            EditorCommand::PasteHtmlTableGrid { columns, cells } => {
                self.html_grid_source(*columns, cells).is_ok()
                    && self.table_target_is_html() != Some(false)
            }
            EditorCommand::PasteHtmlTableSource(source) => {
                self.html_table_source_plan(source).is_ok()
            }
            EditorCommand::SelectTableCells { anchor, focus } => {
                self.table_cell_selection(*anchor, *focus).is_some()
            }
            EditorCommand::EditTable(edit) => self.table_edit_plan(*edit).is_some(),
            EditorCommand::ToggleHtmlDetails { source } => {
                self.html_disclosure_header_at(*source).is_some()
            }
            EditorCommand::DeleteWordBackward | EditorCommand::DeleteWordForward => {
                let forward = matches!(command, EditorCommand::DeleteWordForward);
                self.presentation
                    .selections
                    .as_slice()
                    .iter()
                    .any(|selection| {
                        !selection.is_empty()
                            || self
                                .word_target(&snapshot, selection.focus(), forward)
                                .is_ok_and(|target| target != selection.focus())
                    })
            }
            EditorCommand::DeleteSelections => self
                .presentation
                .selections
                .as_slice()
                .iter()
                .any(|s| !s.is_empty()),
            // **判据是「有没有哪一条动得了」，不是 primary 动不动得了。**
            // 按 primary 判会让「primary 停在文档开头、别的光标在中间」这一
            // 局面下整条退格菜单项变灰——另外几个光标明明删得动。
            EditorCommand::DeleteBackward
            | EditorCommand::MoveLeft
            | EditorCommand::MoveWordLeft => self
                .presentation
                .selections
                .as_slice()
                .iter()
                .any(|selection| !selection.is_empty() || selection.focus() > ByteOffset::ZERO),
            EditorCommand::DeleteForward
            | EditorCommand::MoveRight
            | EditorCommand::MoveWordRight => self
                .presentation
                .selections
                .as_slice()
                .iter()
                .any(|selection| !selection.is_empty() || selection.focus() < snapshot.len_bytes()),
            EditorCommand::MoveUp | EditorCommand::MoveUpExtend => {
                self.vertical_command_available(VerticalDirection::Up)
            }
            EditorCommand::MoveDown | EditorCommand::MoveDownExtend => {
                self.vertical_command_available(VerticalDirection::Down)
            }
            EditorCommand::MoveTableCellNext => {
                self.table_cell_navigation_target(false).is_some()
                    || self.table_append_at_focus().is_some()
            }
            EditorCommand::MoveTableCellPrevious => {
                self.table_cell_navigation_target(true).is_some()
            }
            EditorCommand::InsertNewline
            | EditorCommand::ExtendHorizontal { .. }
            | EditorCommand::MoveDocumentBoundary { .. } => true,
            EditorCommand::IndentList => {
                if self.has_html_list_selection() {
                    return self.html_list_command_available(true);
                }
                self.multiple_html_indent_plan().is_some()
                    || self.html_list_indent_plan().is_some()
                    || self
                        .current_list_line()
                        .is_some_and(|line| self.list_prefix(&line).is_some())
            }
            EditorCommand::OutdentList => {
                if self.has_html_list_selection() {
                    return self.html_list_command_available(false);
                }
                self.multiple_html_outdent_plan().is_some()
                    || self.root_html_list_outdent_plan().is_some()
                    || self.html_list_outdent_plan(false).is_some()
                    || self.current_list_line().is_some_and(|line| {
                        self.list_prefix(&line).is_some_and(|_| {
                            line.content
                                .as_bytes()
                                .iter()
                                .take_while(|byte| **byte == b' ')
                                .next()
                                .is_some()
                        })
                    })
            }
            EditorCommand::Undo => self.state.history.stats().undo_entries() > 0,
            EditorCommand::Redo => self.state.history.stats().redo_entries() > 0,
            EditorCommand::ToggleTask { block } => self
                .presentation
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
        if self.presentation.composition.is_some() {
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
        if self.source_mode()
            && event.key() == crate::EditorKey::Tab
            && event.modifiers() == crate::KeyModifiers::NONE
        {
            return Some(EditorCommand::insert_text("\t"));
        }
        if event.key() == crate::EditorKey::Tab {
            let previous = event.modifiers() == crate::KeyModifiers::SHIFT;
            let plain = event.modifiers() == crate::KeyModifiers::NONE;
            if (plain || previous)
                && (self.table_cell_navigation_target(previous).is_some()
                    || (!previous && self.table_append_at_focus().is_some()))
            {
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
        let block = self.presentation.markdown.blocks().get(index).ok_or(
            EditorDocumentError::BlockOutOfBounds {
                index,
                blocks: self.presentation.markdown.blocks().len(),
            },
        )?;
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

    fn restore_history_selections(
        &mut self,
        saved: &Selections,
    ) -> Result<(), EditorDocumentError> {
        let snapshot = self.snapshot();
        let ranges = saved
            .as_slice()
            .iter()
            .map(|selection| {
                EditorSelection::range(
                    &snapshot,
                    selection.anchor(),
                    selection.focus(),
                    selection.affinity(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        // History restores source positions, so expose the corresponding HTML
        // content just as navigation does. This is presentation-only and must
        // not create a new edit or replace the saved selection.
        if self.composition().is_none() {
            for range in &ranges {
                self.reveal_html_range(TextRange::empty(range.anchor()))?;
                if range.focus() != range.anchor() {
                    self.reveal_html_range(TextRange::empty(range.focus()))?;
                }
            }
        }
        self.set_selections(ranges, saved.primary_index())?;
        if let Some(columns) = saved.table_columns().filter(|_| !self.source_mode()) {
            self.presentation.selections = self.presentation.selections.clone().with_table_slots(
                columns,
                saved
                    .table_slots()
                    .ok_or(SelectionError::InvalidRange)?
                    .to_vec(),
            )?;
        }
        Ok(())
    }

    fn set_edit_selection(&mut self, selection: EditorSelection) {
        self.set_single_selection(selection);
        self.state
            .history
            .finish_selection(self.presentation.selections.clone());
    }

    /// Replays one grouped set of inverse transactions without recording the
    /// replay itself as a new edit. The inverse of each replay becomes the
    /// corresponding redo transaction.
    pub fn undo(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let Some(entries) = self.state.history.pop_undo_group() else {
            self.state.history.break_group();
            return Ok(self.command_result(false));
        };
        let mut redo = Vec::with_capacity(entries.len());
        let mut rollback = Vec::with_capacity(entries.len());
        let original_widths = self.presentation.table_widths.clone();
        for entry in &entries {
            let widths = self.presentation.table_widths.clone();
            let transaction = entry.transaction_for(self.revision());
            match self.apply_transaction_core(&transaction) {
                Ok(applied) => {
                    if let Some(target) = entry.target_selections() {
                        self.restore_history_selections(target)?;
                    }
                    rollback.push(applied.inverse().clone());
                    self.restore_width_history(entry.target_presentation());
                    let token = self.state.width_history.capture(widths);
                    redo.push(
                        entry
                            .inverse(applied.inverse().clone())
                            .with_presentation(token),
                    );
                }
                Err(error) => {
                    for transaction in rollback.iter().rev() {
                        let _ = self.apply_transaction_core(transaction);
                    }
                    self.presentation.table_widths = original_widths;
                    self.state.history.restore_undo_group(&entries);
                    self.prune_width_history();
                    return Err(error);
                }
            }
        }
        self.state.history.push_redo_group(redo);
        self.prune_width_history();
        Ok(self.command_result(true).requiring_full_source_sync())
    }

    /// Replays one grouped set of forward transactions without recording the
    /// replay itself as a new edit. The inverse of each replay is restored to
    /// the undo stack in the original stack order.
    pub fn redo(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let Some(entries) = self.state.history.pop_redo_group() else {
            self.state.history.break_group();
            return Ok(self.command_result(false));
        };
        let mut undo = Vec::with_capacity(entries.len());
        let mut rollback = Vec::with_capacity(entries.len());
        let original_widths = self.presentation.table_widths.clone();
        for entry in &entries {
            let widths = self.presentation.table_widths.clone();
            let transaction = entry.transaction_for(self.revision());
            match self.apply_transaction_core(&transaction) {
                Ok(applied) => {
                    if let Some(target) = entry.target_selections() {
                        self.restore_history_selections(target)?;
                    }
                    rollback.push(applied.inverse().clone());
                    self.restore_width_history(entry.target_presentation());
                    let token = self.state.width_history.capture(widths);
                    undo.push(
                        entry
                            .inverse(applied.inverse().clone())
                            .with_presentation(token),
                    );
                }
                Err(error) => {
                    for transaction in rollback.iter().rev() {
                        let _ = self.apply_transaction_core(transaction);
                    }
                    self.presentation.table_widths = original_widths;
                    self.state.history.restore_redo_group(&entries);
                    self.prune_width_history();
                    return Err(error);
                }
            }
        }
        self.state.history.push_undo_group(undo);
        self.prune_width_history();
        Ok(self.command_result(true).requiring_full_source_sync())
    }

    /// Inserts a line ending and, when the caret is in a list item, continues
    /// its source prefix. A completed task always starts the next item as
    /// unchecked. Pressing Enter on an empty list item exits the list by
    /// removing that line's prefix while preserving its line ending.
    pub fn insert_newline(&mut self) -> Result<CommandResult, EditorDocumentError> {
        if let Some(result) = self.split_html_list_item()? {
            return Ok(result);
        }
        if self.presentation.selections.is_multiple()
            || self
                .html_table_cell_input(self.selection().ordered_range(), "\n")
                .is_some()
        {
            return self.insert_plain_newlines();
        }
        let snapshot = self.snapshot();
        let selection_range = self.selection().ordered_range();
        let line = source_line(&snapshot, selection_range.start())?;
        if self.selection().is_empty() {
            let caret = self.selection().focus();
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
                    self.set_edit_selection(EditorSelection::cursor(
                        applied.result_snapshot(),
                        line.start,
                        crate::CaretAffinity::Downstream,
                    )?);
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
                self.set_edit_selection(EditorSelection::cursor(
                    applied.result_snapshot(),
                    offset,
                    crate::CaretAffinity::Downstream,
                )?);
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
        self.set_edit_selection(EditorSelection::cursor(
            applied.result_snapshot(),
            offset,
            crate::CaretAffinity::Downstream,
        )?);
        Ok(self.command_result(true))
    }

    /// N>1 时的 Enter：每一条选区各插一个换行，不续行、不退出列表。
    ///
    /// # 列表类编辑是 primary 降级，这是一笔登记在案的欠账
    ///
    /// 涉及四条：Enter 的列表续行与空项退出（[`Self::insert_newline`]）、空列表
    /// 项的退格（`delete_empty_list_prefix`）、缩进与反缩进
    /// （[`Self::indent_list`] / [`Self::outdent_list`]）。
    ///
    /// 理由不是省事：**这四条改的都是「整行」，而两个光标可以停在同一行上。**
    /// 「空项退出列表」删的是 `line.content_range()`，两个光标在同一行就产出
    /// 一对完全重叠的 edit，`yu_text::validate_edits` 会拒掉整条 Transaction
    /// ——表现是「按一下 Enter 什么都没发生」。要做对就得先按行去重，那是另一
    /// 层（「一条命令要影响哪些行」）该回答的事。
    ///
    /// 所以：N>1 时 Enter 只插普通换行（就是这个函数，每一条
    /// 选区各插一个，不续行、不退出列表），缩进/反缩进只作用在 primary 那一行。
    /// N=1 时行为与多光标之前完全一样。
    ///
    /// **还债条件**：有人真的在多光标下编辑列表并抱怨。那时先给「一条命令影响
    /// 哪些行」建一个去重的入口，四条一起改。
    ///
    /// # 行尾符按每一条选区自己那一行取
    ///
    /// 不是整篇取一次——一篇文档里两种行尾混着是合法的，CRLF 的那几行要保持
    /// CRLF。
    fn insert_plain_newlines(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let mut edits = Vec::with_capacity(self.presentation.selections.len());
        for selection in self.presentation.selections.as_slice() {
            let range = selection.ordered_range();
            let line = source_line(&snapshot, range.start())?;
            let text = self
                .html_table_cell_input(range, line.insertion_terminator())
                .map_or_else(|| Arc::<str>::from(line.insertion_terminator()), Arc::from);
            edits.push((range, text));
        }
        self.apply_selection_edits(edits, HistoryGroup::ListEditing, CollapseTo::End)
    }

    /// Indents the current list item by two source spaces.
    ///
    /// **primary 那一行。** 理由见 [`Self::insert_plain_newlines`] 上那段说明。
    pub fn indent_list(&mut self) -> Result<CommandResult, EditorDocumentError> {
        if let Some(result) = self.change_html_list_indent(true)? {
            return Ok(result);
        }
        let snapshot = self.snapshot();
        let line = source_line(&snapshot, self.selection().focus())?;
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
    ///
    /// **primary 那一行。** 理由见 [`Self::insert_plain_newlines`] 上那段说明。
    pub fn outdent_list(&mut self) -> Result<CommandResult, EditorDocumentError> {
        if let Some(result) = self.change_html_list_indent(false)? {
            return Ok(result);
        }
        let snapshot = self.snapshot();
        let line = source_line(&snapshot, self.selection().focus())?;
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

    /// 在每一条选区处插入同一段文字。
    ///
    /// **一条命令一个 Transaction、N 条 edit。** `Transaction` 本来就收一组
    /// 互不重叠的 edit，排序、原子提交、一份 inverse 全部由它做
    /// （`yu_text::Transaction::prepare`）——所以 **undo 分组一个字都不用改**：
    /// 三个光标同时打一个字仍然是一次 `history.record`，连续输入照样并进同一
    /// 个 group。这是查代码时推翻的一个预期，值得写在这里。
    fn insert_text(&mut self, text: Arc<str>) -> Result<CommandResult, EditorDocumentError> {
        if text.is_empty() {
            return Ok(self.command_result(false));
        }
        let edits: Vec<_> = self
            .presentation
            .selections
            .as_slice()
            .iter()
            .map(|selection| {
                let range = selection.ordered_range();
                (range, self.table_cell_input(range, Arc::clone(&text)))
            })
            .collect();
        self.apply_selection_edits(edits, HistoryGroup::Typing, CollapseTo::End)
    }

    /// Equal-sized source fragment and target sets distribute one-to-one.
    /// Otherwise paste the combined payload at each target, as plain copy does.
    /// A paste is always one undo step, isolated from typing and other pastes.
    fn paste_fragments(
        &mut self,
        fragments: Vec<Arc<str>>,
    ) -> Result<CommandResult, EditorDocumentError> {
        if fragments.is_empty() {
            return Ok(self.command_result(false));
        }
        if fragments.len() == 1
            && !self.grid_paste_is_outside_table()
            && let Some((columns, cells)) = Self::markdown_table_grid(&fragments[0])
        {
            return self.paste_table_grid(columns, cells);
        }
        let replacements = if fragments.len() == self.presentation.selections.len() {
            fragments
        } else {
            let joined: Arc<str> = fragments.join("\n").into();
            vec![joined; self.presentation.selections.len()]
        };
        let edits = self
            .presentation
            .selections
            .as_slice()
            .iter()
            .zip(replacements)
            .map(|(selection, text)| {
                let range = selection.ordered_range();
                (range, self.table_cell_input(range, text))
            })
            .collect();
        self.state.history.break_group();
        let result = self.apply_selection_edits(edits, HistoryGroup::Typing, CollapseTo::End);
        self.state.history.break_group();
        result
    }

    /// Literal pipes inside one visible cell must not become column separators.
    /// Preserve already escaped Markdown and the backslash parity at the edit
    /// boundary. Cross-cell edits remain separate from this single-cell policy.
    fn table_cell_input(&self, range: TextRange, text: Arc<str>) -> Arc<str> {
        if let Some(encoded) = self.html_table_cell_input(range, &text) {
            return Arc::from(encoded);
        }
        self.table_source_input(range, text)
    }

    fn table_source_input(&self, range: TextRange, text: Arc<str>) -> Arc<str> {
        if !text.contains('|') && !text.contains('\\') {
            return text;
        }
        let Some(block) = self
            .block_index_for_offset(range.start())
            .and_then(|index| self.presentation.markdown.blocks().get(index))
        else {
            return text;
        };
        let Some(table) = yu_markdown::table_for_block(&self.presentation.markdown, block) else {
            return text;
        };
        let Some(cell) = table
            .visible_cell_for_source(range.start().get() as usize)
            .and_then(|address| table.visible_cell(address))
        else {
            return text;
        };
        if range.end().get() as usize > cell.end() {
            return text;
        }
        let snapshot = self.snapshot();
        let source = snapshot.as_str();
        let Some(encoded) = yu_markdown::quote_table_cell_input(
            &source[cell.start()..cell.end()],
            (range.start().get() as usize - cell.start())
                ..(range.end().get() as usize - cell.start()),
            &text,
            range.end().get() as usize == cell.end()
                && source.as_bytes().get(cell.end()) == Some(&b'|'),
        ) else {
            return text;
        };
        if encoded == text.as_ref() {
            text
        } else {
            Arc::from(encoded)
        }
    }

    /// 应用一组「一条选区一个替换」的编辑，并把每一条选区落到它自己那一处。
    ///
    /// `edits` 与当前选区**一一对应、同序**；替换为空且区间为空的那几条是空操作
    /// （文档开头的光标按退格就是），它们不进 Transaction 但仍然占一个落点。
    ///
    /// # 落点是算出来的，不是映射出来的
    ///
    /// 这是这一刀查出来的一个真缺口。`ChangeSet::map_anchor` 在两条 edit 首尾
    /// 相接时，会把前一条的**终点**（选区的终点按 `Affinity::After` 映射）一路
    /// 推到后一条替换之后：在 `aaaa` 里选中两处 `aa` 再打一个字，两条选区映射
    /// 之后重叠，`Selections` 把它们并成一条——源码是对的，屏幕上却只剩一根
    /// 光标。**不报错、不 panic**，而「选中全部匹配再替换」正是这一刀最主要的
    /// 用法。
    ///
    /// 修法不是去改 `map_anchor` 的端点语义（那会动到所有 anchor 的映射），
    /// 而是**这里根本不必去猜**：这些 edit 是这条命令自己造的，第 i 条之前的
    /// 累计位移直接加得出来。`map_through` 仍然跑（`apply_transaction_core`
    /// 里那一次，它服务的是外来的 Transaction：undo/redo、缩进、任务勾选），
    /// 这里只是随后把答案换成精确的那一份。
    fn apply_selection_edits(
        &mut self,
        edits: Vec<(TextRange, Arc<str>)>,
        group: HistoryGroup,
        to: CollapseTo,
    ) -> Result<CommandResult, EditorDocumentError> {
        debug_assert_eq!(
            edits.len(),
            self.presentation.selections.len(),
            "每一条选区必须恰好产出一个替换"
        );
        let mut targets = Vec::with_capacity(edits.len());
        let mut delta = 0_i128;
        for (range, replacement) in &edits {
            let start = i128::from(range.start().get()) + delta;
            let inserted = i128::try_from(replacement.len())
                .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?;
            delta += inserted - i128::from(range.len());
            targets.push(match to {
                CollapseTo::Start => start,
                CollapseTo::End => {
                    start
                        + self
                            .html_table_input_cursor(*range, replacement.len())
                            .map_or(inserted, |position| position as i128)
                }
            });
        }

        let applied: Vec<_> = edits
            .iter()
            .filter(|(range, replacement)| !(range.is_empty() && replacement.is_empty()))
            .map(|(range, replacement)| yu_text::Edit::new(*range, Arc::clone(replacement)))
            .collect();
        if applied.is_empty() {
            return Ok(self.command_result(false));
        }

        let primary = self.presentation.selections.primary_index();
        let transaction = Transaction::new(self.revision(), applied);
        self.apply_transaction_with_group(&transaction, group)?;

        let snapshot = self.snapshot();
        let mut collapsed = Vec::with_capacity(targets.len());
        for target in targets {
            let offset = u64::try_from(target)
                .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?;
            collapsed.push(EditorSelection::cursor(
                &snapshot,
                ByteOffset::new(offset),
                crate::CaretAffinity::Downstream,
            )?);
        }
        self.presentation.selections = Selections::new(&snapshot, collapsed, primary)?;
        self.state
            .history
            .finish_selection(self.presentation.selections.clone());
        Ok(self.command_result(true))
    }

    fn delete_backward(&mut self) -> Result<CommandResult, EditorDocumentError> {
        if self.presentation.selections.table_columns().is_some() {
            return self.execute(EditorCommand::DeleteSelections);
        }

        if let Some(result) = self.backspace_html_list_start()? {
            return Ok(result);
        }

        // 空列表项的退格（删掉整条标记）**只在单光标下走**，见
        // `insert_plain_newlines` 上那段说明：它删的是整行的内容，两个
        // 光标停在同一行上会产出一对重叠的 edit，整条命令因此失败。
        if !self.presentation.selections.is_multiple()
            && self.selection().is_empty()
            && let Some(result) = self.delete_empty_list_prefix()?
        {
            return Ok(result);
        }
        let snapshot = self.snapshot();
        let mut ranges = Vec::with_capacity(self.presentation.selections.len());
        for selection in self.presentation.selections.as_slice() {
            ranges.push(if selection.is_empty() {
                let start = previous_grapheme_boundary(&snapshot, selection.focus())?;
                self.table_atom_deletion_range(
                    TextRange::new(start, selection.focus())
                        .expect("previous grapheme boundary must precede caret"),
                    false,
                )?
            } else {
                selection.ordered_range()
            });
        }
        self.delete_ranges(ranges, HistoryGroup::Deletion)
    }

    fn table_atom_deletion_range(
        &self,
        range: TextRange,
        forward: bool,
    ) -> Result<TextRange, EditorDocumentError> {
        use unicode_segmentation::UnicodeSegmentation;
        let markdown = &self.presentation.markdown;
        let Some(block) = self
            .block_index_for_offset(range.start())
            .and_then(|index| markdown.blocks().get(index))
        else {
            return Ok(range);
        };
        if let Some(range) = self.html_table_grapheme_deletion(range, forward)? {
            return Ok(range);
        }
        let atom = yu_markdown::table_atom_deletion_range(markdown, block, range);
        if atom == range {
            return Ok(range);
        }
        let from = if forward { range.start() } else { range.end() };
        let Some(visual) = self.projected_table_cell(from)? else {
            return Ok(range);
        };
        let position = visual.source_to_visual(from, Bias::After)?.get() as usize;
        let mut graphemes = visual.text().grapheme_indices(true);
        let cluster = if forward {
            graphemes.find(|(start, text)| start + text.len() > position)
        } else {
            graphemes.rev().find(|(start, _)| *start < position)
        };
        let Some((start, text)) = cluster else {
            return Ok(atom);
        };
        // Use the same atom coverage as painted clusters. This keeps multi-
        // scalar literals whole without swallowing a preceding hard break.
        let projected = visual.source_coverage(
            yu_core::VisualRange::new(
                yu_core::VisualOffset::new(start as u64),
                yu_core::VisualOffset::new((start + text.len()) as u64),
            )
            .expect("ordered projected grapheme"),
        )?;
        Ok(yu_markdown::table_atom_deletion_range(
            markdown, block, projected,
        ))
    }

    fn delete_empty_list_prefix(&mut self) -> Result<Option<CommandResult>, EditorDocumentError> {
        let snapshot = self.snapshot();
        let line = source_line(&snapshot, self.selection().focus())?;
        let Some(prefix) = self.list_prefix(&line) else {
            return Ok(None);
        };
        let relative = byte_distance(line.start, self.selection().focus())?;
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
        self.set_edit_selection(EditorSelection::cursor(
            applied.result_snapshot(),
            line.start,
            crate::CaretAffinity::Downstream,
        )?);
        Ok(Some(self.command_result(true)))
    }

    fn delete_word(&mut self, forward: bool) -> Result<CommandResult, EditorDocumentError> {
        if self.presentation.selections.table_columns().is_some() {
            return self.execute(EditorCommand::DeleteSelections);
        }
        let snapshot = self.snapshot();
        let mut ranges = self
            .presentation
            .selections
            .as_slice()
            .iter()
            .map(|selection| {
                if !selection.is_empty() {
                    return Ok(selection.ordered_range());
                }
                let from = selection.focus();
                let target = self.word_target(&snapshot, from, forward)?;
                Ok(TextRange::new(from.min(target), from.max(target))
                    .expect("ordered word deletion"))
            })
            .collect::<Result<Vec<_>, EditorDocumentError>>()?;
        ranges.retain(|range| !range.is_empty());
        if ranges.is_empty() {
            return Ok(self.command_result(false));
        }
        // A later caret can reach behind an earlier selected range. Merge the
        // source union before editing; caret order alone does not sort these ranges.
        ranges.sort_by_key(|range| range.start());
        let mut merged: Vec<TextRange> = Vec::new();
        for range in ranges {
            if let Some(last) = merged.last_mut()
                && range.start() <= last.end()
            {
                *last = TextRange::new(last.start(), last.end().max(range.end())).expect("union");
            } else {
                merged.push(range);
            }
        }
        let transaction = Transaction::new(
            snapshot.revision(),
            merged.into_iter().map(|range| {
                let replacement = self.html_table_deletion_text(range);
                yu_text::Edit::new(range, replacement.unwrap_or_else(|| Arc::from("")))
            }),
        );
        self.state.history.break_group();
        // Pure deletions map all original selections/carets to their surviving
        // boundaries. History records the original set, not the merged edit union.
        let result = self
            .apply_transaction_with_group(&transaction, HistoryGroup::Deletion)
            .map(|_| self.command_result(true));
        self.state.history.break_group();
        result
    }

    fn delete_forward(&mut self) -> Result<CommandResult, EditorDocumentError> {
        if self.presentation.selections.table_columns().is_some() {
            return self.execute(EditorCommand::DeleteSelections);
        }

        let snapshot = self.snapshot();
        let mut ranges = Vec::with_capacity(self.presentation.selections.len());
        for selection in self.presentation.selections.as_slice() {
            ranges.push(if selection.is_empty() {
                let end = next_grapheme_boundary(&snapshot, selection.focus())?;
                self.table_atom_deletion_range(
                    TextRange::new(selection.focus(), end)
                        .expect("next grapheme boundary must follow caret"),
                    true,
                )?
            } else {
                selection.ordered_range()
            });
        }
        self.delete_ranges(ranges, HistoryGroup::Deletion)
    }

    /// 一次删掉 N 段。
    ///
    /// **空区间不进 Transaction**（由 [`Self::apply_selection_edits`] 滤掉）。
    /// 文档开头的光标按退格产出的就是一个空区间，而两个空 edit 落在同一个偏移
    /// 会被 `yu_text` 的 `validate_edits` 当成重叠拒掉——整条 Transaction 因此
    /// 失败，表现是「另外几个光标也删不动」，不报错、不 panic。
    fn delete_ranges(
        &mut self,
        ranges: Vec<TextRange>,
        group: HistoryGroup,
    ) -> Result<CommandResult, EditorDocumentError> {
        if let Some(result) = self.delete_html_paragraph_boundaries(&ranges, group)? {
            return Ok(result);
        }
        // Two source carets can address the same projected escape atom.
        // Delete its bytes once, retaining one mapped target per selection.
        let mut previous_end = ByteOffset::ZERO;
        let edits = ranges
            .into_iter()
            .enumerate()
            .map(|(index, range)| {
                let start = range.start().max(previous_end);
                let end = range.end().max(start);
                previous_end = end;
                let range = TextRange::new(start, end).expect("ordered deletion");
                let replacement = if self.selections().as_slice()[index].is_empty()
                    || self.selections().table_columns().is_none()
                {
                    self.html_table_deletion_text(range)
                        .unwrap_or_else(|| Arc::from(""))
                } else {
                    Arc::from("")
                };
                (range, replacement)
            })
            .collect();
        self.apply_selection_edits(edits, group, CollapseTo::Start)
    }

    fn horizontal_grapheme_target(
        &self,
        snapshot: &TextSnapshot,
        from: ByteOffset,
        forward: bool,
    ) -> Result<ByteOffset, EditorDocumentError> {
        let target = if forward {
            next_grapheme_boundary(snapshot, from)?
        } else {
            previous_grapheme_boundary(snapshot, from)?
        };
        let range = TextRange::new(from.min(target), from.max(target)).expect("ordered step");
        let atom = self.table_atom_deletion_range(range, forward)?;
        Ok(if forward { atom.end() } else { atom.start() })
    }

    fn extend_horizontal(
        &mut self,
        forward: bool,
        word: bool,
    ) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let mut targets = Vec::with_capacity(self.presentation.selections.len());
        for selection in self.presentation.selections.as_slice() {
            let focus = if word {
                self.word_target(&snapshot, selection.focus(), forward)?
            } else {
                self.horizontal_grapheme_target(&snapshot, selection.focus(), forward)?
            };
            targets.push(EditorSelection::range(
                &snapshot,
                selection.anchor(),
                focus,
                crate::CaretAffinity::Downstream,
            )?);
        }
        self.state.history.break_group();
        self.presentation.selections = Selections::new(
            &snapshot,
            targets,
            self.presentation.selections.primary_index(),
        )?;
        Ok(self.command_result(false))
    }

    fn move_document_boundary(
        &mut self,
        end: bool,
        extend: bool,
    ) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let focus = if end {
            snapshot.len_bytes()
        } else {
            ByteOffset::ZERO
        };
        let anchor = if extend {
            self.selection().anchor()
        } else {
            focus
        };
        self.set_single_selection(EditorSelection::range(
            &snapshot,
            anchor,
            focus,
            crate::CaretAffinity::Downstream,
        )?);
        self.state.history.break_group();
        Ok(self.command_result(false))
    }

    fn move_left(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.state.history.break_group();
        self.move_horizontal(false)
    }

    /// 每一条选区各走一步。
    ///
    /// 走完之后归一化：两个相邻的光标向左走会撞到同一个偏移，不合并的话下一次
    /// 插入会被 `validate_edits` 拒掉。收敛点仍然只有 `Selections` 一个。
    fn move_horizontal(&mut self, forward: bool) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let primary = self.presentation.selections.primary_index();
        let mut targets = Vec::with_capacity(self.presentation.selections.len());
        for selection in self.presentation.selections.as_slice() {
            let target = if selection.is_empty() {
                self.horizontal_grapheme_target(&snapshot, selection.focus(), forward)?
            } else if forward {
                selection.ordered_range().end()
            } else {
                selection.ordered_range().start()
            };
            targets.push(EditorSelection::cursor(
                &snapshot,
                target,
                crate::CaretAffinity::Downstream,
            )?);
        }
        self.presentation.selections = Selections::new(&snapshot, targets, primary)?;
        Ok(self.command_result(false))
    }

    /// 表格里跳到上一个/下一个单元格。
    ///
    /// **primary 降级。** 「下一个单元格」是相对一个位置说的，N 个光标散在
    /// 不同的表里就没有第二个答案可选。落点是一个光标，所以这条命令顺带把
    /// 选区塌回一条——那正是该有的行为。
    fn move_table_cell(&mut self, previous: bool) -> Result<CommandResult, EditorDocumentError> {
        self.state.history.break_group();
        let Some(target) = self.table_cell_navigation_target(previous) else {
            if !previous && let Some((at, insertion, focus)) = self.table_append_at_focus() {
                let transaction = Transaction::new(
                    self.revision(),
                    [yu_text::Edit::new(TextRange::empty(at), insertion)],
                );
                let applied =
                    self.apply_transaction_with_group(&transaction, HistoryGroup::TableEditing)?;
                self.set_edit_selection(EditorSelection::cursor(
                    applied.result_snapshot(),
                    focus,
                    crate::CaretAffinity::Downstream,
                )?);
                self.state.history.break_group();
                return Ok(self.command_result(true));
            }
            return Ok(self.command_result(false));
        };
        self.set_single_selection(EditorSelection::cursor(
            &self.snapshot(),
            target,
            crate::CaretAffinity::Downstream,
        )?);
        self.state.preferred_x = None;
        Ok(self.command_result(false))
    }

    fn table_cell_navigation_target(&self, previous: bool) -> Option<ByteOffset> {
        if self.source_mode() {
            return None;
        }
        let focus = self.selection().focus();
        let block_index = self.block_index_for_offset(focus)?;
        let block = self.presentation.markdown.blocks().get(block_index)?;
        let table = self
            .html_table_grid(block)
            .or_else(|| yu_markdown::table_for_block(&self.presentation.markdown, block))?;
        let offset = usize::try_from(focus.get()).ok()?;
        let current = table.visible_cell_for_source(offset)?;
        let (_, target) = if previous {
            table.previous_visible_cell(current)?
        } else {
            table.next_visible_cell(current)?
        };
        ByteOffset::try_from(target.start()).ok()
    }

    /// Appends only new source bytes; existing row spelling and alignment stay intact.
    /// Use the final physical row's container prefix (the delimiter for an empty
    /// table), so a list's opening marker is never duplicated on continuation.
    fn table_append_at_focus(&self) -> Option<(ByteOffset, String, ByteOffset)> {
        let block = self
            .presentation
            .markdown
            .blocks()
            .get(self.block_index_for_offset(self.selection().focus())?)?;
        if self.source_mode() {
            return None;
        }
        if let Some(grid) = self.html_table_grid(block) {
            let current = grid.visible_cell_for_source(self.selection().focus().get() as usize)?;
            if current.row() + 1 != grid.visible_row_count()
                || current.column() + 1 != grid.column_count()
            {
                return None;
            }
            let plan = self.html_table_edit_plan(block, crate::TableEdit::InsertRowAfter)?;
            let edit = plan.edits.first()?;
            let insertion = edit.inserted_text().to_owned();
            // The row builder emits only empty th/td tags; target the first
            // cell's content, inside the new row and its existing section.
            let content = insertion.find("</t")?;
            let at = edit.range().start();
            let focus = ByteOffset::new(at.get().checked_add(content as u64)?);
            return Some((at, insertion, focus));
        }
        let table = yu_markdown::table_for_block(&self.presentation.markdown, block)?;
        let current = table.visible_cell_for_source(self.selection().focus().get() as usize)?;
        if current.row() + 1 != table.visible_row_count()
            || current.column() + 1 != table.column_count()
        {
            return None;
        }
        let snapshot = self.snapshot();
        let source = snapshot.as_str();
        let row = table.row_ranges().last()?;
        let first = table
            .rows()
            .last()
            .map_or(table.delimiter(), Vec::as_slice)
            .first()?;
        let prefix = source.get(row.start()..first.start())?.split('|').next()?;
        let end = table.source_range().end();
        let tail = source.get(row.end()..end)?;
        let newline = if source
            .get(table.source_range().start()..end)?
            .contains("\r\n")
        {
            "\r\n"
        } else {
            "\n"
        };
        let mut insertion = String::new();
        if tail.is_empty() {
            insertion.push_str(newline);
        }
        insertion.push_str(prefix);
        insertion.push('|');
        let focus = ByteOffset::try_from(end.checked_add(insertion.len())?).ok()?;
        for _ in 0..table.column_count() {
            insertion.push_str("  |");
        }
        if !tail.is_empty() {
            insertion.push_str(newline);
        }
        Some((ByteOffset::try_from(end).ok()?, insertion, focus))
    }

    fn move_right(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.state.history.break_group();
        self.move_horizontal(true)
    }

    fn move_word_left(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.state.history.break_group();
        self.move_word(false)
    }

    fn move_word_right(&mut self) -> Result<CommandResult, EditorDocumentError> {
        self.state.history.break_group();
        self.move_word(true)
    }

    /// 每一条选区各走一个词。
    ///
    /// 与左右移动同形：非空选区先塌到那一头，空光标才真的走。落点由
    /// [`Self::word_target`] 一家算，两个方向共用同一份行/词边界处理。
    fn move_word(&mut self, forward: bool) -> Result<CommandResult, EditorDocumentError> {
        let snapshot = self.snapshot();
        let primary = self.presentation.selections.primary_index();
        let mut targets = Vec::with_capacity(self.presentation.selections.len());
        for selection in self.presentation.selections.as_slice() {
            let target = if !selection.is_empty() {
                let range = selection.ordered_range();
                if forward { range.end() } else { range.start() }
            } else {
                self.word_target(&snapshot, selection.focus(), forward)?
            };
            targets.push(EditorSelection::cursor(
                &snapshot,
                target,
                crate::CaretAffinity::Downstream,
            )?);
        }
        self.presentation.selections = Selections::new(&snapshot, targets, primary)?;
        Ok(self.command_result(false))
    }

    /// 一个光标按词移动的落点：先在本行里找，找不到就跨到相邻那一行。
    fn word_target(
        &self,
        snapshot: &TextSnapshot,
        focus: ByteOffset,
        forward: bool,
    ) -> Result<ByteOffset, EditorDocumentError> {
        if let Some(target) = self.table_word_target(focus, forward)? {
            return Ok(target);
        }
        let line_index = snapshot.line_index(focus)?;
        let line = source_line(snapshot, focus)?;
        let relative = byte_distance(line.start, focus)?.min(line.content.len());
        let local_target = if forward {
            next_word_boundary(&line.content, relative)
        } else {
            previous_word_boundary(&line.content, relative)
        };
        let moved_within_line = if forward {
            local_target > relative
        } else {
            local_target < relative
        };
        if moved_within_line {
            return offset_plus(line.start, local_target);
        }

        if forward {
            let next_index = line_index.get().saturating_add(1);
            if next_index < snapshot.summary().line_count() {
                let next_line =
                    source_line(snapshot, snapshot.line_start(LineIndex::new(next_index))?)?;
                let local_target = next_word_boundary(&next_line.content, 0);
                offset_plus(next_line.start, local_target)
            } else {
                Ok(snapshot.len_bytes())
            }
        } else if line_index.get() == 0 {
            Ok(line.start)
        } else {
            let previous_line = source_line(
                snapshot,
                snapshot.line_start(LineIndex::new(line_index.get() - 1))?,
            )?;
            let local_target =
                previous_word_boundary(&previous_line.content, previous_line.content.len());
            offset_plus(previous_line.start, local_target)
        }
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
        self.state.last_source_change = None;
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
        self.state.history.break_group();

        // 不扩展时，非空选区先塌到那一头——每一条各塌各的。
        if !extend
            && self
                .presentation
                .selections
                .as_slice()
                .iter()
                .any(|s| !s.is_empty())
        {
            let snapshot = self.snapshot();
            let primary = self.presentation.selections.primary_index();
            let mut collapsed = Vec::with_capacity(self.presentation.selections.len());
            for selection in self.presentation.selections.as_slice() {
                let range = selection.ordered_range();
                let target = match direction {
                    VerticalDirection::Up => range.start(),
                    VerticalDirection::Down => range.end(),
                };
                collapsed.push(EditorSelection::cursor(
                    &snapshot,
                    target,
                    crate::CaretAffinity::Downstream,
                )?);
            }
            self.presentation.selections = Selections::new(&snapshot, collapsed, primary)?;
            self.state.preferred_x = None;
            return Ok(self.command_result(false));
        }

        // **多光标不吃粘滞列。** `preferred_x` 是一个 f32 视觉列，而它按
        // `yu-state` 的模块文档进不了 `Selections`——那个 crate 的依赖只有
        // `yu-core` 与 `yu-text`，一个布局或投影类型都没有。N>1 时每个光标
        // 用自己**当前**的 x，代价是连按 ↓ 穿过一行短行会左漂；N=1 时行为与
        // 多光标之前完全一样（那条路上 `sticky` 就是原来的 `preferred_x`）。
        //
        // 还债条件：做「⌥⌘↑ 在上方加一个光标」时本来就要按光标存列，那时把
        // `Cursor { selection, preferred_x }` 建起来，这一段跟着删。
        let multiple = self.presentation.selections.is_multiple();
        let sticky = if multiple {
            None
        } else {
            self.state.preferred_x.map(PreferredCaretX::value)
        };

        let source = self.snapshot();
        let primary = self.presentation.selections.primary_index();
        let selections: Vec<_> = self.presentation.selections.as_slice().to_vec();
        let mut moved = Vec::with_capacity(selections.len());
        let mut primary_x = None;
        let mut any_moved = false;
        for (index, selection) in selections.iter().copied().enumerate() {
            match self.move_one_vertically(
                selection,
                direction,
                extend,
                config,
                sticky,
                &source,
                &mut load_layout,
            )? {
                Some((next, x)) => {
                    if index == primary {
                        primary_x = Some(x);
                    }
                    moved.push(next);
                    any_moved = true;
                }
                // 走不动的那一条留在原地。单光标时这等价于原来的
                // 「`return Ok(command_result(false))`」；多光标时它是必须的
                // ——文档最后一行上的光标按 ↓ 不动，不能把别的光标也拖住。
                None => moved.push(selection),
            }
        }
        // 一条都没动就什么都不改，**包括 `preferred_x`**：在最后一行按 ↓ 之后
        // 再按 ↑ 仍然要回到原来那一列，多光标之前就是这个行为。
        if !any_moved {
            return Ok(self.command_result(false));
        }
        self.presentation.selections = Selections::new(&source, moved, primary)?;
        self.state.preferred_x = if multiple {
            None
        } else {
            primary_x.map(PreferredCaretX::new)
        };
        Ok(self.command_result(false))
    }

    /// 一条选区纵向走一步。`None` 表示这个方向上没有可去的地方。
    ///
    /// 返回的第二项是这一步用的目标 x，调用方拿它更新粘滞列。
    #[allow(clippy::too_many_arguments)]
    fn move_one_vertically<F>(
        &mut self,
        selection: EditorSelection,
        direction: VerticalDirection,
        extend: bool,
        config: LayoutConfig,
        sticky: Option<f32>,
        source: &TextSnapshot,
        load_layout: &mut F,
    ) -> Result<Option<(EditorSelection, f32)>, EditorDocumentError>
    where
        F: FnMut(&mut Self, usize, LayoutConfig) -> Result<BlockView, EditorDocumentError>,
    {
        let focus = selection.focus();
        let anchor = selection.anchor();
        let Some(block_index) = self.block_index_for_offset(focus) else {
            return Ok(None);
        };
        let projection_bias = match selection.affinity() {
            crate::CaretAffinity::Upstream => Bias::Before,
            crate::CaretAffinity::Downstream => Bias::After,
        };
        let block_count = self.presentation.markdown.blocks().len();
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
            return Ok(None);
        };
        let desired_x = sticky.unwrap_or(current_x);
        let (target, target_width) = {
            let layout = load_layout(self, target_block, config)?;
            let target_line_index = target_line.unwrap_or_else(|| {
                navigable_line_count(&layout, target_block + 1 < block_count).saturating_sub(1)
            });
            let Some(target_line) = layout.lines().get(target_line_index) else {
                return Ok(None);
            };
            let hit = layout.hit_test(LayoutPoint::new(desired_x, target_line.y()))?;
            (hit.source(), target_line.width())
        };

        let affinity = vertical_hit_affinity(desired_x, target_width);
        let next = if extend {
            EditorSelection::range(source, anchor, target, affinity)?
        } else {
            EditorSelection::cursor(source, target, affinity)?
        };
        Ok(Some((next, desired_x)))
    }

    /// 有没有哪一条选区在这个方向上动得了。判据与 [`Self::command_available`]
    /// 的横向那几条同形。
    fn vertical_command_available(&self, direction: VerticalDirection) -> bool {
        self.presentation
            .selections
            .as_slice()
            .iter()
            .any(|selection| self.vertical_available_for(*selection, direction))
    }

    fn vertical_available_for(
        &self,
        selection: EditorSelection,
        direction: VerticalDirection,
    ) -> bool {
        if !selection.is_empty() {
            return true;
        }
        let Some(block_index) = self.block_index_for_offset(selection.focus()) else {
            return false;
        };
        let Some(block) = self.presentation.markdown.blocks().get(block_index) else {
            return false;
        };
        match direction {
            VerticalDirection::Up => block_index > 0 || selection.focus() > block.range().start(),
            VerticalDirection::Down => {
                block_index.saturating_add(1) < self.presentation.markdown.blocks().len()
                    || selection.focus() < block.range().end()
            }
        }
    }

    fn command_result(&self, changed: bool) -> CommandResult {
        CommandResult::with_source_change(
            self.revision(),
            self.selection(),
            changed,
            self.state.last_source_change,
        )
    }

    fn list_prefix(&self, line: &SourceLine) -> Option<ListLinePrefix> {
        let blocks = self.presentation.markdown.blocks();
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
        source_line(&self.snapshot(), self.selection().focus()).ok()
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
        .filter_map(|widget| match widget {
            BlockWidget::Image(image) => Some(image),
            BlockWidget::Checkbox(_) | BlockWidget::Embedded(_) => None,
        })
        .filter_map(|image| image_resolver(image).map(|size| (image.source(), size)))
        .collect()
}

/// `base + delta`，越界当成非法区间。这个加法在词移动里出现四次，
/// 各写一遍就是四份溢出处理。
fn offset_plus(base: ByteOffset, delta: usize) -> Result<ByteOffset, EditorDocumentError> {
    base.checked_add(
        u64::try_from(delta)
            .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?,
    )
    .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))
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

/// 编辑之后每一条选区塌到哪一头。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollapseTo {
    Start,
    End,
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
    /// A worker request was superseded during viewport measurement.
    Cancelled,
    InvalidTablePaste,
    InvalidImageProperties,
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
            Self::Cancelled => formatter.write_str("viewport preparation cancelled"),
            Self::InvalidImageProperties => {
                formatter.write_str("invalid or stale image properties")
            }
            Self::InvalidTablePaste => {
                formatter.write_str("invalid table paste shape, target, or cell source")
            }
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
            | Self::CompositionActive
            | Self::Cancelled
            | Self::InvalidTablePaste
            | Self::InvalidImageProperties => None,
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
    use crate::layout_tokens::LINE_HEIGHT_BODY;

    #[test]
    fn paragraph_cache_does_not_reuse_a_different_theme_or_direction() {
        let mut document = EditorDocument::new("Theme paragraph");
        let config = LayoutConfig::new(320.0, 16.0);
        document.block_layout(0, config).expect("Github layout");
        let builds = document.layout_cache_stats().builds();
        document
            .block_layout(0, config.with_theme(yu_core::ThemeId::Night))
            .expect("Night layout");
        assert_eq!(document.layout_cache_stats().builds(), builds + 1);
        document
            .block_layout(0, config.with_base_direction(yu_layout::BaseDirection::Rtl))
            .expect("RTL layout");
        assert_eq!(document.layout_cache_stats().builds(), builds + 2);
        document
            .block_layout(0, config)
            .expect("retained Github layout");
        assert_eq!(document.layout_cache_stats().builds(), builds + 2);
    }

    #[test]
    fn render_snapshot_reuse_is_bound_to_worker_document_identity_and_revision() {
        let document = EditorDocument::new("worker cache");
        let snapshot = document.capture_render_snapshot();
        let worker = snapshot.clone().into_layout_context();
        assert!(snapshot.can_reuse_layout_context(&worker));

        let mut edited = document;
        edited
            .apply_transaction(&Transaction::new(
                edited.revision(),
                [Edit::new(TextRange::empty(ByteOffset::ZERO), "x")],
            ))
            .expect("edit should apply");
        let edited_snapshot = edited.capture_render_snapshot();
        assert!(!snapshot.can_reuse_layout_context(&edited));
        assert!(!edited_snapshot.can_reuse_layout_context(&worker));
    }
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
        assert!(
            matches!(document.route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
            .expect("last-cell tab route"), KeyRouteResult::Executed(result) if result.changed())
        );
        assert_eq!(document.snapshot().as_str(), format!("{source}|  |  |\n"));
        assert_eq!(
            document.selection().focus(),
            ByteOffset::new(source.len() as u64 + 1)
        );
        document
            .execute(EditorCommand::undo())
            .expect("undo new row");
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn nested_table_navigation_uses_the_same_cells_as_presentation() {
        for (intro, prefix) in [
            ("", "> "),
            ("", "> > "),
            ("- intro\n\n", "  "),
            ("- intro\n\n", "  > "),
        ] {
            let source = format!(
                "{intro}{prefix}| A | B |\n{prefix}| --- | --- |\n{prefix}| 中 | 😀 |\n\nafter\n"
            );
            let mut document = EditorDocument::new(source.clone());
            let cells: Vec<_> = ["A", "B", "中", "😀"]
                .iter()
                .map(|label| source.find(label).expect("cell"))
                .collect();
            set_caret(&mut document, cells[0]);
            for &offset in cells.iter().skip(1) {
                assert!(
                    matches!(
                        document
                            .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
                            .expect("tab"),
                        KeyRouteResult::Executed(_)
                    ),
                    "{prefix:?}"
                );
                assert_eq!(
                    document.selection().focus().get() as usize,
                    offset,
                    "{prefix:?}"
                );
            }
            for &offset in cells.iter().rev().skip(1) {
                assert!(
                    matches!(
                        document
                            .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::SHIFT))
                            .expect("shift tab"),
                        KeyRouteResult::Executed(_)
                    ),
                    "{prefix:?}"
                );
                assert_eq!(
                    document.selection().focus().get() as usize,
                    offset,
                    "{prefix:?}"
                );
            }
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn table_cell_pipe_input_remains_literal_and_undoable() {
        for prefix in ["", "> ", "> > "] {
            let source = format!("{prefix}| A | B |\n{prefix}| --- | --- |\n{prefix}| x | y |\n");
            let mut document = EditorDocument::new(source.clone());
            let at = source.find('x').expect("cell") + 1;
            set_caret(&mut document, at);
            document
                .execute(EditorCommand::insert_text("|羽"))
                .expect("input");
            let expected = source.replacen("x |", "x\\|羽 |", 1);
            assert_eq!(document.snapshot().as_str(), expected);
            let layout = document
                .block_layout(0, LayoutConfig::new(320.0, 16.0))
                .expect("layout");
            assert_eq!(
                layout
                    .table()
                    .expect("table survives pipe input")
                    .column_widths()
                    .len(),
                2
            );
            assert!(layout.visual().text().contains("x|羽"));
            assert!(!layout.visual().text().contains("\\|"));
            document.execute(EditorCommand::undo()).expect("undo");
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn table_pipe_composition_commit_and_cancel_preserve_source() {
        let source = "> | A | B |
> | --- | --- |
> | x | y |
";
        let mut document = EditorDocument::new(source);
        let at = ByteOffset::new((source.find('x').expect("cell") + 1) as u64);
        set_caret(&mut document, at.get() as usize);
        document
            .begin_composition(
                TextRange::empty(at),
                "|中",
                Utf16Range::empty(yu_core::Utf16Offset::new(2)),
            )
            .expect("preedit");
        assert_eq!(document.snapshot().as_str(), source);
        assert!(document.cancel_composition());
        assert_eq!(document.snapshot().as_str(), source);
        document
            .begin_composition(
                TextRange::empty(at),
                "|中",
                Utf16Range::empty(yu_core::Utf16Offset::new(2)),
            )
            .expect("preedit");
        document.commit_composition("|中").expect("commit");
        assert_eq!(
            document.snapshot().as_str(),
            source.replace("x |", r"x\|中 |")
        );
        assert_eq!(
            document.selection().focus().get(),
            at.get() + r"\|中".len() as u64
        );
        document.execute(EditorCommand::undo()).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn table_pipe_encoding_is_per_selection_and_preserves_existing_escapes() {
        let source = "| A | B |
| --- | --- |
| x | y |

outside
";
        for (input, encoded) in [
            ("|e", r"\|e"),
            (r"\|e", r"\|e"),
            ("`a|b`", "`a|b`"),
            ("``a`|b``", "``a`|b``"),
            (r"`a\|b`", r"`a\|b`"),
        ] {
            let mut document = EditorDocument::new(source);
            let snapshot = document.snapshot();
            let offsets = [
                source.find('x').expect("cell") + 1,
                source.find("outside").expect("prose"),
            ];
            let selections = offsets.map(|offset| {
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(offset as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("caret")
            });
            document.set_selections(selections, 1).expect("selections");
            document
                .execute(EditorCommand::insert_text(input))
                .expect("input");
            assert_eq!(
                document.snapshot().as_str(),
                source
                    .replacen("x |", &format!("x{encoded} |"), 1)
                    .replacen("outside", &format!("{input}outside"), 1)
            );
            let layout = document
                .block_layout(0, LayoutConfig::new(320.0, 16.0))
                .expect("layout");
            assert_eq!(layout.table().expect("table").column_widths().len(), 2);
            assert_eq!(
                layout.visual().text().contains('\\'),
                input.starts_with('`') && input.contains('\\')
            );
            document.execute(EditorCommand::undo()).expect("undo");
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn table_backslash_before_separator_stays_in_its_cell() {
        for prefix in ["", "> ", "> > "] {
            for newline in ["\n", "\r\n"] {
                let source = format!(
                    "{prefix}|A|B|{newline}{prefix}|---|---|{newline}{prefix}|x|y|{newline}"
                );
                let mut document = EditorDocument::new(source.clone());
                let at = source.find('x').expect("cell") + 1;
                set_caret(&mut document, at);
                for (encoded, visible) in [(r"x\\|", "x\\y"), (r"x\\\\|", "x\\\\y")] {
                    document
                        .execute(EditorCommand::insert_text("\\"))
                        .expect("input");
                    assert_eq!(document.snapshot().as_str(), source.replace("x|", encoded));
                    let layout = document
                        .block_layout(0, LayoutConfig::new(320.0, 16.0))
                        .expect("layout");
                    assert_eq!(
                        layout
                            .table()
                            .expect("table survives")
                            .column_widths()
                            .len(),
                        2
                    );
                    assert!(layout.visual().text().contains(visible));
                }
                let current = document.snapshot().as_str().to_string();
                document
                    .execute(EditorCommand::move_table_cell_next())
                    .expect("next cell");
                assert_eq!(
                    document.selection().focus().get() as usize,
                    current.find('y').expect("next cell")
                );
                document
                    .execute(EditorCommand::undo())
                    .expect("undo typing group");
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }

    #[test]
    fn table_escape_deletion_is_one_visible_character_and_undoable() {
        for prefix in ["", "> ", "> > "] {
            for escaped in [r"\|", r"\\", r"\*"] {
                for forward in [false, true] {
                    let source =
                        format!("{prefix}|A|B|\n{prefix}|---|---|\n{prefix}|x{escaped}|y|\n");
                    let mut document = EditorDocument::new(source.clone());
                    let start = source.find('x').expect("cell") + 1;
                    set_caret(
                        &mut document,
                        if forward {
                            start
                        } else {
                            start + escaped.len()
                        },
                    );
                    document
                        .execute(if forward {
                            EditorCommand::DeleteForward
                        } else {
                            EditorCommand::DeleteBackward
                        })
                        .expect("delete escape");
                    assert_eq!(document.snapshot().as_str(), source.replace(escaped, ""));
                    assert_eq!(document.selection().focus().get() as usize, start);
                    assert_eq!(
                        document
                            .block_layout(0, LayoutConfig::new(320.0, 16.0))
                            .expect("layout")
                            .table()
                            .expect("table")
                            .column_widths()
                            .len(),
                        2
                    );
                    document.execute(EditorCommand::undo()).expect("undo");
                    assert_eq!(document.snapshot().as_str(), source);
                    document.execute(EditorCommand::redo()).expect("redo");
                    assert_eq!(document.snapshot().as_str(), source.replace(escaped, ""));
                }
            }
        }
    }

    #[test]
    fn table_escape_deletion_preserves_code_and_merges_duplicate_targets() {
        let source = "|A|B|\n|---|---|\n|x\\|z|`c\\|d`|\n";
        let mut document = EditorDocument::new(source);
        let start = source.find("\\|").expect("escape");
        let snapshot = document.snapshot();
        let selections = [start, start + 1].map(|offset| {
            EditorSelection::cursor(
                &snapshot,
                ByteOffset::new(offset as u64),
                crate::CaretAffinity::Downstream,
            )
            .expect("caret")
        });
        document.set_selections(selections, 1).expect("selections");
        document
            .execute(EditorCommand::DeleteForward)
            .expect("delete shared atom");
        assert_eq!(document.snapshot().as_str(), source.replacen("\\|", "", 1));
        assert_eq!(document.selections().len(), 1);
        assert_eq!(document.selection().focus().get() as usize, start);
        document.execute(EditorCommand::undo()).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.selections().len(), 2);
        assert_eq!(document.selections().primary_index(), 1);
        let code_pipe = source.rfind('|').expect("last separator");
        let code_escape = source[..code_pipe].rfind("\\|").expect("code escape");
        set_caret(&mut document, code_escape);
        document
            .execute(EditorCommand::DeleteForward)
            .expect("code slash");
        assert_eq!(
            document.snapshot().as_str(),
            source.replacen("`c\\|d`", "`c|d`", 1)
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
            .resize_hit_test(LayoutPoint::new(10.0, 0.5), 0.0)
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
            &[11.0, 9.0]
        );
        assert_eq!(
            resized.table().expect("resized table").cells()[1]
                .bounds()
                .x(),
            11.0
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
            &[10.0, 10.0]
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
    fn long_document_evicts_geometry_without_losing_heights_or_publications() {
        let source = (0..700)
            .map(|i| format!("paragraph {i} 中文 text\n\n"))
            .collect::<String>();
        let mut owner = EditorDocument::new(source.as_str());
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(300.0, 16.0),
                16.0,
                0.0,
            ))
            .expect("config");
        let top = ViewportSpan::new(0.0, 100.0);
        let original = owner
            .prepare_layout_snapshot(top, &WideShaper)
            .expect("first frame");
        let first_lines = original
            .block(0)
            .expect("first paragraph")
            .layout()
            .lines()
            .to_vec();
        let mut worker = owner.capture_render_snapshot().into_layout_context();
        for _ in 0..5000 {
            if worker.viewport_stats().measured() == worker.viewport_stats().entries() {
                break;
            }
            worker
                .measure_background_paragraphs(&WideShaper, &|_| None, None, &mut || false)
                .expect("batch");
            let published = worker
                .prepare_layout_snapshot(top, &WideShaper)
                .expect("publish");
            assert!(owner.adopt_layout_snapshot(published));
            owner
                .capture_render_snapshot()
                .merge_into_layout_context(&mut worker);
            assert!(owner.layout_cache_stats().entries() <= 256);
            assert!(worker.layout_cache_stats().entries() <= 256);
        }
        assert_eq!(
            worker.viewport_stats().measured(),
            worker.viewport_stats().entries()
        );
        assert!(worker.layout_cache_stats().evicted() > 0);
        let complete = worker
            .prepare_layout_snapshot(top, &WideShaper)
            .expect("complete");
        let height = complete.content_height();
        let tail = worker
            .prepare_layout_snapshot(
                ViewportSpan::new((height - 100.0).max(0.0), 100.0),
                &WideShaper,
            )
            .expect("scroll to evicted paragraph");
        assert_eq!(tail.content_height(), height);
        assert_eq!(
            worker
                .prepare_layout_snapshot(top, &WideShaper)
                .expect("return")
                .content_height(),
            height
        );
        assert_eq!(
            original.block(0).expect("retained frame").layout().lines(),
            first_lines
        );
        assert_eq!(worker.snapshot().as_str(), source);
        assert_eq!(owner.snapshot().as_str(), source);
    }

    #[test]
    fn published_geometry_survives_edits_and_rejects_changed_inputs() {
        let mut owner = EditorDocument::new("# Heading\n\nparagraph\n\n```\ncode\n```\n");
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(300.0, 16.0),
                16.0,
                0.0,
            ))
            .expect("config");
        let geometry = owner
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 500.0), &WideShaper)
            .expect("snapshot");
        let heading = geometry.block(0).expect("heading");
        let caret = heading
            .layout()
            .caret_for_source(ByteOffset::ZERO, Bias::After)
            .expect("caret");
        let request = owner
            .caret_scroll_request_with_shaper(ViewportSpan::new(0.0, 500.0), 4.0, &WideShaper)
            .expect("scroll request");
        assert_eq!(request.caret().height(), heading.caret_height(caret));
        assert!(request.caret().height() > 16.0);
        assert_eq!(
            request.caret().y(),
            heading.document_point(caret.point()).y()
        );
        let original = geometry.source().as_str().to_owned();
        owner.set_resource_geometry_version(1);
        assert!(!owner.adopt_layout_snapshot(Arc::clone(&geometry)));
        owner.set_resource_geometry_version(0);
        assert!(owner.adopt_layout_snapshot(Arc::clone(&geometry)));
        owner
            .execute(EditorCommand::insert_text("edit"))
            .expect("edit");
        assert!(!owner.adopt_layout_snapshot(Arc::clone(&geometry)));
        assert_eq!(geometry.source().as_str(), original);
        assert_eq!(geometry.revision(), Revision::INITIAL);
        assert!(owner.current_layout_snapshot().is_none());
    }

    #[test]
    fn cancelled_projection_never_replaces_a_completed_snapshot() {
        for mode in 0..3 {
            let source = if mode == 1 {
                "# **Heading**\n\nsecond paragraph\n\nthird paragraph"
            } else {
                "first paragraph\n\nsecond paragraph\n\nthird paragraph"
            };
            let mut owner = EditorDocument::new(source);
            owner
                .set_viewport_config(ViewportConfig::new(
                    LayoutConfig::new(300.0, 16.0),
                    16.0,
                    0.0,
                ))
                .expect("config");
            if mode == 2 {
                owner
                    .begin_composition(
                        TextRange::empty(ByteOffset::ZERO),
                        "中文",
                        utf16_range(2, 2),
                    )
                    .expect("composition");
            }
            let viewport = ViewportSpan::new(0.0, 500.0);
            let completed = owner
                .prepare_layout_snapshot(viewport, &WideShaper)
                .expect("completed geometry");
            let height = completed.content_height();
            owner.set_resource_geometry_version(1);
            let mut checks = 0;
            let result = owner.prepare_layout_snapshot_with_images_cancelable(
                viewport,
                &WideShaper,
                &|_| None,
                None,
                &mut || {
                    checks += 1;
                    checks >= 5
                },
            );
            assert!(
                matches!(result, Err(EditorDocumentError::Cancelled)),
                "mode {mode}"
            );
            assert_eq!(checks, 5);
            assert!(owner.current_layout_snapshot().is_none());
            assert_eq!(completed.content_height(), height);
            assert_eq!(completed.source().as_str(), source);
            assert_eq!(owner.snapshot().as_str(), source);
            let next = owner
                .prepare_layout_snapshot(viewport, &WideShaper)
                .expect("resume after cancellation");
            assert_eq!(next.resource_geometry_version(), 1);
            assert!(!Arc::ptr_eq(&completed, &next));
        }
    }

    #[test]
    fn ready_image_size_changes_replace_snapshot_and_move_following_paragraph() {
        let mut owner = EditorDocument::new("![image](a.png)\n\nfollowing paragraph");
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(300.0, 10.0),
                10.0,
                0.0,
            ))
            .expect("config");
        let viewport = ViewportSpan::new(0.0, 500.0);
        let first = owner
            .prepare_layout_snapshot_with_images(
                viewport,
                &WideShaper,
                &|_| ImageIntrinsicSize::new(40, 40).ok(),
                None,
            )
            .expect("first image");
        let source =
            ByteOffset::new(owner.snapshot().as_str().find("following").expect("text") as u64);
        let old_y = first
            .block_for_source(source)
            .expect("following block")
            .content_y();
        let next = owner
            .prepare_layout_snapshot_with_images(
                viewport,
                &WideShaper,
                &|_| ImageIntrinsicSize::new(40, 120).ok(),
                None,
            )
            .expect("replacement image");
        assert!(!Arc::ptr_eq(&first, &next));
        assert!(
            next.block_for_source(source)
                .expect("following block")
                .content_y()
                >= old_y + 79.0
        );
        assert_eq!(
            first
                .block_for_source(source)
                .expect("retained block")
                .content_y(),
            old_y
        );
    }

    #[test]
    fn failed_resource_reveals_source_and_rejects_stale_geometry() {
        let source = "```math\n\\unknownYuCommand{x}\n```\n\nfollowing";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.find("following").expect("paragraph"));
        let revision = document.revision();
        let range = document.block_decorations(0).expect("block").range();
        let viewport = ViewportSpan::new(0.0, 600.0);
        let before = document
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("preview");
        let worker = document.capture_render_snapshot();
        document
            .set_failed_resource_ranges(revision, vec![range])
            .expect("diagnostic");
        assert!(!document.accepts_layout_snapshot(&before));
        assert!(!worker.can_reuse_layout_context(&document));
        let failed = document
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("source");
        assert!(failed.blocks()[0].layout().embedded().is_empty());
        assert!(
            failed.blocks()[0]
                .layout()
                .visual()
                .text()
                .contains("```math")
        );
        assert!(
            failed.blocks()[0]
                .layout()
                .visual()
                .text()
                .contains("unknownYuCommand")
        );
        let mut background = document.capture_render_snapshot().into_layout_context();
        assert_eq!(
            background.block_visual_text(0).expect("worker text").text(),
            document.block_visual_text(0).expect("live text").text()
        );
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.revision(), revision);
        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit outside failed block");
        assert!(
            document
                .set_failed_resource_ranges(revision, vec![range])
                .is_err()
        );
        // An unaffected block must not retain a raw projection shifted from
        // an old diagnostic. The new revision gets a new render request.
        let edited = document
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("edited");
        assert!(!edited.blocks()[0].layout().embedded().is_empty());
    }

    #[test]
    fn toc_projection_updates_layout_without_rewriting_marker_source() {
        let source = "[toc]\n\n# **中文🙂**\n\n### Child\n\nTail";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.len());
        let projection = crate::TocProjection::build(document.markdown()).expect("TOC");
        assert_eq!(&*projection.text, "中文🙂\n  Child");
        for link in projection.links.iter() {
            for offset in link.visual.clone() {
                assert_eq!(projection.target_at(offset), Some(link.target));
            }
        }
        let viewport = ViewportSpan::new(0.0, 600.0);
        let before = document
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("before");
        assert_eq!(
            before.blocks()[0].layout().visual().text(),
            "中文🙂\n  Child\n"
        );
        let at = source.find("Child").expect("heading") + "Child".len();
        set_caret(&mut document, at);
        document
            .execute(EditorCommand::insert_text(" updated"))
            .expect("edit heading");
        let changed = document
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("changed");
        assert_eq!(
            changed.blocks()[0].layout().visual().text(),
            "中文🙂\n  Child updated\n"
        );
        let mut worker = document.capture_render_snapshot().into_layout_context();
        assert_eq!(
            worker.block_visual_text(0).expect("background").text(),
            changed.blocks()[0].layout().visual().text()
        );
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        let restored = document
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("restored");
        assert_eq!(
            restored.blocks()[0].layout().visual().text(),
            before.blocks()[0].layout().visual().text()
        );
        set_caret(&mut document, 2);
        assert!(
            document
                .visual_text_for_visual_state()
                .expect("editing marker")
                .text()
                .contains("[toc]")
        );
        document.set_source_mode(true).expect("source mode");
        assert!(
            document
                .block_visual_text(0)
                .expect("raw")
                .text()
                .contains("[toc]")
        );
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn adjacent_footnote_references_keep_identity_on_both_caret_edges() {
        let source = "正文[^a][^b]\n\n[^a]: first\n\n[^b]: second\n";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.len());
        let references = document.markdown().footnotes().references().to_vec();
        let layout = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 600.0), &WideShaper)
            .expect("layout");
        let block = layout.blocks()[0].layout();
        assert_eq!(block.visual().text(), "正文12\n");
        for reference in &references {
            let cluster = block
                .clusters()
                .iter()
                .find(|cluster| cluster.source() == reference.source)
                .expect("numeric reference cluster");
            for fraction in [0.25, 0.75] {
                let hit = block
                    .hit_test(LayoutPoint::new(
                        cluster.x() + cluster.width() * fraction,
                        cluster.y() + cluster.line_height() / 2.0,
                    ))
                    .expect("hit");
                assert_eq!(hit.content_source(), Some(reference.source));
                assert_eq!(
                    document
                        .markdown()
                        .footnotes()
                        .navigation_target(hit.content_source().expect("reference").start()),
                    document
                        .markdown()
                        .footnotes()
                        .navigation_target(reference.source.start())
                );
            }
        }
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn adjacent_equation_references_keep_object_identity_on_both_caret_edges() {
        let source = "\\eqref{a}\\eqref{b}\n\n$$x\\label{a}$$\n\n$$y\\label{b}$$";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.len());
        let spans: Vec<_> = document
            .block_decorations(0)
            .expect("references")
            .widgets()
            .iter()
            .filter_map(|widget| match widget {
                yu_markdown::BlockWidget::Embedded(span) => Some(*span),
                _ => None,
            })
            .collect();
        assert_eq!(spans.len(), 2);
        let revision = document.revision();
        document
            .set_embedded_sizes(
                revision,
                spans
                    .iter()
                    .map(|span| {
                        (
                            span.source,
                            ImageIntrinsicSize::new(40, 20)
                                .expect("size")
                                .with_baseline(Some(15_000))
                                .expect("baseline"),
                        )
                    })
                    .collect(),
            )
            .expect("sizes");
        let layout = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 600.0), &WideShaper)
            .expect("layout");
        let block = layout.blocks()[0].layout();
        for (i, (span, placed)) in block.embedded().iter().enumerate() {
            let bounds = placed.bounds();
            for fraction in [0.25, 0.75] {
                let hit = block
                    .hit_test(LayoutPoint::new(
                        bounds.x() + bounds.width() * fraction,
                        bounds.y() + bounds.height() / 2.0,
                    ))
                    .expect("hit");
                assert_eq!(hit.content_source(), Some(span.source));
                let target = document
                    .markdown()
                    .equation_reference_target(hit.content_source().expect("object").start())
                    .expect("target");
                assert_eq!(
                    target.start().get() as usize,
                    source
                        .find(if i == 0 { "$$x" } else { "$$y" })
                        .expect("definition")
                );
            }
        }
    }

    #[test]
    fn failed_inline_formula_preserves_neighbor_styles_and_widgets() {
        let source = "**bold** $\\bad$ and $x$ ![image](a.png)\n\nend";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.len());
        let decorations = document.block_decorations(0).expect("block").clone();
        let failed = decorations
            .widgets()
            .iter()
            .find_map(|widget| match widget {
                yu_markdown::BlockWidget::Embedded(span) => Some(span.source),
                _ => None,
            })
            .expect("formula");
        let revision = document.revision();
        document
            .set_failed_resource_ranges(revision, vec![failed])
            .expect("failure");
        let decorations = document.block_decorations(0).expect("projection").clone();
        assert_eq!(
            decorations.widgets().len(),
            2,
            "healthy math and image remain"
        );
        let text = document.block_visual_text(0).expect("visual");
        assert!(text.text().contains("$\\bad$"));
        assert!(!text.text().contains("**"));
        assert!(!text.text().contains("$x$"));
        let layout = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 600.0), &WideShaper)
            .expect("remapped widget identities");
        assert_eq!(layout.blocks()[0].layout().embedded().len(), 1);
        assert_eq!(layout.blocks()[0].layout().images().len(), 1);
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn inline_math_uses_native_baseline_in_shared_line_geometry() {
        let source = "before $x_2$ after\n\nend";
        let mut document = EditorDocument::new(source);
        set_caret(&mut document, source.len());
        let resource = document
            .block_decorations(0)
            .expect("block")
            .widgets()
            .iter()
            .find_map(|w| match w {
                yu_markdown::BlockWidget::Embedded(span) => Some(*span),
                _ => None,
            })
            .expect("math");
        let size = ImageIntrinsicSize::new(40, 30)
            .expect("size")
            .with_baseline(Some(22_000))
            .expect("baseline");
        let revision = document.revision();
        document
            .set_embedded_sizes(revision, vec![(resource.source, size)])
            .expect("dimensions");
        let layout = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 600.0), &WideShaper)
            .expect("layout");
        let block = layout.blocks()[0].layout();
        let object = block.embedded()[0].1.bounds();
        assert_eq!(object.height(), 30.0);
        let line = &block.lines()[block.embedded()[0].1.line()];
        assert!(
            (object.y() + 22.0 - line.y() - line.baseline()).abs() < 0.01,
            "formula baseline must coincide with the shared text line"
        );
        assert!(block.visual().text().contains("before "));
        assert!(block.visual().text().contains(" after"));
        assert!(!block.visual().text().contains("x_2"));
        let hit = block
            .hit_test(LayoutPoint::new(object.x() + 2.0, object.y() + 2.0))
            .expect("hit");
        assert_eq!(hit.source(), resource.source.start());
        assert_eq!(document.snapshot().as_str(), source);
        let worker = document.capture_render_snapshot().into_layout_context();
        assert_eq!(worker.revision(), document.revision());
    }

    #[test]
    fn native_resource_dimensions_move_text_and_share_hit_geometry() {
        let source = "```math\nx^2\n```\n\nfollowing paragraph";
        let mut owner = EditorDocument::new(source);
        let following = source.find("following").expect("text");
        set_caret(&mut owner, following);
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(300.0, 10.0),
                10.0,
                0.0,
            ))
            .expect("config");
        let viewport = ViewportSpan::new(0.0, 600.0);
        let before = owner
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("placeholder");
        let range = before.blocks()[0].layout().embedded()[0].0.source;
        let old_y = before
            .block_for_source(ByteOffset::new(following as u64))
            .expect("following")
            .content_y();
        let revision = owner.revision();
        owner
            .set_embedded_sizes(
                revision,
                vec![(range, ImageIntrinsicSize::new(200, 180).expect("size"))],
            )
            .expect("ready");
        assert!(
            !owner.accepts_layout_snapshot(&before),
            "old resource geometry must not be adopted"
        );
        let ready = owner
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("ready layout");
        let resource = ready.blocks()[0].layout();
        let bounds = resource.embedded()[0].1.bounds();
        assert!((bounds.width() - 200.0).abs() < 0.01);
        assert!((bounds.height() - 180.0).abs() < 0.01);
        assert!(
            ready
                .block_for_source(ByteOffset::new(following as u64))
                .expect("following")
                .content_y()
                > old_y + 100.0
        );
        let hit = resource
            .hit_test(LayoutPoint::new(
                bounds.x() + 1.0,
                bounds.y() + bounds.height() - 1.0,
            ))
            .expect("hit at bottom");
        assert_eq!(hit.source(), range.start());
        assert!(
            hit.image().is_none(),
            "formula must not open image properties"
        );
        assert_eq!(owner.snapshot().as_str(), source);
        assert_eq!(owner.revision(), revision);
        owner
            .set_embedded_sizes(revision, vec![])
            .expect("remove resource");
        let cleared = owner
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("placeholder again");
        assert!(
            cleared.blocks()[0].layout().embedded()[0]
                .1
                .bounds()
                .height()
                < 180.0
        );
        assert!(
            owner
                .set_embedded_sizes(Revision::new(999), vec![])
                .is_err()
        );
        set_caret(&mut owner, source.find("x^2").expect("formula"));
        let editing = owner
            .prepare_layout_snapshot(viewport, &WideShaper)
            .expect("source reveal");
        assert!(editing.blocks()[0].layout().embedded().is_empty());
        assert!(editing.blocks()[0].layout().visual().text().contains("x^2"));
    }

    #[test]
    fn the_wide_shaper_conforms_to_the_shaping_contract() {
        let violations = yu_core::shaping_conformance::audit(&WideShaper);
        assert!(violations.is_empty(), "{violations:#?}");
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
        assert_eq!(document.state.preferred_x, Some(PreferredCaretX::new(10.0)));

        document
            .execute(EditorCommand::move_down())
            .expect("second vertical move should preserve preferred x");
        assert_eq!(document.selection().focus().get(), 24);
        assert_eq!(document.state.preferred_x, Some(PreferredCaretX::new(10.0)));
        assert_eq!(document.revision(), revision);
        assert_eq!(document.snapshot().as_str(), source);

        document
            .execute(EditorCommand::move_up())
            .expect("up should return to the short line");
        assert_eq!(document.selection().focus().get(), 13);
        document
            .execute(EditorCommand::MoveLeft)
            .expect("horizontal movement should clear preferred x");
        assert_eq!(document.state.preferred_x, None);
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
        assert_eq!(document.state.preferred_x, Some(PreferredCaretX::new(20.0)));

        document
            .move_vertical_with_shaper(false, false, config, &WideShaper)
            .expect("second shaped down should preserve preferred x");
        assert_eq!(document.selection().focus().get(), 24);
        assert_eq!(document.state.preferred_x, Some(PreferredCaretX::new(20.0)));
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
    fn fragment_paste_distributes_table_cells_and_isolates_history() {
        for prefix in ["", "> ", "> > "] {
            for eol in ["\n", "\r\n"] {
                let source = format!(
                    "{prefix}| A | B |{eol}{prefix}| --- | --- |{eol}{prefix}| x | y |{eol}"
                );
                let mut document = EditorDocument::new(source.clone());
                let snapshot = document.snapshot();
                let ranges = ['x', 'y'].map(|ch| {
                    let start = source.find(ch).expect("cell") as u64;
                    EditorSelection::range(
                        &snapshot,
                        ByteOffset::new(start + 1),
                        ByteOffset::new(start),
                        crate::CaretAffinity::Upstream,
                    )
                    .expect("reverse cell")
                });
                document.set_selections(ranges, 1).expect("targets");
                document
                    .execute(EditorCommand::PasteFragments(vec![
                        "中文|羽".into(),
                        "👨‍👩‍👧‍👦".into(),
                    ]))
                    .expect("paste");
                let expected = source.replace('x', "中文\\|羽").replace('y', "👨‍👩‍👧‍👦");
                assert_eq!(document.snapshot().as_str(), expected);
                assert_eq!(document.selections().primary_index(), 1);
                let layout = document
                    .block_layout(0, LayoutConfig::new(320.0, 16.0))
                    .expect("layout");
                assert_eq!(
                    layout
                        .table()
                        .expect("table preserved")
                        .column_widths()
                        .len(),
                    2
                );
                assert!(layout.visual().text().contains("中文|羽"));
                document
                    .execute(EditorCommand::insert_text("!"))
                    .expect("typing");
                document
                    .execute(EditorCommand::undo())
                    .expect("undo typing only");
                assert_eq!(document.snapshot().as_str(), expected);
                document.execute(EditorCommand::undo()).expect("undo paste");
                assert_eq!(document.snapshot().as_str(), source);
                for (actual, original) in document.selections().as_slice().iter().zip(ranges) {
                    assert_eq!(
                        (actual.anchor(), actual.focus(), actual.affinity()),
                        (original.anchor(), original.focus(), original.affinity())
                    );
                }
                document.execute(EditorCommand::redo()).expect("redo paste");
                assert_eq!(document.snapshot().as_str(), expected);
            }
        }
    }

    #[test]
    fn fragment_paste_preserves_embedded_newlines_and_mismatch_fallback() {
        let mut document = EditorDocument::new("x y");
        let snapshot = document.snapshot();
        let ranges = [(0, 1), (2, 3)].map(|(a, f)| {
            EditorSelection::range(
                &snapshot,
                ByteOffset::new(a),
                ByteOffset::new(f),
                crate::CaretAffinity::Downstream,
            )
            .expect("range")
        });
        document.set_selections(ranges, 0).expect("targets");
        document
            .execute(EditorCommand::PasteFragments(vec![
                "first\nsecond".into(),
                "🪶\0tail".into(),
            ]))
            .expect("fragments");
        assert_eq!(document.snapshot().as_str(), "first\nsecond 🪶\0tail");
        document.execute(EditorCommand::undo()).expect("undo");
        document
            .execute(EditorCommand::PasteFragments(vec![
                "a".into(),
                "b".into(),
                "c".into(),
            ]))
            .expect("mismatch");
        assert_eq!(document.snapshot().as_str(), "a\nb\nc a\nb\nc");
        document.execute(EditorCommand::undo()).expect("undo");
        document
            .execute(EditorCommand::PasteFragments(Vec::new()))
            .expect("empty payload");
        assert_eq!(document.snapshot().as_str(), "x y");
        document
            .execute(EditorCommand::PasteFragments(vec!["".into(), "q".into()]))
            .expect("empty fragment replaces only its target");
        assert_eq!(document.snapshot().as_str(), " q");
    }

    #[test]
    fn cut_selection_deletion_never_backspaces_empty_carets() {
        let mut document = EditorDocument::new("ab cd ef");
        let snapshot = document.snapshot();
        let ranges = [(0, 2), (4, 4), (6, 8)].map(|(a, f)| {
            EditorSelection::range(
                &snapshot,
                ByteOffset::new(a),
                ByteOffset::new(f),
                crate::CaretAffinity::Downstream,
            )
            .expect("selection")
        });
        document.set_selections(ranges, 1).expect("ranges");
        assert!(document.command_available(&EditorCommand::DeleteSelections));
        document
            .execute(EditorCommand::DeleteSelections)
            .expect("cut");
        assert_eq!(document.snapshot().as_str(), " cd ");
        assert_eq!(document.selection().focus().get(), 2);
        document.execute(EditorCommand::undo()).expect("undo");
        assert_eq!(document.snapshot().as_str(), "ab cd ef");
        assert_eq!(
            document.selections().as_slice(),
            &ranges.map(|s| EditorSelection::range(
                &document.snapshot(),
                s.anchor(),
                s.focus(),
                s.affinity()
            )
            .expect("restored"))
        );
        document.execute(EditorCommand::redo()).expect("redo");
        assert_eq!(document.snapshot().as_str(), " cd ");
        assert!(!document.command_available(&EditorCommand::DeleteSelections));
        let revision = document.revision();
        document
            .execute(EditorCommand::DeleteSelections)
            .expect("empty cut");
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn grouped_history_restores_oriented_selections_and_edit_endpoints() {
        fn endpoints(
            document: &EditorDocument,
        ) -> Vec<(ByteOffset, ByteOffset, crate::CaretAffinity)> {
            document
                .selections()
                .as_slice()
                .iter()
                .map(|s| (s.anchor(), s.focus(), s.affinity()))
                .collect()
        }
        let mut document = EditorDocument::new("ab cd");
        let source = document.snapshot();
        let ranges = [
            EditorSelection::range(
                &source,
                ByteOffset::new(2),
                ByteOffset::ZERO,
                crate::CaretAffinity::Upstream,
            )
            .expect("reverse"),
            EditorSelection::range(
                &source,
                ByteOffset::new(3),
                ByteOffset::new(5),
                crate::CaretAffinity::Downstream,
            )
            .expect("forward"),
        ];
        document.set_selections(ranges, 0).expect("selections");
        let before = endpoints(&document);
        document
            .execute(EditorCommand::insert_text("羽"))
            .expect("replace");
        document
            .execute(EditorCommand::insert_text("🪶"))
            .expect("grouped insert");
        let after = endpoints(&document);
        let edited = document.snapshot().as_str().to_owned();
        for _ in 0..3 {
            set_caret(&mut document, 0);
            document.execute(EditorCommand::undo()).expect("undo");
            assert_eq!(document.snapshot().as_str(), "ab cd");
            assert_eq!(endpoints(&document), before);
            assert_eq!(document.selections().primary_index(), 0);
            assert_eq!(document.selections().revision(), document.revision());
            set_caret(&mut document, 5);
            document.execute(EditorCommand::redo()).expect("redo");
            assert_eq!(document.snapshot().as_str(), edited);
            assert_eq!(endpoints(&document), after);
            assert_eq!(document.selections().primary_index(), 0);
            assert_eq!(document.selections().revision(), document.revision());
        }
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
    fn preedit_in_a_source_separator_keeps_the_committed_paragraph_origin() {
        for source in ["正文\n\n", "正文\n\n\n\n"] {
            let mut document = EditorDocument::new(source);
            let config = LayoutConfig::new(320.0, 16.0);
            document
                .set_viewport_config(ViewportConfig::new(config, 25.6, 0.0))
                .expect("viewport");
            set_caret(&mut document, source.len());
            document
                .begin_composition(
                    TextRange::empty(ByteOffset::new(source.len() as u64)),
                    "中文",
                    utf16_range(2, 2),
                )
                .expect("preedit");
            let index = document
                .composition_block_index()
                .expect("composition block");
            let view = document
                .visible_blocks_with_composition_and_shaper(
                    ViewportSpan::new(0.0, 600.0),
                    &WideShaper,
                )
                .expect("preedit viewport");
            let before = view
                .blocks()
                .iter()
                .find(|block| block.index() == index)
                .expect("visible composition")
                .y();
            let paragraph = document
                .block_layout_with_composition_and_shaper(index, config, &WideShaper)
                .expect("preedit paragraph");
            assert_eq!(paragraph.visual().text(), "中文");
            assert_eq!(
                paragraph.lines().len(),
                1,
                "source blanks must not prefix marked text with display lines"
            );
            assert_eq!(document.snapshot().as_str(), source);
            document.commit_composition("中文").expect("commit");
            let index = document
                .block_index_for_offset(document.selection().focus())
                .expect("committed block");
            let view = document
                .visible_blocks_with_visual_state_and_shaper(
                    ViewportSpan::new(0.0, 600.0),
                    &WideShaper,
                )
                .expect("committed viewport");
            let after = view
                .blocks()
                .iter()
                .find(|block| block.index() == index)
                .expect("visible commit")
                .y();
            assert!(
                (before - after).abs() < 0.01,
                "composition moved vertically: {before} -> {after}"
            );
            assert_eq!(document.snapshot().as_str(), format!("{source}中文"));
        }
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
                .all(|block| if block.index() == span.start {
                    block.height() > 0.0
                } else {
                    block.height() == 0.0
                })
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

    /// **J1 走完整条产品链路**：`apply_transaction` 之后只重扫被改的那个块。
    ///
    /// `yu-markdown` 那几条直接驱动 `parse_incremental`，压的是解析器自己；
    /// 这一条压的是**接线**——`EditorDocument` 有没有真的把上一版文档与
    /// `ChangeSet` 交给它。接线断了不会报错，只会让每次敲键都整篇重解析。
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
        let full = u64::from(document.markdown().reparsed_bytes());
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

        let rescanned = u64::from(document.markdown().reparsed_bytes());
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

    /// 改一条 definition 会让引用它的块重算装饰。
    ///
    /// 这条用例此前断言的是反过来的事——「引用式链接的装饰不再依赖
    /// definition 索引」。那句话与不变量 C6 冲突：parser 只产出**候选**引用，
    /// `[id]` 成不成立要装饰阶段查表。不查表的话 `[没定义]` 会画成一个哪儿也
    /// 去不了的链接；查了表而缓存不失效的话，把 definition 改个名之后那个
    /// 链接还画成链接。
    ///
    /// 代价是**每一次 definition 的内容编辑都清一遍装饰缓存**——它不区分
    /// 「哪些块用到了这条定义」。definition 不常改，先按整清算；真成为热点
    /// 再按标签建反向索引。
    #[test]
    fn a_definition_edit_rebuilds_the_blocks_that_reference_it() {
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
        let shifted_range = shifted.range();
        let shifted_hidden = hidden_spans(shifted);
        assert_eq!(
            shifted_range,
            source_range(paragraph.start().get() + 1, paragraph.end().get() + 1)
        );
        assert_eq!(
            document.decoration_cache_stats().builds(),
            2,
            "标签改了名，`[id]` 不再成立，那一块要重算"
        );
        assert!(
            shifted_hidden.is_empty(),
            "查不到定义的候选不是链接，定界符原样留着"
        );
    }

    /// 与上一条相对：定义**没变**的编辑只把块挪一挪，不重算。
    ///
    /// 失效的判据是引用表的**内容指纹**，不是它的位置。折位置的话每敲一个字
    /// 都要把整篇文档的装饰重算一遍——不报错，只是慢。
    #[test]
    fn an_edit_that_leaves_the_definitions_alone_only_shifts_blocks() {
        let source = "[id]: /docs\n\n[id]\n\n尾巴\n";
        let mut document = EditorDocument::new(source);
        let paragraph = document
            .block_decorations(2)
            .expect("引用式段落的装饰")
            .range();
        assert_eq!(document.decoration_cache_stats().entries(), 1);

        // 改**最后**那一段：定义没动，`[id]` 那一块也没被碰到。挨着块边界
        // 插入会让那一块整个作废（`shift_through` 的规则），那考的就不是
        // 引用表了。
        let tail = source.find("尾巴").expect("尾巴") as u64 + "尾巴".len() as u64;
        let transaction = Transaction::new(
            document.revision(),
            [Edit::new(source_range(tail, tail), "更长")],
        );
        document
            .apply_transaction(&transaction)
            .expect("编辑应当成功");

        assert_eq!(
            document.block_decorations(2).expect("平移过的装饰").range(),
            paragraph,
            "这一块自己没动"
        );
        assert_eq!(
            document.decoration_cache_stats().builds(),
            1,
            "定义没变就不该重算"
        );
    }

    #[test]
    fn toggle_task_is_a_source_transaction_and_rebuilds_task_decorations() {
        let mut document = EditorDocument::new("- [ ] todo\n");
        let decorations = document.block_decorations(0).expect("任务项的装饰");
        assert_eq!(
            hidden_spans(decorations),
            vec![(0, 6)],
            "task prefix is replaced by a checkbox"
        );
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
    fn code_control_placeholders_do_not_rewrite_source_or_prose() {
        let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
        let source =
            format!("{family}\n\n`{family}`\n\n```swift\nlet x = \"{family}\"\n\tend\n```\n");
        let mut document = EditorDocument::new(&source);
        let projected = document.visual_text().expect("projection");
        assert_eq!(
            projected.text().matches(family).count(),
            2,
            "prose and inline code stay joined"
        );
        assert!(projected.text().contains("👨•👩•👧•👦"));
        assert!(projected.text().contains("\tend"), "tab remains a tab");
        assert_eq!(document.snapshot().as_str(), source);
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
        assert_eq!(metrics.blocks()[0].height(), 1.8);

        let shaped = document
            .visible_blocks_with_shaper(ViewportSpan::new(0.0, 2.0), &WideShaper)
            .expect("shaped viewport should measure");
        // Two 1.6pt shaped lines plus the final paragraph's 0.8pt margin.
        // Measurement replaces the one-line metrics estimate without losing it.
        assert_eq!(shaped.blocks()[0].height(), 4.0);
        assert_eq!(shaped.content_height(), 4.0);

        let metrics_again = document
            .visible_blocks(ViewportSpan::new(0.0, 2.0))
            .expect("metrics viewport should remeasure after backend switch");
        assert_eq!(metrics_again.blocks()[0].height(), 1.8);
    }

    /// 块间距折进块高贡献：缝 = max(after(上块), before(下块))，整条折在缝
    /// **上块**的高度里。前缀和自动衔接——每块顶在上块底上，缝藏在高度中，
    /// 光标/滚动/AX 消费的数学不变。间距表以正文行高（line_height × 1.6）为
    /// 单位，这里 line_height = 1.0，于是一「行」间距 = 1.6。
    ///
    /// 块的视觉文本带尾部换行符，排成「内容 + 一个空行」——内容按行数算时
    /// 要带上那一行。
    #[test]
    fn block_spacing_folds_into_viewport_heights() {
        let mut document = EditorDocument::new("# title\n\nparagraph\n");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");

        let snapshot = document
            .visible_blocks(ViewportSpan::new(0.0, 100.0))
            .expect("viewport should measure");
        let blocks = snapshot.blocks();
        assert_eq!(blocks.len(), 3, "标题、空行、段落三块");
        assert_eq!(blocks[0].kind(), BlockKind::Heading { level: 1 });

        // Github headings use one rem; source separators add no height.
        let gap = 1.0;
        assert_eq!(
            blocks[0].height(),
            3.0 + gap + 1.0 / 16.0,
            "首标题段前距 + 内容 2 行 + 缩放后的边框 + 折进来的缝"
        );
        // 空行块自己 2 行（换行符自己占一行，与段落块的尾行同理）；它到下一段
        // 没有缝（两边都是 0）。
        assert_eq!(
            blocks[1].height(),
            0.0,
            "source separator has no reading height"
        );
        // 前缀和衔接：每块的原点就是上块的底。
        assert_eq!(blocks[1].y(), blocks[0].y() + blocks[0].height());
        assert_eq!(blocks[2].y(), blocks[1].y() + blocks[1].height());
        // The final paragraph keeps its own margin inside the page padding.
        assert_eq!(
            blocks[2].height(),
            2.8,
            "two content lines plus trailing margin"
        );
    }

    /// 列表内连续 item 之间间距为 0；item 的段前段后只在列表边界上起作用。
    #[test]
    fn quoted_lists_have_independent_paragraphs_and_source_backed_markers() {
        let source = "> opening\n>\n> - first\n>   - child\n> - last\n\nend\n";
        let mut document = EditorDocument::new(source);
        assert!(document.markdown.blocks().len() >= 6);
        let rendered = document.visual_text().expect("projection");
        assert!(!rendered.text().contains('>'), "{}", rendered.text());
        assert!(!rendered.text().contains("- first"), "{}", rendered.text());
        assert!(!rendered.text().contains("- child"), "{}", rendered.text());
        assert!(rendered.text().contains("first"));
        assert!(rendered.text().contains("child"));
        assert_eq!(document.snapshot().as_str(), source);
        let mut markers = 0;
        for index in 0..document.markdown.blocks().len() {
            let decorations = document.block_decorations(index).expect("decorations");
            if decorations
                .line_ornaments()
                .into_iter()
                .any(|(_, ornament)| matches!(ornament, yu_markdown::BlockOrnament::Marker(_)))
            {
                markers += 1;
            }
        }
        assert_eq!(markers, 3);
    }

    #[test]
    fn ordered_list_cache_updates_after_renumbering_and_undo_without_rebuilding_unrelated_text() {
        let source = "1. first\n1. second\n\noutside\n";
        let mut document = EditorDocument::new(source);
        let config = LayoutConfig::new(400.0, 16.0);
        let second = document
            .block_index_for_source(ByteOffset::new(source.find("second").expect("text") as u64))
            .expect("second");
        let outside = document
            .block_index_for_source(ByteOffset::new(source.find("outside").expect("text") as u64))
            .expect("outside");
        let number = |document: &mut EditorDocument| {
            document
                .block_layout_with_shaper(second, config, &WideShaper)
                .expect("layout")
                .ornaments()
                .marker()
                .expect("number")
                .text()
                .to_owned()
        };
        assert_eq!(number(&mut document), "2.");
        document
            .block_layout_with_shaper(outside, config, &WideShaper)
            .expect("cached unrelated paragraph");
        document
            .set_selection(
                EditorSelection::range(
                    &document.snapshot(),
                    ByteOffset::ZERO,
                    ByteOffset::new(1),
                    crate::CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("select number");
        document
            .execute(EditorCommand::insert_text("7"))
            .expect("replace starting number");
        assert_eq!(
            document.snapshot().as_str(),
            "7. first\n1. second\n\noutside\n"
        );
        let before = document.layout_cache_stats().builds();
        document
            .block_layout_with_shaper(outside, config, &WideShaper)
            .expect("unchanged paragraph");
        assert_eq!(document.layout_cache_stats().builds(), before);
        assert_eq!(
            number(&mut document),
            "8.",
            "source spelling of the second item stays 1."
        );
        document.execute(EditorCommand::undo()).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(number(&mut document), "2.");
        document.execute(EditorCommand::redo()).expect("redo");
        assert_eq!(number(&mut document), "8.");
        set_caret(&mut document, 0);
        document
            .execute(EditorCommand::insert_text("intro\n\n"))
            .expect("unrelated prefix");
        let target = document
            .snapshot()
            .as_str()
            .find("second")
            .expect("second after prefix");
        let target = document
            .block_index_for_source(ByteOffset::new(target as u64))
            .expect("block");
        let before = document.layout_cache_stats().builds();
        assert_eq!(
            document
                .block_layout_with_shaper(target, config, &WideShaper)
                .expect("shifted cached layout")
                .ornaments()
                .marker()
                .expect("number")
                .text(),
            "8."
        );
        assert_eq!(
            document.layout_cache_stats().builds(),
            before,
            "source shifts alone preserve shaping"
        );
        let before_insert = document.snapshot().as_str().to_owned();
        let position = before_insert.find("1. second").expect("item prefix");
        set_caret(&mut document, position);
        document
            .execute(EditorCommand::insert_text("1. middle\n"))
            .expect("insert sibling");
        assert_eq!(
            document.snapshot().as_str(),
            before_insert.replace("1. second", "1. middle\n1. second")
        );
        let offset = document.snapshot().as_str().find("second").expect("second");
        let index = document
            .block_index_for_source(ByteOffset::new(offset as u64))
            .expect("block");
        assert_eq!(
            document
                .block_layout_with_shaper(index, config, &WideShaper)
                .expect("renumbered sibling")
                .ornaments()
                .marker()
                .expect("number")
                .text(),
            "9."
        );
        document
            .execute(EditorCommand::undo())
            .expect("undo inserted item");
        assert_eq!(document.snapshot().as_str(), before_insert);
    }

    #[test]
    fn nested_list_transitions_follow_container_tightness() {
        for (source, pairs) in [
            (
                "> intro\n>\n> - first\n> - last\n>\n> outro\n",
                vec![("first", "last", 8.0), ("last", "outro", 12.8)],
            ),
            (
                "- parent\n  - child\n- last\n\n1. separate\n1. final\n",
                vec![
                    ("parent", "child", 8.0),
                    ("child", "last", 8.0),
                    ("last", "separate", 12.8),
                    ("separate", "final", 8.0),
                ],
            ),
            (
                "- first\n\n  continuation\n\n- next\n",
                vec![
                    ("first", "continuation", 8.0),
                    ("continuation", "next", 8.0),
                ],
            ),
            (
                "- parent\n  > quoted\n  >\n  > second\n- last\n",
                vec![("quoted", "second", 12.8)],
            ),
        ] {
            let mut document = EditorDocument::new(source);
            document
                .set_viewport_config(ViewportConfig::new(
                    LayoutConfig::new(400.0, 16.0),
                    16.0,
                    0.0,
                ))
                .expect("config");
            let geometry = document
                .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &WideShaper)
                .expect("geometry");
            for (upper, lower, expected_gap) in pairs {
                let block = |text| {
                    geometry
                        .block_for_source(
                            ByteOffset::new(source.find(text).expect("source") as u64),
                        )
                        .expect("block")
                };
                let upper = block(upper);
                let lower = block(lower);
                let gap = lower.content_y() - upper.content_y() - upper.layout().height();
                assert!(
                    (gap - expected_gap).abs() < 0.01,
                    "{source:?}: gap {gap}, expected {expected_gap}"
                );
            }
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn quote_containers_cover_internal_gaps_and_follow_parent_indentation() {
        use yu_markdown::PresentationKind;
        let source = "> first\n>\n> second\n\n- item\n\n  > inner\n  >\n  > end\n\noutside\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(400.0, 16.0),
                16.0,
                0.0,
            ))
            .expect("config");
        let geometry = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &WideShaper)
            .expect("geometry");
        let quotes: Vec<_> = geometry
            .containers()
            .iter()
            .filter(|node| node.kind == PresentationKind::Quote)
            .collect();
        assert_eq!(quotes.len(), 2);
        let block = |text| {
            geometry
                .block_for_source(ByteOffset::new(source.find(text).expect("source") as u64))
                .expect("block")
        };
        for (quote, first, last, x) in [
            (quotes[0], "first", "second", 0.0),
            (quotes[1], "inner", "end", 30.0),
        ] {
            let bar = quote.quote_bar.expect("quote bar");
            assert_eq!(bar.x(), x);
            assert!((bar.y() - block(first).content_y()).abs() < 0.01);
            let bottom = block(last).content_y() + block(last).layout().height();
            assert!((bar.bottom() - bottom).abs() < 0.01);
            assert!(!quote.clipped_start && !quote.clipped_end);
            assert!(bar.height() > block(first).layout().height() + block(last).layout().height());
        }
        let outside_y = block("outside").content_y();
        assert!(
            quotes[1].bounds.bottom() < outside_y,
            "outside margin must not extend the quote bar"
        );
        let first_y = quotes[0].bounds.y();
        document
            .execute(EditorCommand::insert_text("prefix\n\n"))
            .expect("edit");
        assert_eq!(
            quotes[0].bounds.y(),
            first_y,
            "published containers stay immutable"
        );
        assert!(!document.accepts_layout_snapshot(&geometry));
    }

    #[test]
    fn mixed_quote_and_list_order_controls_markers_and_continuation_alignment() {
        for (source, marker_before_bar) in [
            ("- > first\n  > second\n", true),
            ("> - first\n>   second\n", false),
        ] {
            let mut document = EditorDocument::new(source);
            let config = LayoutConfig::new(400.0, 16.0);
            document
                .set_viewport_config(ViewportConfig::new(config, 16.0, 0.0))
                .expect("config");
            let geometry = document
                .prepare_layout_snapshot(ViewportSpan::new(0.0, 400.0), &WideShaper)
                .expect("geometry");
            let bar = geometry
                .containers()
                .iter()
                .find_map(|node| node.quote_bar)
                .expect("quote bar");
            let layout = document
                .block_layout_with_shaper(0, config, &WideShaper)
                .expect("paragraph");
            let marker = layout.ornaments().marker().expect("list marker");
            if marker_before_bar {
                assert!(marker.x() + marker.advance() < bar.x());
            } else {
                assert!(marker.x() > bar.right());
            }
            for text in ["first", "second"] {
                let caret = layout
                    .caret_for_source(
                        ByteOffset::new(source.find(text).expect("text") as u64),
                        Bias::After,
                    )
                    .expect("caret");
                assert!(
                    (caret.point().x() - 49.0).abs() < 0.01,
                    "{source:?}: {text} x={}",
                    caret.point().x()
                );
            }
            assert_eq!(document.snapshot().as_str(), source);
        }
        let source = "- intro\n\n  > first\n  > second\n";
        let mut document = EditorDocument::new(source);
        let index = document
            .block_index_for_source(ByteOffset::new(source.find("first").expect("text") as u64))
            .expect("quote paragraph");
        let layout = document
            .block_layout_with_shaper(index, LayoutConfig::new(400.0, 16.0), &WideShaper)
            .expect("continued quote");
        for text in ["first", "second"] {
            let caret = layout
                .caret_for_source(
                    ByteOffset::new(source.find(text).expect("text") as u64),
                    Bias::After,
                )
                .expect("caret");
            assert!(
                (caret.point().x() - 49.0).abs() < 0.01,
                "{text} x={}",
                caret.point().x()
            );
        }
    }

    #[test]
    fn quoted_list_separator_has_height_only_when_it_contains_the_caret() {
        let source = "> - first\n> - last\n>\n> after\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(400.0, 16.0),
                16.0,
                0.0,
            ))
            .expect("config");
        let offset = source.rfind("\n>\n").expect("separator") + 2;
        let index = document
            .block_index_for_source(ByteOffset::new(offset as u64))
            .expect("separator block");
        let full = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 1000.0), &WideShaper)
            .expect("geometry");
        let blank = full.block(index).expect("separator");
        assert_eq!(blank.metadata().kind(), BlockKind::BlankLine);
        assert_eq!(blank.metadata().height(), 0.0);
        let list = full
            .containers()
            .iter()
            .find(|node| matches!(node.kind, yu_markdown::PresentationKind::List { .. }))
            .expect("list container");
        assert!(
            !list.clipped_start && !list.clipped_end,
            "trailing marker is covered by the snapshot"
        );
        let last = full
            .block_for_source(ByteOffset::new(source.find("last").expect("text") as u64))
            .expect("last item");
        assert!(
            (list.bounds.bottom() - last.content_y() - last.layout().height()).abs() < 0.01,
            "list box excludes the external margin"
        );
        set_caret(&mut document, offset);
        let active = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 1000.0), &WideShaper)
            .expect("active geometry");
        assert!(
            active
                .block(index)
                .expect("active empty paragraph")
                .metadata()
                .height()
                >= 25.6
        );
        assert_eq!(
            full.block(index)
                .expect("retained geometry")
                .metadata()
                .height(),
            0.0
        );
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn selecting_through_a_separator_does_not_insert_a_caret_paragraph() {
        for source in ["first\r\n\r\nsecond\r\n", "> first\n>\n> second\n"] {
            let mut document = EditorDocument::new(source);
            document
                .set_viewport_config(ViewportConfig::new(
                    LayoutConfig::new(400.0, 16.0),
                    16.0,
                    0.0,
                ))
                .expect("config");
            let separator = document
                .markdown
                .blocks()
                .iter()
                .position(|block| block.kind() == BlockKind::BlankLine)
                .expect("separator");
            let blank_offset = document
                .markdown
                .blocks()
                .get(separator)
                .expect("blank")
                .range()
                .start();
            let text_offset = ByteOffset::new(source.find("first").expect("text") as u64);
            let baseline = document
                .prepare_layout_snapshot(ViewportSpan::new(0.0, 1000.0), &WideShaper)
                .expect("baseline");
            assert_eq!(
                baseline
                    .block(separator)
                    .expect("blank")
                    .metadata()
                    .height(),
                0.0
            );
            set_caret(&mut document, blank_offset.get() as usize);
            let caret = document
                .prepare_layout_snapshot(ViewportSpan::new(0.0, 1000.0), &WideShaper)
                .expect("caret");
            assert!(caret.block(separator).expect("blank").metadata().height() > 0.0);
            for (anchor, focus) in [(text_offset, blank_offset), (blank_offset, text_offset)] {
                document
                    .set_selection(
                        EditorSelection::range(
                            &document.snapshot(),
                            anchor,
                            focus,
                            crate::CaretAffinity::Downstream,
                        )
                        .expect("range"),
                    )
                    .expect("selection");
                let selected = document
                    .prepare_layout_snapshot(ViewportSpan::new(0.0, 1000.0), &WideShaper)
                    .expect("selected");
                assert_eq!(
                    selected
                        .block(separator)
                        .expect("blank")
                        .metadata()
                        .height(),
                    0.0,
                    "nonempty selection must not create an editing paragraph"
                );
                assert_eq!(
                    selected
                        .block(separator + 1)
                        .expect("following")
                        .content_y(),
                    baseline
                        .block(separator + 1)
                        .expect("following")
                        .content_y()
                );
            }
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn final_block_retains_its_margin_and_closing_container_margin() {
        for (source, expected_margin) in [
            ("last\n", 12.8),
            ("last\n\n", 12.8),
            ("- last\n", 12.8),
            ("> last\n", 12.8),
            ("# last\n", 16.0),
            ("```\nlast\n```\n", 15.0),
        ] {
            let document = EditorDocument::new(source);
            let config = LayoutConfig::new(400.0, 16.0);
            let index = document
                .markdown
                .blocks()
                .iter()
                .enumerate()
                .filter_map(|(index, block)| {
                    (block.kind() != BlockKind::BlankLine).then_some(index)
                })
                .last()
                .expect("content");
            let kind = document.markdown.blocks().get(index).expect("block").kind();
            let height = document.block_box_height(index, 25.6, config);
            let without_margin = 25.6
                + document.block_content_origin(index, config)
                + content_bottom_inset(kind, config);
            assert!(
                (height - without_margin - expected_margin).abs() < 0.001,
                "{source:?}: trailing margin {}",
                height - without_margin
            );
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn intentional_empty_paragraphs_add_paragraph_advance_without_rewriting_source() {
        let mut baseline = None;
        for breaks in 2..=8 {
            let source = format!("before{}after\n", "\n".repeat(breaks));
            let mut document = EditorDocument::new(&source);
            document
                .set_viewport_config(ViewportConfig::new(
                    LayoutConfig::new(400.0, 16.0),
                    16.0,
                    0.0,
                ))
                .expect("config");
            let snapshot = document
                .prepare_layout_snapshot(ViewportSpan::new(0.0, 1000.0), &WideShaper)
                .expect("snapshot");
            let after = snapshot
                .block_for_source(ByteOffset::new(source.find("after").expect("text") as u64))
                .expect("after paragraph")
                .content_y();
            let first = *baseline.get_or_insert(after);
            let expected = first + (breaks / 2 - 1) as f32 * 38.4;
            assert!(
                (after - expected).abs() < 0.01,
                "{breaks} newlines: {after} != {expected}"
            );
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn partial_container_bounds_are_marked_and_stay_with_their_snapshot() {
        use yu_markdown::PresentationKind;
        let source = "> first\n>\n> middle\n>\n> third\n>\n> last\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(400.0, 16.0),
                16.0,
                0.0,
            ))
            .expect("config");
        let full = document
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &WideShaper)
            .expect("full geometry");
        let middle = full
            .block_for_source(ByteOffset::new(source.find("middle").expect("text") as u64))
            .expect("middle");
        document.set_resource_geometry_version(1);
        let part = document
            .prepare_layout_snapshot(
                ViewportSpan::new(middle.content_y() + 2.0, 1.0),
                &WideShaper,
            )
            .expect("partial geometry");
        let quote = part
            .containers()
            .iter()
            .find(|node| node.kind == PresentationKind::Quote)
            .expect("partial quote");
        assert!(quote.clipped_start && quote.clipped_end);
        let full_quote = full
            .containers()
            .iter()
            .find(|node| node.kind == PresentationKind::Quote)
            .expect("full quote");
        assert!(!full_quote.clipped_start && !full_quote.clipped_end);
        assert_eq!(quote.source, full_quote.source);
        assert!(quote.bounds.height() < full_quote.bounds.height());
        assert_eq!(quote.bounds.y(), middle.metadata().y());
    }

    #[test]
    fn list_item_spacing_preserves_inner_and_outer_margins() {
        let mut document = EditorDocument::new("- one\n- two\n\nafter\n");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");

        let snapshot = document
            .visible_blocks(ViewportSpan::new(0.0, 100.0))
            .expect("viewport should measure");
        let blocks = snapshot.blocks();
        assert_eq!(blocks.len(), 4, "两个 item、空行、段落");

        // List paragraphs retain the theme half-rem margin. Content is two lines.
        assert_eq!(blocks[0].height(), 2.5);
        assert_eq!(blocks[1].y(), blocks[0].y() + blocks[0].height());
        // 列表边界：第二个 item 下面是空行，缝 = after(item) = 0.4 行。
        let boundary = 0.5 * LINE_HEIGHT_BODY;
        assert_eq!(blocks[1].height(), 2.0 + boundary);
        assert_eq!(blocks[2].y(), blocks[1].y() + blocks[1].height());
        assert_eq!(
            blocks[2].height(),
            0.0,
            "source separator has no reading height"
        );
    }

    /// 代码块的不同上、下内边距按字号缩放后折进块高，内容在盒里的起点是
    /// `content_origin_y`——caret/选中换算文档坐标时补同一个数。
    #[test]
    fn code_block_vertical_padding_folds_into_viewport_heights() {
        let mut document = EditorDocument::new("```\nbody\n```\n\ntail\n");
        document
            .set_viewport_config(ViewportConfig::new(LayoutConfig::new(80.0, 1.0), 1.0, 0.0))
            .expect("viewport config should be valid");

        let snapshot = document
            .visible_blocks(ViewportSpan::new(0.0, 100.0))
            .expect("viewport should measure");
        let blocks = snapshot.blocks();
        assert!(matches!(
            blocks[0].kind(),
            BlockKind::FencedCodeBlock { .. }
        ));
        let origin = content_origin_y(blocks[0].kind(), LayoutConfig::new(80.0, 1.0));
        assert_eq!(origin, 9.0 / 16.0);
        // The 15pt code margin wins over the 12.8pt paragraph margin.
        let gap = 15.0 / 16.0;
        assert_eq!(
            blocks[0].height(),
            origin + 2.0 + 7.0 / 16.0 + gap,
            "缩放后的上内边距 + 内容 + 下内边距 + 段间距"
        );
        assert_eq!(blocks[1].y(), blocks[0].y() + blocks[0].height());

        // 内容原点折进 caret 的文档坐标：光标落在代码第一个字符上时，y = 前缀
        // （0）+ 上内边距 + 行内偏移（0）。漏掉原点光标整体上移 5pt，不报错。
        set_caret(&mut document, 4);
        let request = document
            .caret_scroll_request(ViewportSpan::new(0.0, 100.0), 0.0)
            .expect("caret scroll request should resolve");
        assert_eq!(request.caret().block(), 0);
        assert_eq!(request.caret().y(), origin);
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
        // 16 = 占位那一行（行高 10 × 正文倍率 1.6），块尾换行符一行同高。
        assert_eq!(placeholder.blocks()[0].height(), 40.0);

        let intrinsic = ImageIntrinsicSize::new(200, 100).expect("image dimensions");
        let ready = document
            .visible_blocks_with_shaper_and_image_resolver(
                ViewportSpan::new(0.0, 100.0),
                &WideShaper,
                |_| Some(intrinsic),
            )
            .expect("ready image viewport should measure");
        // 56 = 40 + 16：40 是图片那一行（200×100 缩到 80 宽就是 40 高），16
        // 是块尾那个换行符自己的行（10 × 正文行高倍率 1.6）。图片 widget 化
        // 之前这里是 50：图片是排完之后另贴上去的盒子，行不知道它有多高，块
        // 高只能取 `max(行盒累加, 图片下沿)`——于是图片压在块尾那一行上面。
        assert_eq!(ready.blocks()[0].height(), 64.0);
        assert_eq!(ready.blocks()[1].y(), 64.0);
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
            BlockKind::Heading { level: 1 }
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

    #[test]
    fn overscan_changes_preserve_measured_block_heights() {
        let mut document = EditorDocument::new("a long wrapped paragraph\n\nsecond paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(12.0, 10.0),
                10.0,
                0.0,
            ))
            .expect("valid render snapshot fixture");
        document
            .visible_blocks_with_shaper(ViewportSpan::new(0.0, 60.0), &WideShaper)
            .expect("valid render snapshot fixture");
        let heights = document.viewport.height_index().clone();
        let stats = document.viewport_stats();
        document
            .set_viewport_overscan(120.0)
            .expect("valid render snapshot fixture");
        assert_eq!(document.viewport.height_index(), &heights);
        assert_eq!(document.viewport_stats(), stats);
        assert_eq!(document.viewport_config().overscan(), 120.0);
        assert!(document.set_viewport_overscan(f32::NAN).is_err());
        assert_eq!(document.viewport_config().overscan(), 120.0);
        assert_eq!(document.viewport.height_index(), &heights);
    }

    #[test]
    fn render_layout_preserves_prefix_heights_and_returns_worker_measurements() {
        let source = format!(
            "{}tail",
            "a long wrapped paragraph with many words\n\n".repeat(20)
        );
        let mut owner = EditorDocument::new(&source);
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(12.0, 10.0),
                10.0,
                0.0,
            ))
            .expect("valid render snapshot fixture");
        owner
            .visible_blocks_with_shaper(ViewportSpan::new(0.0, 60.0), &WideShaper)
            .expect("valid render snapshot fixture");
        let tail = ByteOffset::new((source.len() - 4) as u64);
        owner
            .set_selection(
                EditorSelection::cursor(&owner.snapshot(), tail, crate::CaretAffinity::Downstream)
                    .expect("valid render snapshot fixture"),
            )
            .expect("valid render snapshot fixture");
        let expected = owner
            .caret_scroll_request_with_shaper(ViewportSpan::new(0.0, 60.0), 0.0, &WideShaper)
            .expect("valid render snapshot fixture");
        let mut worker = owner.capture_render_snapshot().into_layout_context();
        let copied = worker
            .caret_scroll_request_with_shaper(ViewportSpan::new(0.0, 60.0), 0.0, &WideShaper)
            .expect("valid render snapshot fixture");
        assert_eq!(
            copied.caret().y(),
            expected.caret().y(),
            "capturing must not reset previously measured prefixes"
        );
        worker
            .visible_blocks_with_shaper(
                ViewportSpan::new(expected.caret().y() - 30.0, 60.0),
                &WideShaper,
            )
            .expect("valid render snapshot fixture");
        let measured = worker
            .caret_scroll_request_with_shaper(ViewportSpan::new(0.0, 60.0), 0.0, &WideShaper)
            .expect("valid render snapshot fixture");
        assert!(measured.caret().y() > expected.caret().y());
        let owner_builds = owner.layout_cache_stats().builds();
        assert!(owner.adopt_measurements(worker.measurements()));
        assert_eq!(
            owner.layout_cache_stats().builds(),
            owner_builds,
            "adoption cannot shape on the owner"
        );
        let adopted = owner
            .caret_scroll_request_with_shaper(ViewportSpan::new(0.0, 60.0), 0.0, &WideShaper)
            .expect("valid render snapshot fixture");
        assert_eq!(adopted.caret().y(), measured.caret().y());
    }

    #[test]
    fn preedit_selection_updates_mapping_without_reshaping() {
        let mut document = EditorDocument::new("text");
        document
            .begin_composition(source_range(0, 0), "中文", utf16_range(0, 0))
            .expect("composition");
        let first = document
            .layout_snapshot_for_source(ByteOffset::ZERO, &WideShaper)
            .expect("first");
        let builds = document.layout_cache_stats().builds();
        document
            .update_composition("中文", utf16_range(2, 2))
            .expect("move preedit selection");
        let second = document
            .layout_snapshot_for_source(ByteOffset::ZERO, &WideShaper)
            .expect("second");
        assert_eq!(document.layout_cache_stats().builds(), builds);
        assert_ne!(
            first.blocks()[0]
                .layout()
                .visual()
                .composition_selection_bytes(),
            second.blocks()[0]
                .layout()
                .visual()
                .composition_selection_bytes()
        );
        assert_eq!(document.snapshot().as_str(), "text");
    }

    #[test]
    fn source_query_prioritizes_one_paragraph_and_projection_cache_reuses_it() {
        let source = format!("{}\n# active heading\n", "paragraph\n\n".repeat(200));
        let mut document = EditorDocument::new(&source);
        let focus = ByteOffset::new(source.find("active").expect("heading") as u64);
        document
            .set_selection(
                EditorSelection::cursor(
                    &document.snapshot(),
                    focus,
                    crate::CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("selection");
        document
            .layout_snapshot_for_source(focus, &WideShaper)
            .expect("active geometry");
        assert_eq!(
            document.viewport_stats().measured(),
            1,
            "no overscan before synchronous active query"
        );
        let builds = document.layout_cache_stats().builds();
        document
            .set_selection(
                EditorSelection::cursor(
                    &document.snapshot(),
                    ByteOffset::new(focus.get() + 1),
                    crate::CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("selection");
        document
            .layout_snapshot_for_source(focus, &WideShaper)
            .expect("active geometry");
        assert_eq!(
            document.layout_cache_stats().builds(),
            builds,
            "same projection only moves caret"
        );
    }

    #[test]
    fn independent_measurements_merge_and_resize_preserves_prefix_estimates() {
        let source = "first paragraph with wrapping\n\nsecond paragraph\n\nlast paragraph\n";
        let mut owner = EditorDocument::new(source);
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(80.0, 16.0),
                16.0,
                0.0,
            ))
            .expect("config");
        // Establish shaped backend before forking independent preparation.
        owner
            .layout_snapshot_for_source(ByteOffset::ZERO, &WideShaper)
            .expect("first");
        let mut worker = owner.capture_render_snapshot().into_layout_context();
        let last = ByteOffset::new(source.find("last").expect("last") as u64);
        worker
            .layout_snapshot_for_source(last, &WideShaper)
            .expect("worker last");
        let second = ByteOffset::new(source.find("second").expect("second") as u64);
        owner
            .layout_snapshot_for_source(second, &WideShaper)
            .expect("owner second");
        assert!(owner.adopt_measurements(worker.measurements()));
        assert_eq!(
            owner.viewport_stats().measured(),
            3,
            "neither participant loses measured paragraphs"
        );
        let old_height = owner.viewport.height_index().total_height();
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(160.0, 16.0),
                16.0,
                0.0,
            ))
            .expect("resize");
        assert_eq!(owner.viewport_stats().measured(), 0);
        assert_eq!(
            owner.viewport.height_index().total_height(),
            old_height,
            "resize keeps geometry as estimates"
        );
        owner
            .layout_snapshot_for_source(last, &WideShaper)
            .expect("viewport target");
        assert_eq!(owner.viewport_stats().measured(), 1);
    }

    #[test]
    fn render_layout_rejects_other_documents_edits_and_changed_visual_state() {
        let mut owner = EditorDocument::new("alpha beta");
        owner
            .visible_blocks_with_shaper(ViewportSpan::new(0.0, 60.0), &WideShaper)
            .expect("valid render snapshot fixture");
        let layout = owner
            .capture_render_snapshot()
            .into_layout_context()
            .measurements();
        let before = owner.viewport_stats();
        let foreign = EditorDocument::new("alpha beta")
            .capture_render_snapshot()
            .into_layout_context()
            .measurements();
        assert!(!owner.adopt_measurements(foreign));
        assert_eq!(owner.viewport_stats(), before);
        let original_selection = owner.selection();
        owner
            .set_selection(
                EditorSelection::cursor(
                    &owner.snapshot(),
                    ByteOffset::new(3),
                    crate::CaretAffinity::Downstream,
                )
                .expect("valid render snapshot fixture"),
            )
            .expect("valid render snapshot fixture");
        assert!(!owner.adopt_measurements(layout.clone()));
        owner
            .set_selection(original_selection)
            .expect("valid render snapshot fixture");
        owner
            .begin_composition(source_range(0, 0), "preedit", utf16_range(0, 0))
            .expect("valid render snapshot fixture");
        assert!(!owner.adopt_measurements(layout.clone()));
        assert!(owner.cancel_composition());
        let config = owner.viewport_config();
        owner
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(40.0, 10.0),
                10.0,
                0.0,
            ))
            .expect("valid render snapshot fixture");
        assert!(!owner.adopt_measurements(layout.clone()));
        owner
            .set_viewport_config(config)
            .expect("valid render snapshot fixture");
        owner
            .execute(EditorCommand::insert_text("changed"))
            .expect("valid render snapshot fixture");
        assert!(!owner.adopt_measurements(layout.clone()));
        owner
            .reset_source("alpha beta")
            .expect("valid render snapshot fixture");
        owner
            .set_selection(original_selection)
            .expect("valid render snapshot fixture");
        assert!(
            !owner.adopt_measurements(layout),
            "reset to Revision::INITIAL must invalidate old source identity"
        );
    }

    #[test]
    fn immutable_render_input_survives_owner_edits_and_rebuilds_on_worker() {
        let mut document = EditorDocument::new("alpha beta 👨‍👩‍👧‍👦");
        document.set_search_query("alpha");
        document.set_search_query("beta");
        document
            .begin_composition(source_range(0, 5), "日本🙂", utf16_range(2, 2))
            .expect("valid render snapshot fixture");
        let expected_source = document.snapshot().as_str().to_owned();
        let expected_selections = document.selections().clone();
        let expected_composition = document.composition().cloned();
        let expected_search_generation = document.search_generation;
        let viewport_before = document.viewport_stats();
        let layout_before = document.layout_cache_stats();
        let input = document.capture_render_snapshot();
        assert_eq!(document.viewport_stats(), viewport_before);
        assert_eq!(document.layout_cache_stats(), layout_before);
        assert!(document.cancel_composition());
        document
            .execute(EditorCommand::insert_text("changed"))
            .expect("valid render snapshot fixture");
        drop(document);
        let owner_thread = std::thread::current().id();
        let rebuilt = std::thread::spawn(move || {
            assert_ne!(std::thread::current().id(), owner_thread);
            input.into_layout_context()
        })
        .join()
        .expect("valid render snapshot fixture");
        assert_eq!(rebuilt.snapshot().as_str(), expected_source);
        assert_eq!(rebuilt.revision(), Revision::INITIAL);
        assert_eq!(rebuilt.selections(), &expected_selections);
        assert_eq!(rebuilt.composition(), expected_composition.as_ref());
        assert_eq!(rebuilt.search_generation, expected_search_generation);
        assert_eq!(
            rebuilt
                .search()
                .expect("valid render snapshot fixture")
                .query(),
            "beta"
        );
    }

    #[test]
    fn replacing_source_rebuilds_search_and_keeps_published_input_immutable() {
        let mut document = EditorDocument::new("old needle");
        document.set_search_query("needle");
        let input = document.capture_render_snapshot();
        document
            .reset_source("needle new needle")
            .expect("replace source");
        let current = document.search().expect("active search");
        assert_eq!(
            current.matches(),
            &[source_range(0, 6), source_range(11, 17)]
        );
        let old = input.into_layout_context();
        assert_eq!(old.snapshot().as_str(), "old needle");
        assert_eq!(
            old.search().expect("old search").matches(),
            &[source_range(4, 10)]
        );
        assert!(!document.adopt_measurements(old.measurements()));
        assert_eq!(document.snapshot().as_str(), "needle new needle");
        assert_eq!(document.history_stats().undo_entries(), 0);
    }

    #[test]
    fn layout_context_preserves_revision_and_shares_search_without_editor_state() {
        let mut document = EditorDocument::new("alpha beta");
        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit should succeed");
        let snapshot = document.snapshot();
        let selection = EditorSelection::range(
            &snapshot,
            ByteOffset::new(0),
            ByteOffset::new(5),
            crate::CaretAffinity::Downstream,
        )
        .expect("selection");
        document.set_selection(selection).expect("selection update");
        document.set_search_query("alpha");

        let clone = document.capture_render_snapshot().into_layout_context();
        assert_eq!(clone.snapshot().as_str(), document.snapshot().as_str());
        assert_eq!(clone.revision(), document.revision());
        assert_eq!(clone.selections(), document.selections());
        assert_eq!(clone.search().map(|search| search.query()), Some("alpha"));
        assert!(Arc::ptr_eq(
            clone.search.as_ref().expect("search"),
            document.search.as_ref().expect("search")
        ));
        assert_eq!(document.history_stats().undo_entries(), 1);
    }
}
