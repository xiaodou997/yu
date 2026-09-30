#![forbid(unsafe_op_in_unsafe_fn)]

//! Direct3D 11 + DXGI flip-model renderer for Yu.
//!
//! All scene interpretation remains in `yu-render::backend`. This crate owns
//! only native GPU objects and translates the already-flat `DrawCommand` array
//! into D3D11 submissions. It deliberately has no Markdown/layout logic.

use std::fmt;

use yu_render::BackendError;

#[cfg(any(target_os = "windows", test))]
mod command;

#[cfg(target_os = "windows")]
mod native;

#[cfg(target_os = "windows")]
pub use native::{D3DRenderer, D3DTextureIdentity};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum D3DRenderError {
    UnsupportedPlatform,
    Backend(BackendError),
    Native(String),
    DeviceLost,
    MissingTexture { resource: u64, image_kind: u32 },
    MissingAtlasPage(u32),
    InvalidResource(&'static str),
}

impl fmt::Display for D3DRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("D3D11 renderer requires Windows"),
            Self::Backend(error) => error.fmt(formatter),
            Self::Native(error) => formatter.write_str(error),
            Self::DeviceLost => formatter.write_str("D3D11 device was removed or reset"),
            Self::MissingTexture {
                resource,
                image_kind,
            } => write!(
                formatter,
                "missing D3D image texture {resource} kind {image_kind}"
            ),
            Self::MissingAtlasPage(page) => {
                write!(formatter, "missing D3D glyph atlas page {page}")
            }
            Self::InvalidResource(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for D3DRenderError {}

impl From<BackendError> for D3DRenderError {
    fn from(error: BackendError) -> Self {
        Self::Backend(error)
    }
}

#[cfg(target_os = "windows")]
impl From<windows::core::Error> for D3DRenderError {
    fn from(error: windows::core::Error) -> Self {
        use windows::Win32::Graphics::Dxgi::{
            DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
            DXGI_ERROR_DRIVER_INTERNAL_ERROR,
        };

        let code = error.code();
        if code == DXGI_ERROR_DEVICE_HUNG
            || code == DXGI_ERROR_DEVICE_REMOVED
            || code == DXGI_ERROR_DEVICE_RESET
            || code == DXGI_ERROR_DRIVER_INTERNAL_ERROR
        {
            Self::DeviceLost
        } else {
            Self::Native(error.to_string())
        }
    }
}
