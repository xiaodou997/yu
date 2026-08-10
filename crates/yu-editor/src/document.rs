use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::{Revision, TextRange, Utf16Range};
use yu_markdown::{IncrementalParseError, MarkdownDocument};
use yu_text::{
    AppliedTransaction, EditError, TextBuffer, TextPositionError, TextSnapshot, Transaction,
};

use crate::{
    BlockProjection, CommandResult, CompositionError, CompositionOverlay, EditorCommand,
    EditorSelection, Projection, ProjectionCache, ProjectionCacheStats, ProjectionError,
    SelectionError,
    command::{next_grapheme_boundary, previous_grapheme_boundary},
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
    projections: ProjectionCache,
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
            projections: ProjectionCache::default(),
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

    /// Returns the source-backed projection for a range in the current
    /// revision, building it on first use and reusing it on later queries.
    pub fn projection(&mut self, range: TextRange) -> Result<&Projection, EditorDocumentError> {
        let snapshot = self.snapshot();
        self.projections
            .get_or_build(&snapshot, range)
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
            .get_or_build_block(&snapshot, block)
            .map_err(EditorDocumentError::Projection)
    }

    /// Replaces the selection after checking that it belongs to this revision.
    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), SelectionError> {
        selection.utf16_range(&self.snapshot())?;
        self.selection = selection;
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
        let applied = self.buffer.apply(transaction)?;
        let incremental = yu_markdown::parse_incremental(
            &self.markdown,
            applied.result_snapshot(),
            applied.change_set(),
        )?;
        self.selection = self
            .selection
            .map_through(applied.change_set(), applied.result_snapshot())?;
        self.projections
            .map_through(applied.change_set(), applied.result_snapshot())
            .map_err(EditorDocumentError::Projection)?;
        self.projections.retain_blocks(incremental.document());
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
        let applied = self.apply_transaction(&transaction)?;
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
        Ok(applied)
    }

    /// Drops the active overlay without changing source or revision.
    #[must_use]
    pub fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    /// Replaces the source for a newly opened document and resets its revision.
    pub fn reset_source(&mut self, source: impl Into<String>) -> Result<(), EditorDocumentError> {
        if self.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        self.buffer = TextBuffer::new(source);
        self.markdown = yu_markdown::parse(&self.buffer.snapshot());
        self.projections.clear();
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
        }
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
        let applied = self.apply_transaction(&transaction)?;
        self.selection = EditorSelection::cursor(
            applied.result_snapshot(),
            offset,
            crate::CaretAffinity::Downstream,
        )?;
        Ok(self.command_result(true))
    }

    fn delete_backward(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let range = if self.selection.is_empty() {
            let start = previous_grapheme_boundary(&self.snapshot(), self.selection.focus())?;
            TextRange::new(start, self.selection.focus())
                .expect("previous grapheme boundary must precede caret")
        } else {
            self.selection.ordered_range()
        };
        self.delete_range(range)
    }

    fn delete_forward(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let range = if self.selection.is_empty() {
            let end = next_grapheme_boundary(&self.snapshot(), self.selection.focus())?;
            TextRange::new(self.selection.focus(), end)
                .expect("next grapheme boundary must follow caret")
        } else {
            self.selection.ordered_range()
        };
        self.delete_range(range)
    }

    fn delete_range(&mut self, range: TextRange) -> Result<CommandResult, EditorDocumentError> {
        if range.is_empty() {
            return Ok(self.command_result(false));
        }
        let transaction = Transaction::new(
            self.revision(),
            [yu_text::Edit::new(range, Arc::<str>::from(""))],
        );
        let applied = self.apply_transaction(&transaction)?;
        self.selection = EditorSelection::cursor(
            applied.result_snapshot(),
            range.start(),
            crate::CaretAffinity::Downstream,
        )?;
        Ok(self.command_result(true))
    }

    fn move_left(&mut self) -> Result<CommandResult, EditorDocumentError> {
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
}

/// Errors raised while coordinating canonical edits and composition state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorDocumentError {
    Composition(CompositionError),
    Edit(EditError),
    Markdown(IncrementalParseError),
    Position(TextPositionError),
    Projection(ProjectionError),
    Selection(SelectionError),
    BlockOutOfBounds { index: usize, blocks: usize },
    CompositionNotActive,
    CompositionActive,
}

impl fmt::Display for EditorDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Composition(error) => error.fmt(formatter),
            Self::Edit(error) => error.fmt(formatter),
            Self::Markdown(error) => error.fmt(formatter),
            Self::Position(error) => error.fmt(formatter),
            Self::Projection(error) => error.fmt(formatter),
            Self::Selection(error) => error.fmt(formatter),
            Self::BlockOutOfBounds { index, blocks } => {
                write!(
                    formatter,
                    "Markdown block index {index} is outside {blocks} blocks"
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
            Self::Markdown(error) => Some(error),
            Self::Position(error) => Some(error),
            Self::Projection(error) => Some(error),
            Self::Selection(error) => Some(error),
            Self::BlockOutOfBounds { .. }
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
    use yu_core::{ByteOffset, Utf16Offset};
    use yu_markdown::BlockKind;
    use yu_text::Edit;

    fn source_range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test source range should be ordered")
    }

    fn utf16_range(start: u64, end: u64) -> Utf16Range {
        Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end))
            .expect("test UTF-16 range should be ordered")
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
                BlockProjection::Inline(_) => panic!("fenced code must not use inline projection"),
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
            BlockProjection::Inline(_) => panic!("fenced code must use code projection"),
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
            BlockProjection::Inline(_) => panic!("fenced code must use code projection"),
        };
        assert_eq!(new_content.start().get(), old_content.start().get() + 3);
        assert_eq!(new_content.end().get(), old_content.end().get() + 3);
        assert_eq!(document.projection_cache_stats().builds(), 1);
        assert_eq!(document.projection_cache_stats().remapped(), 1);
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
