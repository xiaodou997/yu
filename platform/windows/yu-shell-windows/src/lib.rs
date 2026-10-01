#![forbid(unsafe_op_in_unsafe_fn)]

//! Native Windows product shell.
//!
//! The shell owns Windows windowing, menus, DPI/theme facts and file dialogs.
//! The canonical Markdown source, dirty state, history and close lifecycle stay
//! inside `yu_storage::DocumentEditorSession`. The editor viewport is a
//! dedicated native surface host connected to the shared scene/render plan,
//! DirectWrite glyph atlas and D3D renderer, including background resources.

pub mod locale;
pub mod model;
#[cfg(any(target_os = "windows", test))]
mod text_input;

#[cfg(target_os = "windows")]
mod chrome;
#[cfg(target_os = "windows")]
mod native;
#[cfg(target_os = "windows")]
mod resources;
#[cfg(target_os = "windows")]
mod tsf;

use std::fmt;

pub use locale::{Locale, Strings};
pub use model::{DocumentSlot, SaveAction, ShellState, SidebarMode, WindowMetrics};

#[derive(Debug)]
pub enum ShellError {
    Storage(yu_storage::StorageError),
    Platform(String),
    UnsupportedPlatform,
}

impl fmt::Display for ShellError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(formatter),
            Self::Platform(error) => formatter.write_str(error),
            Self::UnsupportedPlatform => formatter.write_str("Yu Windows shell requires Windows"),
        }
    }
}

impl std::error::Error for ShellError {}

impl From<yu_storage::StorageError> for ShellError {
    fn from(error: yu_storage::StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<yu_storage::CloseStateError> for ShellError {
    fn from(error: yu_storage::CloseStateError) -> Self {
        Self::Storage(yu_storage::StorageError::CloseState(error))
    }
}

#[cfg(target_os = "windows")]
pub fn run() -> Result<(), ShellError> {
    native::run()
}

#[cfg(not(target_os = "windows"))]
pub fn run() -> Result<(), ShellError> {
    Err(ShellError::UnsupportedPlatform)
}
