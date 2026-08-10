#![forbid(unsafe_code)]

//! Platform-independent editor state.

mod accessibility;
mod caret;
mod command;
mod composition;
mod document;
mod selection;

pub use accessibility::{
    AccessibilityTextError, AccessibilityTextPosition, AccessibilityTextRange,
    AccessibilityTextSnapshot,
};
pub use caret::{
    CaretAffinity, CaretPositionError, CaretPositionMap, NativeCaretPosition, SourceCaretPosition,
};
pub use command::{CommandResult, EditorCommand};
pub use composition::{CompositionError, CompositionOverlay};
pub use document::{EditorDocument, EditorDocumentError};
pub use selection::{EditorSelection, SelectionError};
