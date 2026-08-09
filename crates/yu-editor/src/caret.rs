use std::error::Error;
use std::fmt;

use yu_core::{ByteOffset, Revision, Utf16Offset};
use yu_text::{TextPositionError, TextSnapshot};

/// Selects one of the two visual caret locations available at a line boundary.
///
/// This is deliberately separate from `yu_core::Affinity`, which controls how
/// source anchors follow edits. Caret affinity is a layout concern: upstream is
/// the end of the preceding visual line, downstream is the start of the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretAffinity {
    Upstream,
    #[default]
    Downstream,
}

/// A caret anchored to a UTF-8 source position in one immutable revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceCaretPosition {
    revision: Revision,
    offset: ByteOffset,
    affinity: CaretAffinity,
}

impl SourceCaretPosition {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn offset(self) -> ByteOffset {
        self.offset
    }

    #[must_use]
    pub const fn affinity(self) -> CaretAffinity {
        self.affinity
    }
}

/// A caret position expressed in the UTF-16 coordinates used by native text systems.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeCaretPosition {
    revision: Revision,
    offset: Utf16Offset,
    affinity: CaretAffinity,
}

impl NativeCaretPosition {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn offset(self) -> Utf16Offset {
        self.offset
    }

    #[must_use]
    pub const fn affinity(self) -> CaretAffinity {
        self.affinity
    }
}

/// Revision-bound conversion at the source/native text-system boundary.
///
/// Phase 1 uses an identity projection, so the projected text has the same
/// content as the source. A future projection map will sit before this adapter;
/// the native UTF-16 coordinate must never be treated as a source byte offset.
#[derive(Clone, Debug)]
pub struct CaretPositionMap {
    source: TextSnapshot,
}

impl CaretPositionMap {
    #[must_use]
    pub fn new(source: TextSnapshot) -> Self {
        Self { source }
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    pub fn bind_source(
        &self,
        offset: ByteOffset,
        affinity: CaretAffinity,
    ) -> Result<SourceCaretPosition, CaretPositionError> {
        self.source.utf16_offset(offset)?;
        Ok(SourceCaretPosition {
            revision: self.revision(),
            offset,
            affinity,
        })
    }

    pub fn bind_native(
        &self,
        offset: Utf16Offset,
        affinity: CaretAffinity,
    ) -> Result<NativeCaretPosition, CaretPositionError> {
        self.source.byte_offset_for_utf16(offset)?;
        Ok(NativeCaretPosition {
            revision: self.revision(),
            offset,
            affinity,
        })
    }

    pub fn to_native(
        &self,
        position: SourceCaretPosition,
    ) -> Result<NativeCaretPosition, CaretPositionError> {
        self.validate_revision(position.revision)?;
        Ok(NativeCaretPosition {
            revision: self.revision(),
            offset: self.source.utf16_offset(position.offset)?,
            affinity: position.affinity,
        })
    }

    pub fn to_source(
        &self,
        position: NativeCaretPosition,
    ) -> Result<SourceCaretPosition, CaretPositionError> {
        self.validate_revision(position.revision)?;
        Ok(SourceCaretPosition {
            revision: self.revision(),
            offset: self.source.byte_offset_for_utf16(position.offset)?,
            affinity: position.affinity,
        })
    }

    fn validate_revision(&self, actual: Revision) -> Result<(), CaretPositionError> {
        let expected = self.revision();
        if actual != expected {
            return Err(CaretPositionError::StaleRevision { expected, actual });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaretPositionError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    Position(TextPositionError),
}

impl fmt::Display for CaretPositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "caret position revision {actual:?} does not match {expected:?}"
            ),
            Self::Position(error) => error.fmt(formatter),
        }
    }
}

impl Error for CaretPositionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Position(error) => Some(error),
            Self::StaleRevision { .. } => None,
        }
    }
}

impl From<TextPositionError> for CaretPositionError {
    fn from(error: TextPositionError) -> Self {
        Self::Position(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::TextRange;
    use yu_text::{Edit, TextBuffer, Transaction};

    fn source_range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test range must be ordered")
    }

    #[test]
    fn source_and_native_positions_round_trip_across_unicode() {
        let buffer = TextBuffer::new("a😊羽");
        let map = CaretPositionMap::new(buffer.snapshot());
        let source = map
            .bind_source(ByteOffset::new(5), CaretAffinity::Downstream)
            .expect("source boundary should bind");

        let native = map.to_native(source).expect("source should map to UTF-16");
        let round_trip = map.to_source(native).expect("UTF-16 should map to source");

        assert_eq!(native.offset(), Utf16Offset::new(3));
        assert_eq!(round_trip, source);
    }

    #[test]
    fn visual_affinity_survives_coordinate_conversion() {
        let buffer = TextBuffer::new("wrapped text");
        let map = CaretPositionMap::new(buffer.snapshot());

        for affinity in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
            let source = map
                .bind_source(ByteOffset::new(7), affinity)
                .expect("source boundary should bind");
            let native = map.to_native(source).expect("source should map to UTF-16");

            assert_eq!(native.affinity(), affinity);
            assert_eq!(
                map.to_source(native)
                    .expect("native position should map back"),
                source
            );
        }
    }

    #[test]
    fn native_position_cannot_split_a_surrogate_pair() {
        let buffer = TextBuffer::new("😊");
        let map = CaretPositionMap::new(buffer.snapshot());

        assert!(matches!(
            map.bind_native(Utf16Offset::new(1), CaretAffinity::Downstream),
            Err(CaretPositionError::Position(
                TextPositionError::Utf16InsideScalar(_)
            ))
        ));
    }

    #[test]
    fn position_from_an_old_revision_is_rejected() {
        let mut buffer = TextBuffer::new("old");
        let old_map = CaretPositionMap::new(buffer.snapshot());
        let old = old_map
            .bind_source(ByteOffset::new(3), CaretAffinity::Downstream)
            .expect("old caret should bind");
        buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(source_range(0, 3), "new")],
            ))
            .expect("replacement should apply");
        let new_map = CaretPositionMap::new(buffer.snapshot());

        assert!(matches!(
            new_map.to_native(old),
            Err(CaretPositionError::StaleRevision { .. })
        ));
    }
}
