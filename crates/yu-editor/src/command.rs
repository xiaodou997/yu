use std::sync::Arc;

use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};
use yu_core::{ByteOffset, Utf16Range};
use yu_text::TextSnapshot;

use crate::{EditorSelection, SelectionError};

/// The first revision-bound editing commands shared by native and future
/// custom frontends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorCommand {
    InsertText(Arc<str>),
    DeleteBackward,
    DeleteForward,
    MoveLeft,
    MoveRight,
    MoveWordLeft,
    MoveWordRight,
    MoveUp,
    MoveDown,
    MoveUpExtend,
    MoveDownExtend,
    MoveTableCellNext,
    MoveTableCellPrevious,
    InsertNewline,
    IndentList,
    OutdentList,
    Undo,
    Redo,
    ToggleTask { block: usize },
}

/// A source replacement range in native UTF-16 coordinates.
///
/// The old range belongs to the command's input revision and the new range
/// belongs to its result revision. Native mirrors can use this pair to update
/// only the changed source span; commands whose history replay spans multiple
/// edits may intentionally request a full-source fallback at the ABI layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceChange {
    old_range: Utf16Range,
    new_range: Utf16Range,
}

/// The source synchronization scope required by a completed command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceSync {
    #[default]
    None,
    Range(SourceChange),
    Full,
}

impl SourceChange {
    #[must_use]
    pub const fn new(old_range: Utf16Range, new_range: Utf16Range) -> Self {
        Self {
            old_range,
            new_range,
        }
    }

    #[must_use]
    pub const fn old_range(self) -> Utf16Range {
        self.old_range
    }

    #[must_use]
    pub const fn new_range(self) -> Utf16Range {
        self.new_range
    }
}

/// The result of resolving a native key before a platform text-input path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRouteResult {
    Unhandled,
    Executed(CommandResult),
}

impl EditorCommand {
    #[must_use]
    pub fn insert_text(text: impl Into<Arc<str>>) -> Self {
        Self::InsertText(text.into())
    }

    #[must_use]
    pub const fn toggle_task(block: usize) -> Self {
        Self::ToggleTask { block }
    }

    #[must_use]
    pub const fn insert_newline() -> Self {
        Self::InsertNewline
    }

    #[must_use]
    pub const fn indent_list() -> Self {
        Self::IndentList
    }

    #[must_use]
    pub const fn outdent_list() -> Self {
        Self::OutdentList
    }

    #[must_use]
    pub const fn undo() -> Self {
        Self::Undo
    }

    #[must_use]
    pub const fn redo() -> Self {
        Self::Redo
    }

    #[must_use]
    pub const fn move_word_left() -> Self {
        Self::MoveWordLeft
    }

    #[must_use]
    pub const fn move_word_right() -> Self {
        Self::MoveWordRight
    }

    #[must_use]
    pub const fn move_up() -> Self {
        Self::MoveUp
    }

    #[must_use]
    pub const fn move_down() -> Self {
        Self::MoveDown
    }

    #[must_use]
    pub const fn move_up_extend() -> Self {
        Self::MoveUpExtend
    }

    #[must_use]
    pub const fn move_down_extend() -> Self {
        Self::MoveDownExtend
    }

    #[must_use]
    pub const fn move_table_cell_next() -> Self {
        Self::MoveTableCellNext
    }

    #[must_use]
    pub const fn move_table_cell_previous() -> Self {
        Self::MoveTableCellPrevious
    }
}

/// The selection and revision resulting from one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandResult {
    revision: yu_core::Revision,
    selection: EditorSelection,
    changed: bool,
    source_sync: SourceSync,
}

impl CommandResult {
    #[must_use]
    pub const fn revision(self) -> yu_core::Revision {
        self.revision
    }

    #[must_use]
    pub const fn selection(self) -> EditorSelection {
        self.selection
    }

    #[must_use]
    pub const fn changed(self) -> bool {
        self.changed
    }

    /// Returns the changed source span when the command has a local range
    /// suitable for native mirror synchronization.
    #[must_use]
    pub const fn source_change(self) -> Option<SourceChange> {
        match self.source_sync {
            SourceSync::Range(change) => Some(change),
            SourceSync::None | SourceSync::Full => None,
        }
    }

    #[must_use]
    pub const fn source_sync(self) -> SourceSync {
        self.source_sync
    }

    pub(crate) const fn with_source_change(
        revision: yu_core::Revision,
        selection: EditorSelection,
        changed: bool,
        source_change: Option<SourceChange>,
    ) -> Self {
        Self {
            revision,
            selection,
            changed,
            source_sync: if changed {
                match source_change {
                    Some(change) => SourceSync::Range(change),
                    None => SourceSync::None,
                }
            } else {
                SourceSync::None
            },
        }
    }

    pub(crate) const fn requiring_full_source_sync(mut self) -> Self {
        self.source_sync = if self.changed {
            SourceSync::Full
        } else {
            SourceSync::None
        };
        self
    }
}

pub(crate) fn previous_grapheme_boundary(
    snapshot: &TextSnapshot,
    offset: ByteOffset,
) -> Result<ByteOffset, SelectionError> {
    let (offset, total) = validated_offsets(snapshot, offset)?;
    if offset == 0 {
        return Ok(ByteOffset::ZERO);
    }

    let mut cursor = GraphemeCursor::new(offset, total, true);
    let mut chunk = if offset == total {
        snapshot
            .chunk_before(byte_offset(total)?)?
            .ok_or(SelectionError::InvalidRange)?
    } else {
        let mut chunk_cursor = snapshot.chunk_cursor(byte_offset(offset)?)?;
        chunk_cursor.next().ok_or(SelectionError::InvalidRange)?
    };

    loop {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| SelectionError::InvalidRange)?;
        match cursor.prev_boundary(chunk.text(), chunk_start) {
            Ok(Some(boundary)) => return byte_offset(boundary),
            Ok(None) => return Ok(ByteOffset::ZERO),
            Err(error @ (GraphemeIncomplete::PrevChunk | GraphemeIncomplete::PreContext(_))) => {
                if let GraphemeIncomplete::PreContext(context_end) = error {
                    provide_pre_context(snapshot, context_end, &mut cursor)?;
                    continue;
                }
                if chunk_start == 0 {
                    return Ok(ByteOffset::ZERO);
                }
                chunk = snapshot
                    .chunk_before(byte_offset(chunk_start)?)?
                    .ok_or(SelectionError::InvalidRange)?;
            }
            Err(GraphemeIncomplete::NextChunk | GraphemeIncomplete::InvalidOffset) => {
                return Err(SelectionError::InvalidRange);
            }
        }
    }
}

pub(crate) fn next_grapheme_boundary(
    snapshot: &TextSnapshot,
    offset: ByteOffset,
) -> Result<ByteOffset, SelectionError> {
    let (offset, total) = validated_offsets(snapshot, offset)?;
    if offset == total {
        return ByteOffset::try_from(total).map_err(|_| SelectionError::InvalidRange);
    }

    let mut cursor = GraphemeCursor::new(offset, total, true);
    let mut chunks = snapshot.chunk_cursor(byte_offset(offset)?)?;
    let mut chunk = chunks.next().ok_or(SelectionError::InvalidRange)?;

    loop {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| SelectionError::InvalidRange)?;
        match cursor.next_boundary(chunk.text(), chunk_start) {
            Ok(Some(boundary)) => return byte_offset(boundary),
            Ok(None) => return byte_offset(total),
            Err(GraphemeIncomplete::NextChunk) => {
                chunk = chunks.next().ok_or(SelectionError::InvalidRange)?;
            }
            Err(GraphemeIncomplete::PreContext(context_end)) => {
                provide_pre_context(snapshot, context_end, &mut cursor)?;
            }
            Err(GraphemeIncomplete::PrevChunk | GraphemeIncomplete::InvalidOffset) => {
                return Err(SelectionError::InvalidRange);
            }
        }
    }
}

/// Moves to the start of the preceding UAX word-boundary segment.
///
/// Whitespace segments are skipped; punctuation, symbols and emoji remain
/// individually navigable segments instead of being silently merged into a
/// neighboring alphanumeric word.
pub(crate) fn previous_word_boundary(text: &str, offset: usize) -> usize {
    let prefix = &text[..offset];
    for (start, segment) in prefix.split_word_bound_indices().rev() {
        if !segment.chars().all(char::is_whitespace) {
            return start;
        }
    }
    0
}

/// Moves to the end of the next UAX word-boundary segment.
///
/// Leading whitespace is skipped so Option/Control-right behaves like a word
/// command rather than stopping once on every space run.
pub(crate) fn next_word_boundary(text: &str, offset: usize) -> usize {
    let suffix = &text[offset..];
    for (start, segment) in suffix.split_word_bound_indices() {
        if !segment.chars().all(char::is_whitespace) {
            return offset + start + segment.len();
        }
    }
    text.len()
}

fn validated_offsets(
    snapshot: &TextSnapshot,
    offset: ByteOffset,
) -> Result<(usize, usize), SelectionError> {
    snapshot.utf16_offset(offset)?;
    let offset = usize::try_from(offset).map_err(|_| SelectionError::InvalidRange)?;
    let total = usize::try_from(snapshot.len_bytes()).map_err(|_| SelectionError::InvalidRange)?;
    Ok((offset, total))
}

fn byte_offset(offset: usize) -> Result<ByteOffset, SelectionError> {
    ByteOffset::try_from(offset).map_err(|_| SelectionError::InvalidRange)
}

fn provide_pre_context(
    snapshot: &TextSnapshot,
    context_end: usize,
    cursor: &mut GraphemeCursor,
) -> Result<(), SelectionError> {
    if context_end == 0 {
        return Err(SelectionError::InvalidRange);
    }
    let context_offset = byte_offset(context_end)?;
    let chunk = if let Some(chunk) = snapshot.chunk_before(context_offset)? {
        if usize::try_from(chunk.end()).map_err(|_| SelectionError::InvalidRange)? == context_end {
            chunk
        } else {
            let mut chunks = snapshot.chunk_cursor(context_offset)?;
            chunks.next().ok_or(SelectionError::InvalidRange)?
        }
    } else {
        let mut chunks = snapshot.chunk_cursor(context_offset)?;
        chunks.next().ok_or(SelectionError::InvalidRange)?
    };
    let chunk_start = usize::try_from(chunk.start()).map_err(|_| SelectionError::InvalidRange)?;
    let local_end = context_end
        .checked_sub(chunk_start)
        .ok_or(SelectionError::InvalidRange)?;
    if local_end == 0 || local_end > chunk.text().len() {
        return Err(SelectionError::InvalidRange);
    }
    cursor.provide_context(&chunk.text()[..local_end], chunk_start);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::Revision;
    use yu_text::{Edit, TextBuffer, Transaction, retained_snapshot_stats};

    #[test]
    fn grapheme_boundaries_keep_combining_marks_and_zwj_emoji_together() {
        let snapshot = TextBuffer::new("e\u{301} 👩‍🔬x").snapshot();
        let combining_end = ByteOffset::try_from("e\u{301}".len()).expect("offset fits");
        let emoji_start = ByteOffset::try_from("e\u{301} ".len()).expect("offset fits");
        let emoji_end = ByteOffset::try_from("e\u{301} 👩‍🔬".len()).expect("offset fits");

        assert_eq!(
            previous_grapheme_boundary(&snapshot, combining_end).expect("boundary"),
            ByteOffset::ZERO
        );
        assert_eq!(
            next_grapheme_boundary(&snapshot, emoji_start).expect("boundary"),
            emoji_end
        );
        assert_eq!(
            previous_grapheme_boundary(&snapshot, emoji_end).expect("boundary"),
            emoji_start
        );
    }

    #[test]
    fn command_result_exposes_the_revision_contract() {
        let snapshot = TextBuffer::new("").snapshot();
        let selection = EditorSelection::cursor(
            &snapshot,
            ByteOffset::ZERO,
            crate::CaretAffinity::Downstream,
        )
        .expect("empty caret should be valid");
        let result = CommandResult::with_source_change(Revision::INITIAL, selection, false, None);
        assert_eq!(result.revision(), Revision::INITIAL);
        assert_eq!(result.selection(), selection);
        assert!(!result.changed());
        assert_eq!(result.source_sync(), SourceSync::None);

        let zero = Utf16Range::empty(yu_core::Utf16Offset::ZERO);
        let stale_change = SourceChange::new(zero, zero);
        let unchanged = CommandResult::with_source_change(
            Revision::INITIAL,
            selection,
            false,
            Some(stale_change),
        );
        assert_eq!(unchanged.source_sync(), SourceSync::None);
    }

    #[test]
    fn chunk_boundaries_keep_combining_marks_and_zwj_emoji_together() {
        for backend in yu_text::StorageBackend::ALL {
            let mut buffer = TextBuffer::with_backend("e\u{301} 👩‍🔬x", backend);
            buffer
                .apply(&Transaction::new(
                    buffer.revision(),
                    [
                        Edit::new(
                            yu_core::TextRange::new(ByteOffset::new(0), ByteOffset::new(1))
                                .expect("range"),
                            "e",
                        ),
                        Edit::new(
                            yu_core::TextRange::new(ByteOffset::new(4), ByteOffset::new(8))
                                .expect("range"),
                            "👩",
                        ),
                    ],
                ))
                .expect("edit should split the source into chunks");
            let snapshot = buffer.snapshot();
            let combining_end = ByteOffset::new("e\u{301}".len() as u64);
            let emoji_start = ByteOffset::new("e\u{301} ".len() as u64);
            let emoji_end = ByteOffset::new("e\u{301} 👩‍🔬".len() as u64);

            assert_eq!(
                next_grapheme_boundary(&snapshot, ByteOffset::ZERO).expect("boundary"),
                combining_end,
                "backend {backend}"
            );
            assert_eq!(
                next_grapheme_boundary(&snapshot, emoji_start).expect("boundary"),
                emoji_end,
                "backend {backend}"
            );
            assert_eq!(
                previous_grapheme_boundary(&snapshot, emoji_end).expect("boundary"),
                emoji_start,
                "backend {backend}"
            );
        }
    }

    #[test]
    fn word_boundaries_skip_whitespace_but_keep_symbols_as_segments() {
        let text = "hello  世界🙂!";
        assert_eq!(
            previous_word_boundary(text, text.len()),
            "hello  世界🙂".len()
        );
        assert_eq!(
            previous_word_boundary(text, "hello  世界🙂".len()),
            "hello  世界".len()
        );
        assert_eq!(
            previous_word_boundary(text, "hello  世界".len()),
            "hello  世".len()
        );
        assert_eq!(previous_word_boundary(text, "hello  ".len()), 0);
        assert_eq!(next_word_boundary(text, 0), "hello".len());
        assert_eq!(next_word_boundary(text, "hello".len()), "hello  世".len());
        assert_eq!(
            next_word_boundary(text, "hello  世界".len()),
            "hello  世界🙂".len()
        );
    }

    #[test]
    fn chunk_queries_do_not_materialize_a_piece_tree_snapshot() {
        let mut buffer = TextBuffer::new("prefix ".repeat(128) + "e\u{301} 👩‍🔬x");
        buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(
                    yu_core::TextRange::empty(ByteOffset::new(0)),
                    "羽",
                )],
            ))
            .expect("edit should apply");
        let snapshot = buffer.snapshot();
        let before =
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers();
        let end = snapshot.len_bytes();

        let _ = previous_grapheme_boundary(&snapshot, end).expect("boundary");
        let after = retained_snapshot_stats(&[snapshot]).materialized_buffers();
        assert_eq!(after, before);
    }
}
