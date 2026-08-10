#![forbid(unsafe_code)]

//! Platform-independent editor state.

mod accessibility;
mod caret;
mod command;
mod composition;
mod document;
mod projection;
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
pub use projection::{ProjectionCache, ProjectionCacheStats};
pub use selection::{EditorSelection, SelectionError};
pub use yu_markdown::{Block, BlockKind, MarkdownDocument};
pub use yu_projection::{
    Projection, ProjectionBias, ProjectionError, VisualOffset, VisualRange, VisualRun,
    VisualRunKind,
};
