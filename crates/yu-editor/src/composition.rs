use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::{ByteOffset, Revision, TextRange, Utf16Offset, Utf16Range};
use yu_text::{Edit, Transaction};

/// Ephemeral IME preedit state projected over canonical source.
///
/// Creating or updating this value never mutates `TextBuffer`. Only [`Self::commit`]
/// creates a permanent transaction; cancellation is implemented by dropping it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionOverlay {
    base_revision: Revision,
    replacement_range: TextRange,
    text: Arc<str>,
    selection_utf16: Utf16Range,
    selection_bytes: TextRange,
}

impl CompositionOverlay {
    pub fn new(
        base_revision: Revision,
        replacement_range: TextRange,
        text: impl Into<Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<Self, CompositionError> {
        let text = text.into();
        let selection_bytes = utf16_range_to_bytes(&text, selection_utf16)?;
        Ok(Self {
            base_revision,
            replacement_range,
            text,
            selection_utf16,
            selection_bytes,
        })
    }

    #[must_use]
    pub fn base_revision(&self) -> Revision {
        self.base_revision
    }

    #[must_use]
    pub fn replacement_range(&self) -> TextRange {
        self.replacement_range
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn selection_utf16(&self) -> Utf16Range {
        self.selection_utf16
    }

    #[must_use]
    pub fn selection_bytes(&self) -> TextRange {
        self.selection_bytes
    }

    /// Replaces preedit text without changing the canonical document or base range.
    pub fn update(
        &mut self,
        text: impl Into<Arc<str>>,
        selection_utf16: Utf16Range,
    ) -> Result<(), CompositionError> {
        let text = text.into();
        let selection_bytes = utf16_range_to_bytes(&text, selection_utf16)?;
        self.text = text;
        self.selection_utf16 = selection_utf16;
        self.selection_bytes = selection_bytes;
        Ok(())
    }

    /// Converts the composition into the one permanent document mutation it owns.
    #[must_use]
    pub fn commit(self, committed_text: impl Into<Arc<str>>) -> Transaction {
        Transaction::new(
            self.base_revision,
            [Edit::new(self.replacement_range, committed_text)],
        )
    }
}

fn utf16_range_to_bytes(text: &str, range: Utf16Range) -> Result<TextRange, CompositionError> {
    let start = utf16_offset_to_byte(text, range.start())?;
    let end = utf16_offset_to_byte(text, range.end())?;
    TextRange::new(start, end).ok_or(CompositionError::InvalidSelection(range))
}

fn utf16_offset_to_byte(
    text: &str,
    requested: Utf16Offset,
) -> Result<ByteOffset, CompositionError> {
    let requested = requested.get();
    let mut utf16_offset = 0_u64;

    for (byte_offset, character) in text.char_indices() {
        if utf16_offset == requested {
            return ByteOffset::try_from(byte_offset).map_err(|_| CompositionError::OffsetOverflow);
        }
        utf16_offset += u64::from(character.len_utf16() as u8);
        if utf16_offset > requested {
            return Err(CompositionError::SplitSurrogatePair(Utf16Offset::new(
                requested,
            )));
        }
    }

    if utf16_offset == requested {
        return ByteOffset::try_from(text.len()).map_err(|_| CompositionError::OffsetOverflow);
    }

    Err(CompositionError::SelectionOutOfBounds {
        requested: Utf16Offset::new(requested),
        text_len: Utf16Offset::new(utf16_offset),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompositionError {
    InvalidSelection(Utf16Range),
    SplitSurrogatePair(Utf16Offset),
    SelectionOutOfBounds {
        requested: Utf16Offset,
        text_len: Utf16Offset,
    },
    OffsetOverflow,
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSelection(range) => write!(formatter, "invalid IME selection {range:?}"),
            Self::SplitSurrogatePair(offset) => {
                write!(
                    formatter,
                    "UTF-16 offset {offset:?} splits a surrogate pair"
                )
            }
            Self::SelectionOutOfBounds {
                requested,
                text_len,
            } => write!(
                formatter,
                "IME selection offset {requested:?} exceeds preedit length {text_len:?}"
            ),
            Self::OffsetOverflow => formatter.write_str("composition byte offset overflow"),
        }
    }
}

impl Error for CompositionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::TextBuffer;

    fn source_range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test range should be valid")
    }

    fn utf16_range(start: u64, end: u64) -> Utf16Range {
        Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end))
            .expect("test range should be valid")
    }

    #[test]
    fn preedit_updates_do_not_mutate_source() {
        let buffer = TextBuffer::new("hello");
        let mut overlay = CompositionOverlay::new(
            buffer.revision(),
            source_range(5, 5),
            "pin",
            utf16_range(3, 3),
        )
        .expect("initial preedit should be valid");

        overlay
            .update("pinyin", utf16_range(6, 6))
            .expect("updated preedit should be valid");

        assert_eq!(buffer.snapshot().as_str(), "hello");
        assert_eq!(overlay.text(), "pinyin");
        assert_eq!(overlay.replacement_range(), source_range(5, 5));
    }

    #[test]
    fn only_commit_creates_a_document_transaction() {
        let mut buffer = TextBuffer::new("hello ");
        let overlay = CompositionOverlay::new(
            buffer.revision(),
            source_range(6, 6),
            "shi jie",
            utf16_range(7, 7),
        )
        .expect("preedit should be valid");

        let transaction = overlay.commit("世界");
        buffer
            .apply(&transaction)
            .expect("composition commit should apply");

        assert_eq!(buffer.snapshot().as_str(), "hello 世界");
        assert_eq!(buffer.revision(), Revision::new(1));
    }

    #[test]
    fn dropping_overlay_is_a_zero_mutation_cancel() {
        let buffer = TextBuffer::new("before");
        let overlay = CompositionOverlay::new(
            buffer.revision(),
            source_range(0, 6),
            "hou",
            utf16_range(3, 3),
        )
        .expect("preedit should be valid");

        drop(overlay);

        assert_eq!(buffer.snapshot().as_str(), "before");
        assert_eq!(buffer.revision(), Revision::INITIAL);
    }

    #[test]
    fn utf16_selection_maps_emoji_to_utf8_bytes() {
        let overlay = CompositionOverlay::new(
            Revision::INITIAL,
            source_range(0, 0),
            "a😊b",
            utf16_range(1, 3),
        )
        .expect("emoji selection should be valid");

        assert_eq!(overlay.selection_bytes(), source_range(1, 5));
    }

    #[test]
    fn utf16_selection_cannot_split_surrogate_pair() {
        let error = CompositionOverlay::new(
            Revision::INITIAL,
            source_range(0, 0),
            "😊",
            utf16_range(1, 1),
        )
        .expect_err("selection inside surrogate pair must fail");

        assert!(matches!(error, CompositionError::SplitSurrogatePair(_)));
    }

    #[test]
    fn commit_is_rejected_if_document_changed_during_composition() {
        let mut buffer = TextBuffer::new("hello");
        let overlay = CompositionOverlay::new(
            buffer.revision(),
            source_range(5, 5),
            "yu",
            utf16_range(2, 2),
        )
        .expect("preedit should be valid");
        let unrelated = Transaction::new(buffer.revision(), [Edit::new(source_range(0, 0), "!")]);
        buffer
            .apply(&unrelated)
            .expect("unrelated edit should apply");

        let error = buffer
            .apply(&overlay.commit("羽"))
            .expect_err("stale composition must not overwrite newer source");

        assert!(matches!(error, yu_text::EditError::StaleRevision { .. }));
        assert_eq!(buffer.snapshot().as_str(), "!hello");
    }
}
