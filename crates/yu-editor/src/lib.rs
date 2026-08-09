#![forbid(unsafe_code)]

//! Platform-independent editor state.

mod accessibility;
mod composition;

pub use accessibility::{
    AccessibilityTextError, AccessibilityTextPosition, AccessibilityTextRange,
    AccessibilityTextSnapshot,
};
pub use composition::{CompositionError, CompositionOverlay};
