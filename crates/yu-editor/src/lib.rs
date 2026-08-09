#![forbid(unsafe_code)]

//! Platform-independent editor state.

mod composition;

pub use composition::{CompositionError, CompositionOverlay};
