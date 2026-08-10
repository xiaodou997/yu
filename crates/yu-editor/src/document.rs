use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::{Revision, TextRange, Utf16Range};
use yu_text::{
    AppliedTransaction, EditError, TextBuffer, TextPositionError, TextSnapshot, Transaction,
};

use crate::{CompositionError, CompositionOverlay};

/// The canonical source and transient composition state owned by one editor.
///
/// `TextBuffer` remains the only persistent source of truth. The optional
/// `CompositionOverlay` is deliberately kept beside it so platform adapters
/// cannot accidentally commit preedit text through a separate shadow buffer.
#[derive(Debug)]
pub struct EditorDocument {
    buffer: TextBuffer,
    composition: Option<CompositionOverlay>,
}

impl EditorDocument {
    /// Creates a document at the initial revision.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            buffer: TextBuffer::new(source),
            composition: None,
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

    /// Returns the active composition without exposing mutable editor state.
    #[must_use]
    pub fn composition(&self) -> Option<&CompositionOverlay> {
        self.composition.as_ref()
    }

    /// Applies a permanent transaction to the canonical source.
    ///
    /// An active composition is not implicitly rewritten or committed. If the
    /// transaction advances the revision, a later composition commit will
    /// return a stale-revision error and the platform can cancel/restart it.
    pub fn apply_transaction(
        &mut self,
        transaction: &Transaction,
    ) -> Result<AppliedTransaction, EditError> {
        self.buffer.apply(transaction)
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
        let transaction = composition.clone().commit(committed_text);
        let applied = self.buffer.apply(&transaction)?;
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
        Ok(())
    }
}

/// Errors raised while coordinating canonical edits and composition state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorDocumentError {
    Composition(CompositionError),
    Edit(EditError),
    Position(TextPositionError),
    CompositionNotActive,
    CompositionActive,
}

impl fmt::Display for EditorDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Composition(error) => error.fmt(formatter),
            Self::Edit(error) => error.fmt(formatter),
            Self::Position(error) => error.fmt(formatter),
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
            Self::Position(error) => Some(error),
            Self::CompositionNotActive | Self::CompositionActive => None,
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

impl From<TextPositionError> for EditorDocumentError {
    fn from(error: TextPositionError) -> Self {
        Self::Position(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, Utf16Offset};
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
        assert!(document.composition().is_none());
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
