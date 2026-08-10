#![forbid(unsafe_code)]

//! Platform-independent editor state.

mod accessibility;
mod caret;
mod composition;
mod document;

pub use accessibility::{
    AccessibilityTextError, AccessibilityTextPosition, AccessibilityTextRange,
    AccessibilityTextSnapshot,
};
pub use caret::{
    CaretAffinity, CaretPositionError, CaretPositionMap, NativeCaretPosition, SourceCaretPosition,
};
pub use composition::{CompositionError, CompositionOverlay};
pub use document::{EditorDocument, EditorDocumentError};
