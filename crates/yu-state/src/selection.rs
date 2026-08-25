use std::error::Error;
use std::fmt;

use yu_core::{
    Affinity, ByteOffset, CaretAffinity, Revision, SourceCaretPosition, TextAnchor, TextRange,
    Utf16Range,
};
use yu_text::{AnchorMapError, ChangeSet, TextPositionError, TextSnapshot};

/// A source selection whose two endpoints belong to one immutable revision.
///
/// `anchor` is the endpoint where the selection started and `focus` is the
/// endpoint currently carrying the caret. Keeping both endpoints (instead of
/// only an ordered range) preserves backward selections and makes extending a
/// selection deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorSelection {
    revision: Revision,
    anchor: ByteOffset,
    focus: ByteOffset,
    affinity: CaretAffinity,
}

impl EditorSelection {
    /// Creates a collapsed selection at a validated source offset.
    pub fn cursor(
        snapshot: &TextSnapshot,
        offset: ByteOffset,
        affinity: CaretAffinity,
    ) -> Result<Self, SelectionError> {
        snapshot.utf16_offset(offset)?;
        Ok(Self {
            revision: snapshot.revision(),
            anchor: offset,
            focus: offset,
            affinity,
        })
    }

    /// Creates a selection from two validated source offsets.
    pub fn range(
        snapshot: &TextSnapshot,
        anchor: ByteOffset,
        focus: ByteOffset,
        affinity: CaretAffinity,
    ) -> Result<Self, SelectionError> {
        snapshot.utf16_offset(anchor)?;
        snapshot.utf16_offset(focus)?;
        Ok(Self {
            revision: snapshot.revision(),
            anchor,
            focus,
            affinity,
        })
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn anchor(self) -> ByteOffset {
        self.anchor
    }

    #[must_use]
    pub const fn focus(self) -> ByteOffset {
        self.focus
    }

    #[must_use]
    pub const fn affinity(self) -> CaretAffinity {
        self.affinity
    }

    #[must_use]
    pub fn ordered_range(self) -> TextRange {
        let (start, end) = if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        };
        TextRange::new(start, end).expect("selection endpoints are ordered")
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.anchor == self.focus
    }

    /// Returns the caret position when this selection is collapsed.
    #[must_use]
    pub fn caret(self) -> Option<SourceCaretPosition> {
        self.is_empty()
            .then(|| SourceCaretPosition::new(self.revision, self.focus, self.affinity))
    }

    /// Converts the selection into native UTF-16 coordinates for one snapshot.
    pub fn utf16_range(&self, snapshot: &TextSnapshot) -> Result<Utf16Range, SelectionError> {
        self.validate_snapshot(snapshot)?;
        let range = self.ordered_range();
        let start = snapshot.utf16_offset(range.start())?;
        let end = snapshot.utf16_offset(range.end())?;
        Utf16Range::new(start, end).ok_or(SelectionError::InvalidRange)
    }

    /// Maps both endpoints through a successful source edit.
    pub fn map_through(
        self,
        change_set: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Self, SelectionError> {
        let anchor = change_set.map_anchor(TextAnchor::new(
            self.revision,
            self.anchor,
            self.endpoint_affinity(true),
        ))?;
        let focus = change_set.map_anchor(TextAnchor::new(
            self.revision,
            self.focus,
            self.endpoint_affinity(false),
        ))?;
        Self::range(snapshot, anchor.offset(), focus.offset(), self.affinity)
    }

    fn endpoint_affinity(self, is_anchor: bool) -> Affinity {
        if self.is_empty() {
            return match self.affinity {
                CaretAffinity::Upstream => Affinity::Before,
                CaretAffinity::Downstream => Affinity::After,
            };
        }

        let anchor_is_start = self.anchor < self.focus;
        let endpoint_is_start = if is_anchor {
            anchor_is_start
        } else {
            !anchor_is_start
        };
        if endpoint_is_start {
            Affinity::Before
        } else {
            Affinity::After
        }
    }

    fn validate_snapshot(&self, snapshot: &TextSnapshot) -> Result<(), SelectionError> {
        if self.revision != snapshot.revision() {
            return Err(SelectionError::StaleRevision {
                expected: snapshot.revision(),
                actual: self.revision,
            });
        }
        snapshot.utf16_offset(self.anchor)?;
        snapshot.utf16_offset(self.focus)?;
        Ok(())
    }
}

/// Errors raised while constructing or mapping a revision-bound selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    Position(TextPositionError),
    AnchorMap(AnchorMapError),
    InvalidRange,
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "selection revision {actual:?} does not match snapshot {expected:?}"
            ),
            Self::Position(error) => error.fmt(formatter),
            Self::AnchorMap(error) => error.fmt(formatter),
            Self::InvalidRange => formatter.write_str("selection endpoints could not form a range"),
        }
    }
}

impl Error for SelectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Position(error) => Some(error),
            Self::AnchorMap(error) => Some(error),
            Self::StaleRevision { .. } | Self::InvalidRange => None,
        }
    }
}

impl From<TextPositionError> for SelectionError {
    fn from(error: TextPositionError) -> Self {
        Self::Position(error)
    }
}

impl From<AnchorMapError> for SelectionError {
    fn from(error: AnchorMapError) -> Self {
        Self::AnchorMap(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange, Utf16Offset};
    use yu_text::{Edit, TextBuffer, Transaction};

    fn range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test range should be ordered")
    }

    #[test]
    fn selection_preserves_backward_direction_and_utf16_coordinates() {
        let buffer = TextBuffer::new("a😊羽");
        let selection = EditorSelection::range(
            &buffer.snapshot(),
            ByteOffset::new(8),
            ByteOffset::new(1),
            CaretAffinity::Downstream,
        )
        .expect("selection endpoints should be valid");

        assert_eq!(selection.ordered_range(), range(1, 8));
        assert_eq!(selection.anchor(), ByteOffset::new(8));
        assert_eq!(selection.focus(), ByteOffset::new(1));
        assert_eq!(
            selection
                .utf16_range(&buffer.snapshot())
                .expect("selection should map to UTF-16"),
            Utf16Range::new(Utf16Offset::new(1), Utf16Offset::new(4))
                .expect("test UTF-16 range should be ordered")
        );
    }

    #[test]
    fn collapsed_downstream_caret_follows_inserted_text() {
        let mut buffer = TextBuffer::new("abc");
        let selection = EditorSelection::cursor(
            &buffer.snapshot(),
            ByteOffset::new(1),
            CaretAffinity::Downstream,
        )
        .expect("caret should be valid");
        let applied = buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(range(1, 1), "XY")],
            ))
            .expect("insert should apply");
        let mapped = selection
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("caret should map");

        assert_eq!(mapped.anchor(), ByteOffset::new(3));
        assert_eq!(mapped.focus(), ByteOffset::new(3));
    }

    #[test]
    fn old_selection_is_rejected_by_a_new_snapshot() {
        let buffer = TextBuffer::new("old");
        let selection = EditorSelection::cursor(
            &buffer.snapshot(),
            ByteOffset::new(3),
            CaretAffinity::Downstream,
        )
        .expect("caret should be valid");
        let mut next = TextBuffer::new("old");
        next.apply(&Transaction::new(
            next.revision(),
            [Edit::new(range(0, 0), "new")],
        ))
        .expect("edit should apply");

        assert!(matches!(
            selection.utf16_range(&next.snapshot()),
            Err(SelectionError::StaleRevision { .. })
        ));
    }
}
