use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};

use crate::TextSnapshot;
use crate::storage::Storage;

/// One replacement expressed in the coordinates of a transaction's base revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    range: TextRange,
    insert: Arc<str>,
}

impl Edit {
    #[must_use]
    pub fn new(range: TextRange, insert: impl Into<Arc<str>>) -> Self {
        Self {
            range,
            insert: insert.into(),
        }
    }

    #[must_use]
    pub fn range(&self) -> TextRange {
        self.range
    }

    #[must_use]
    pub fn inserted_text(&self) -> &str {
        &self.insert
    }
}

/// An atomic set of non-overlapping edits in one base revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    base_revision: Revision,
    edits: Vec<Edit>,
}

impl Transaction {
    #[must_use]
    pub fn new(base_revision: Revision, edits: impl IntoIterator<Item = Edit>) -> Self {
        Self {
            base_revision,
            edits: edits.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn base_revision(&self) -> Revision {
        self.base_revision
    }

    #[must_use]
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub(crate) fn prepare(
        &self,
        current_revision: Revision,
        source: &Storage,
    ) -> Result<PreparedTransaction, EditError> {
        if self.base_revision != current_revision {
            return Err(EditError::StaleRevision {
                expected: current_revision,
                actual: self.base_revision,
            });
        }

        let next_revision = current_revision.next().ok_or(EditError::RevisionOverflow)?;
        let mut edits = self.edits.clone();
        edits.sort_by_key(|edit| (edit.range.start(), edit.range.end()));
        validate_edits(&edits, source)?;

        let mut changes = Vec::with_capacity(edits.len());
        let mut inverse_edits = Vec::with_capacity(edits.len());
        let mut delta = 0_i128;

        for edit in &edits {
            let start =
                usize::try_from(edit.range.start()).map_err(|_| EditError::OffsetOverflow)?;
            let end = usize::try_from(edit.range.end()).map_err(|_| EditError::OffsetOverflow)?;
            let deleted: Arc<str> = Arc::from(source.slice(start..end));

            let new_start_value = i128::from(edit.range.start().get()) + delta;
            let new_end_value = new_start_value
                + i128::try_from(edit.insert.len()).map_err(|_| EditError::OffsetOverflow)?;
            let new_start = offset_from_i128(new_start_value)?;
            let new_end = offset_from_i128(new_end_value)?;
            let new_range = TextRange::new(new_start, new_end).ok_or(EditError::OffsetOverflow)?;

            changes.push(TextChange {
                old_range: edit.range,
                new_range,
            });
            inverse_edits.push(Edit::new(new_range, deleted));

            let old_len = i128::from(edit.range.len());
            let inserted_len =
                i128::try_from(edit.insert.len()).map_err(|_| EditError::OffsetOverflow)?;
            delta += inserted_len - old_len;
        }

        Ok(PreparedTransaction {
            edits,
            change_set: ChangeSet {
                before: current_revision,
                after: next_revision,
                changes,
            },
            inverse: Transaction::new(next_revision, inverse_edits),
        })
    }
}

pub(crate) struct PreparedTransaction {
    pub(crate) edits: Vec<Edit>,
    pub(crate) change_set: ChangeSet,
    pub(crate) inverse: Transaction,
}

fn validate_edits(edits: &[Edit], source: &Storage) -> Result<(), EditError> {
    let mut previous: Option<TextRange> = None;

    for edit in edits {
        let start = usize::try_from(edit.range.start()).map_err(|_| EditError::OffsetOverflow)?;
        let end = usize::try_from(edit.range.end()).map_err(|_| EditError::OffsetOverflow)?;

        if end > source.len_bytes() {
            return Err(EditError::OutOfBounds {
                range: edit.range,
                source_len: source.len_bytes(),
            });
        }
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return Err(EditError::NotUtf8Boundary(edit.range));
        }

        if let Some(previous) = previous {
            let overlaps = edit.range.start() < previous.end();
            let duplicate_empty = edit.range.is_empty()
                && previous.is_empty()
                && edit.range.start() == previous.start();
            if overlaps || duplicate_empty {
                return Err(EditError::OverlappingEdits {
                    first: previous,
                    second: edit.range,
                });
            }
        }
        previous = Some(edit.range);
    }

    Ok(())
}

fn offset_from_i128(value: i128) -> Result<ByteOffset, EditError> {
    let value = u64::try_from(value).map_err(|_| EditError::OffsetOverflow)?;
    Ok(ByteOffset::new(value))
}

/// The source range before an edit and its corresponding range after the edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextChange {
    old_range: TextRange,
    new_range: TextRange,
}

impl TextChange {
    #[must_use]
    pub fn old_range(self) -> TextRange {
        self.old_range
    }

    #[must_use]
    pub fn new_range(self) -> TextRange {
        self.new_range
    }
}

/// Coordinate mapping produced by one successful transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSet {
    before: Revision,
    after: Revision,
    changes: Vec<TextChange>,
}

impl ChangeSet {
    #[must_use]
    pub fn before(&self) -> Revision {
        self.before
    }

    #[must_use]
    pub fn after(&self) -> Revision {
        self.after
    }

    #[must_use]
    pub fn changes(&self) -> &[TextChange] {
        &self.changes
    }

    /// Maps an anchor from this change set's input revision to its output revision.
    pub fn map_anchor(&self, anchor: TextAnchor) -> Result<TextAnchor, AnchorMapError> {
        if anchor.revision() != self.before {
            return Err(AnchorMapError::WrongRevision {
                expected: self.before,
                actual: anchor.revision(),
            });
        }

        let original = anchor.offset().get();
        let mut delta = 0_i128;

        for change in &self.changes {
            let old = change.old_range;
            let start = old.start().get();
            let end = old.end().get();
            let new_start = i128::from(start) + delta;
            let new_end = new_start + i128::from(change.new_range.len());

            if original < start {
                break;
            }

            if old.is_empty() && original == start {
                let mapped = match anchor.affinity() {
                    Affinity::Before => new_start,
                    Affinity::After => new_end,
                };
                return Ok(TextAnchor::new(
                    self.after,
                    offset_from_i128(mapped).map_err(|_| AnchorMapError::OffsetOverflow)?,
                    anchor.affinity(),
                ));
            }

            if original == start || (original > start && original < end) {
                let mapped = match anchor.affinity() {
                    Affinity::Before => new_start,
                    Affinity::After => new_end,
                };
                return Ok(TextAnchor::new(
                    self.after,
                    offset_from_i128(mapped).map_err(|_| AnchorMapError::OffsetOverflow)?,
                    anchor.affinity(),
                ));
            }

            if original >= end {
                delta += i128::from(change.new_range.len()) - i128::from(old.len());
            }
        }

        let mapped = i128::from(original) + delta;
        Ok(TextAnchor::new(
            self.after,
            offset_from_i128(mapped).map_err(|_| AnchorMapError::OffsetOverflow)?,
            anchor.affinity(),
        ))
    }
}

/// Result of a successful atomic edit.
#[derive(Clone, Debug)]
pub struct AppliedTransaction {
    result_snapshot: TextSnapshot,
    change_set: ChangeSet,
    inverse: Transaction,
}

impl AppliedTransaction {
    pub(crate) fn new(
        result_snapshot: TextSnapshot,
        change_set: ChangeSet,
        inverse: Transaction,
    ) -> Self {
        Self {
            result_snapshot,
            change_set,
            inverse,
        }
    }

    #[must_use]
    pub fn result_snapshot(&self) -> &TextSnapshot {
        &self.result_snapshot
    }

    #[must_use]
    pub fn change_set(&self) -> &ChangeSet {
        &self.change_set
    }

    #[must_use]
    pub fn inverse(&self) -> &Transaction {
        &self.inverse
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    OutOfBounds {
        range: TextRange,
        source_len: usize,
    },
    NotUtf8Boundary(TextRange),
    OverlappingEdits {
        first: TextRange,
        second: TextRange,
    },
    OffsetOverflow,
    RevisionOverflow,
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "transaction revision {actual:?} does not match current {expected:?}"
            ),
            Self::OutOfBounds { range, source_len } => {
                write!(
                    formatter,
                    "range {range:?} exceeds source length {source_len}"
                )
            }
            Self::NotUtf8Boundary(range) => {
                write!(formatter, "range {range:?} is not on UTF-8 boundaries")
            }
            Self::OverlappingEdits { first, second } => {
                write!(formatter, "edits {first:?} and {second:?} overlap")
            }
            Self::OffsetOverflow => formatter.write_str("text offset overflow"),
            Self::RevisionOverflow => formatter.write_str("document revision overflow"),
        }
    }
}

impl Error for EditError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorMapError {
    WrongRevision {
        expected: Revision,
        actual: Revision,
    },
    OffsetOverflow,
}

impl fmt::Display for AnchorMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRevision { expected, actual } => write!(
                formatter,
                "anchor revision {actual:?} does not match change set input {expected:?}"
            ),
            Self::OffsetOverflow => formatter.write_str("mapped anchor offset overflow"),
        }
    }
}

impl Error for AnchorMapError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextBuffer;

    fn range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test range should be valid")
    }

    #[test]
    fn multi_edit_transaction_is_atomic_and_invertible() {
        let mut buffer = TextBuffer::new("hello world");
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(range(0, 5), "hi"), Edit::new(range(11, 11), "!")],
        );

        let applied = buffer
            .apply(&transaction)
            .expect("valid transaction should apply");
        assert_eq!(buffer.snapshot().as_str(), "hi world!");

        buffer
            .apply(applied.inverse())
            .expect("inverse transaction should apply");
        assert_eq!(buffer.snapshot().as_str(), "hello world");
    }

    #[test]
    fn failed_transaction_does_not_mutate_buffer() {
        let mut buffer = TextBuffer::new("羽");
        let transaction = Transaction::new(buffer.revision(), [Edit::new(range(1, 2), "x")]);

        assert!(matches!(
            buffer.apply(&transaction),
            Err(EditError::NotUtf8Boundary(_))
        ));
        assert_eq!(buffer.snapshot().as_str(), "羽");
        assert_eq!(buffer.revision(), Revision::INITIAL);
    }

    #[test]
    fn stale_transaction_is_rejected() {
        let mut buffer = TextBuffer::new("abc");
        let transaction = Transaction::new(Revision::new(7), [Edit::new(range(0, 0), "x")]);

        assert!(matches!(
            buffer.apply(&transaction),
            Err(EditError::StaleRevision { .. })
        ));
    }

    #[test]
    fn insertion_respects_anchor_affinity() {
        let mut buffer = TextBuffer::new("abcd");
        let before = TextAnchor::new(buffer.revision(), ByteOffset::new(2), Affinity::Before);
        let after = TextAnchor::new(buffer.revision(), ByteOffset::new(2), Affinity::After);
        let transaction = Transaction::new(buffer.revision(), [Edit::new(range(2, 2), "XY")]);
        let applied = buffer
            .apply(&transaction)
            .expect("valid transaction should apply");

        assert_eq!(
            applied
                .change_set()
                .map_anchor(before)
                .expect("anchor should map")
                .offset(),
            ByteOffset::new(2)
        );
        assert_eq!(
            applied
                .change_set()
                .map_anchor(after)
                .expect("anchor should map")
                .offset(),
            ByteOffset::new(4)
        );
    }

    #[test]
    fn anchors_after_multiple_edits_accumulate_delta() {
        let mut buffer = TextBuffer::new("one two three");
        let anchor = TextAnchor::new(buffer.revision(), ByteOffset::new(13), Affinity::After);
        let transaction = Transaction::new(
            buffer.revision(),
            [
                Edit::new(range(0, 3), "1"),
                Edit::new(range(4, 7), "second"),
            ],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("valid transaction should apply");

        assert_eq!(buffer.snapshot().as_str(), "1 second three");
        assert_eq!(
            applied
                .change_set()
                .map_anchor(anchor)
                .expect("anchor should map")
                .offset(),
            ByteOffset::new(14)
        );
    }
}
