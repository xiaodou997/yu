use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;
use yu_core::ByteOffset;
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
}

impl EditorCommand {
    #[must_use]
    pub fn insert_text(text: impl Into<Arc<str>>) -> Self {
        Self::InsertText(text.into())
    }
}

/// The selection and revision resulting from one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandResult {
    revision: yu_core::Revision,
    selection: EditorSelection,
    changed: bool,
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

    pub(crate) const fn new(
        revision: yu_core::Revision,
        selection: EditorSelection,
        changed: bool,
    ) -> Self {
        Self {
            revision,
            selection,
            changed,
        }
    }
}

pub(crate) fn previous_grapheme_boundary(
    snapshot: &TextSnapshot,
    offset: ByteOffset,
) -> Result<ByteOffset, SelectionError> {
    let offset = usize::try_from(offset).map_err(|_| SelectionError::InvalidRange)?;
    let offset = ByteOffset::try_from(offset).map_err(|_| SelectionError::InvalidRange)?;
    snapshot.utf16_offset(offset)?;
    if offset == ByteOffset::ZERO {
        return Ok(ByteOffset::ZERO);
    }

    let source = snapshot.as_str();
    let mut previous = 0_usize;
    for (boundary, _) in source.grapheme_indices(true) {
        if boundary >= offset.get() as usize {
            break;
        }
        previous = boundary;
    }
    ByteOffset::try_from(previous).map_err(|_| SelectionError::InvalidRange)
}

pub(crate) fn next_grapheme_boundary(
    snapshot: &TextSnapshot,
    offset: ByteOffset,
) -> Result<ByteOffset, SelectionError> {
    let offset = usize::try_from(offset).map_err(|_| SelectionError::InvalidRange)?;
    let offset = ByteOffset::try_from(offset).map_err(|_| SelectionError::InvalidRange)?;
    snapshot.utf16_offset(offset)?;
    let source = snapshot.as_str();
    for (boundary, _) in source.grapheme_indices(true) {
        if boundary > offset.get() as usize {
            return ByteOffset::try_from(boundary).map_err(|_| SelectionError::InvalidRange);
        }
    }
    ByteOffset::try_from(source.len()).map_err(|_| SelectionError::InvalidRange)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::Revision;
    use yu_text::TextBuffer;

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
        let result = CommandResult::new(Revision::INITIAL, selection, false);
        assert_eq!(result.revision(), Revision::INITIAL);
        assert_eq!(result.selection(), selection);
        assert!(!result.changed());
    }
}
