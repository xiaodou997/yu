use std::error::Error;
use std::fmt;

use yu_core::{LineIndex, Revision, TextRange, Utf16Offset, Utf16Range};
use yu_text::{TextPositionError, TextSnapshot};

/// A native UTF-16 position bound to one immutable document revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityTextPosition {
    revision: Revision,
    offset: Utf16Offset,
}

impl AccessibilityTextPosition {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn offset(self) -> Utf16Offset {
        self.offset
    }
}

/// A native UTF-16 range bound to one immutable document revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityTextRange {
    revision: Revision,
    range: Utf16Range,
}

impl AccessibilityTextRange {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn range(self) -> Utf16Range {
        self.range
    }
}

/// Synchronous text queries exposed to a platform accessibility adapter.
///
/// The adapter must create a fresh instance for the revision it is serving.
/// Geometry and visible ranges remain layout responsibilities and are not part
/// of this source-coordinate model.
#[derive(Clone, Debug)]
pub struct AccessibilityTextSnapshot {
    source: TextSnapshot,
    selection: TextRange,
    selection_utf16: Utf16Range,
}

impl AccessibilityTextSnapshot {
    pub fn new(source: TextSnapshot, selection: TextRange) -> Result<Self, AccessibilityTextError> {
        let selection_utf16 = source_range_to_utf16(&source, selection)?;
        Ok(Self {
            source,
            selection,
            selection_utf16,
        })
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    #[must_use]
    pub fn number_of_characters(&self) -> Utf16Offset {
        self.source.summary().utf16_units()
    }

    #[must_use]
    pub fn full_range(&self) -> AccessibilityTextRange {
        AccessibilityTextRange {
            revision: self.revision(),
            range: Utf16Range::new(Utf16Offset::ZERO, self.number_of_characters())
                .expect("zero must not exceed the Snapshot UTF-16 length"),
        }
    }

    #[must_use]
    pub fn selected_range(&self) -> AccessibilityTextRange {
        AccessibilityTextRange {
            revision: self.revision(),
            range: self.selection_utf16,
        }
    }

    #[must_use]
    pub fn selected_source_range(&self) -> TextRange {
        self.selection
    }

    pub fn bind_position(
        &self,
        offset: Utf16Offset,
    ) -> Result<AccessibilityTextPosition, AccessibilityTextError> {
        self.source.byte_offset_for_utf16(offset)?;
        Ok(AccessibilityTextPosition {
            revision: self.revision(),
            offset,
        })
    }

    pub fn bind_range(
        &self,
        range: Utf16Range,
    ) -> Result<AccessibilityTextRange, AccessibilityTextError> {
        utf16_range_to_source(&self.source, range)?;
        Ok(AccessibilityTextRange {
            revision: self.revision(),
            range,
        })
    }

    pub fn range_for_source(
        &self,
        range: TextRange,
    ) -> Result<AccessibilityTextRange, AccessibilityTextError> {
        Ok(AccessibilityTextRange {
            revision: self.revision(),
            range: source_range_to_utf16(&self.source, range)?,
        })
    }

    pub fn source_range(
        &self,
        range: AccessibilityTextRange,
    ) -> Result<TextRange, AccessibilityTextError> {
        self.validate_revision(range.revision)?;
        utf16_range_to_source(&self.source, range.range)
    }

    pub fn text_for_range(
        &self,
        range: AccessibilityTextRange,
    ) -> Result<String, AccessibilityTextError> {
        let source_range = self.source_range(range)?;
        collect_text(&self.source, source_range)
    }

    pub fn line_for_position(
        &self,
        position: AccessibilityTextPosition,
    ) -> Result<LineIndex, AccessibilityTextError> {
        self.validate_revision(position.revision)?;
        let byte = self.source.byte_offset_for_utf16(position.offset)?;
        Ok(self.source.line_index(byte)?)
    }

    pub fn range_for_line(
        &self,
        line: LineIndex,
    ) -> Result<AccessibilityTextRange, AccessibilityTextError> {
        let start = self.source.line_start(line)?;
        let line_count = self.source.summary().line_count();
        let end = if line.get().saturating_add(1) < line_count {
            self.source
                .line_start(LineIndex::new(line.get().saturating_add(1)))?
        } else {
            self.source.len_bytes()
        };
        self.range_for_source(
            TextRange::new(start, end).expect("ordered line boundaries must form a range"),
        )
    }

    fn validate_revision(&self, actual: Revision) -> Result<(), AccessibilityTextError> {
        let expected = self.revision();
        if actual != expected {
            return Err(AccessibilityTextError::StaleRevision { expected, actual });
        }
        Ok(())
    }
}

fn source_range_to_utf16(
    source: &TextSnapshot,
    range: TextRange,
) -> Result<Utf16Range, AccessibilityTextError> {
    let start = source.utf16_offset(range.start())?;
    let end = source.utf16_offset(range.end())?;
    Utf16Range::new(start, end).ok_or(AccessibilityTextError::InvalidSourceRange(range))
}

fn utf16_range_to_source(
    source: &TextSnapshot,
    range: Utf16Range,
) -> Result<TextRange, AccessibilityTextError> {
    let start = source.byte_offset_for_utf16(range.start())?;
    let end = source.byte_offset_for_utf16(range.end())?;
    TextRange::new(start, end).ok_or(AccessibilityTextError::InvalidUtf16Range(range))
}

fn collect_text(source: &TextSnapshot, range: TextRange) -> Result<String, AccessibilityTextError> {
    let start =
        usize::try_from(range.start()).map_err(|_| AccessibilityTextError::OffsetOverflow)?;
    let end = usize::try_from(range.end()).map_err(|_| AccessibilityTextError::OffsetOverflow)?;
    let capacity = end.saturating_sub(start);
    let mut result = String::with_capacity(capacity);

    for chunk in source.chunk_cursor(range.start())? {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| AccessibilityTextError::OffsetOverflow)?;
        if chunk_start >= end {
            break;
        }
        let chunk_end = chunk_start.saturating_add(chunk.text().len());
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            result.push_str(&chunk.text()[local_start..local_end]);
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessibilityTextError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    InvalidSourceRange(TextRange),
    InvalidUtf16Range(Utf16Range),
    Position(TextPositionError),
    OffsetOverflow,
}

impl fmt::Display for AccessibilityTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "accessibility query revision {actual:?} does not match {expected:?}"
            ),
            Self::InvalidSourceRange(range) => {
                write!(formatter, "invalid accessibility source range {range:?}")
            }
            Self::InvalidUtf16Range(range) => {
                write!(formatter, "invalid accessibility UTF-16 range {range:?}")
            }
            Self::Position(error) => error.fmt(formatter),
            Self::OffsetOverflow => formatter.write_str("accessibility offset overflow"),
        }
    }
}

impl Error for AccessibilityTextError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Position(error) => Some(error),
            Self::StaleRevision { .. }
            | Self::InvalidSourceRange(_)
            | Self::InvalidUtf16Range(_)
            | Self::OffsetOverflow => None,
        }
    }
}

impl From<TextPositionError> for AccessibilityTextError {
    fn from(error: TextPositionError) -> Self {
        Self::Position(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::ByteOffset;
    use yu_text::{Edit, TextBuffer, Transaction, retained_snapshot_stats};

    fn source_range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test source range must be ordered")
    }

    fn utf16_range(start: u64, end: u64) -> Utf16Range {
        Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end))
            .expect("test UTF-16 range must be ordered")
    }

    #[test]
    fn selection_and_text_queries_bridge_utf8_and_utf16() {
        let buffer = TextBuffer::new("a😊\n羽\r\nlast");
        let accessibility = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(1, 5))
            .expect("emoji selection should be valid");

        assert_eq!(accessibility.number_of_characters(), Utf16Offset::new(11));
        assert_eq!(accessibility.selected_range().range(), utf16_range(1, 3));
        assert_eq!(
            accessibility
                .text_for_range(accessibility.selected_range())
                .expect("selected text query should succeed"),
            "😊"
        );
        assert_eq!(
            accessibility
                .source_range(accessibility.selected_range())
                .expect("selected source range should resolve"),
            source_range(1, 5)
        );
    }

    #[test]
    fn logical_line_queries_include_the_terminating_lf() {
        let buffer = TextBuffer::new("a😊\n羽\r\nlast");
        let accessibility = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("empty selection should be valid");

        let first = accessibility
            .range_for_line(LineIndex::ZERO)
            .expect("first line should exist");
        let second = accessibility
            .range_for_line(LineIndex::new(1))
            .expect("second line should exist");
        let last = accessibility
            .range_for_line(LineIndex::new(2))
            .expect("last line should exist");

        assert_eq!(first.range(), utf16_range(0, 4));
        assert_eq!(second.range(), utf16_range(4, 7));
        assert_eq!(last.range(), utf16_range(7, 11));
        assert_eq!(
            accessibility
                .line_for_position(
                    accessibility
                        .bind_position(Utf16Offset::new(5))
                        .expect("line position should bind")
                )
                .expect("line query should succeed"),
            LineIndex::new(1)
        );
    }

    #[test]
    fn stale_ranges_are_rejected_after_the_document_changes() {
        let mut buffer = TextBuffer::new("old");
        let old = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("old snapshot should be valid");
        let old_range = old.full_range();
        let transaction =
            Transaction::new(buffer.revision(), [Edit::new(source_range(0, 3), "new")]);
        buffer
            .apply(&transaction)
            .expect("replacement should apply");
        let new = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("new snapshot should be valid");

        assert!(matches!(
            new.text_for_range(old_range),
            Err(AccessibilityTextError::StaleRevision { .. })
        ));
    }

    #[test]
    fn native_range_cannot_split_a_surrogate_pair() {
        let buffer = TextBuffer::new("😊");
        let accessibility = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("empty selection should be valid");

        assert!(matches!(
            accessibility.bind_range(utf16_range(1, 1)),
            Err(AccessibilityTextError::Position(
                TextPositionError::Utf16InsideScalar(_)
            ))
        ));
    }

    #[test]
    fn text_query_does_not_materialize_a_piece_tree_snapshot() {
        let mut buffer = TextBuffer::new("alpha");
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(source_range(5, 5), "😊\nomega")],
        );
        buffer.apply(&transaction).expect("append should apply");
        let snapshot = buffer.snapshot();
        let accessibility = AccessibilityTextSnapshot::new(snapshot.clone(), source_range(0, 0))
            .expect("snapshot should be valid");
        let materialized_before =
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers();
        let range = accessibility
            .bind_range(utf16_range(5, 8))
            .expect("cross-piece range should bind");

        assert_eq!(
            accessibility
                .text_for_range(range)
                .expect("cross-piece query should succeed"),
            "😊\n"
        );
        assert_eq!(
            retained_snapshot_stats(&[snapshot]).materialized_buffers(),
            materialized_before
        );
    }
}
