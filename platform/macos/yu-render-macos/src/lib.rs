#![allow(clippy::missing_const_for_fn)]
// cfg-gated platform branches use explicit returns so the macOS and stub
// implementations remain structurally parallel.
#![allow(clippy::needless_return)]

//! macOS Metal boundary for Yu's backend-neutral render plan.
//!
//! The Objective-C bridge in `native/metal_bridge.m` owns only the calls that
//! require Apple framework types. Rust owns device/surface/texture lifetime,
//! validates all dimensions, and exposes no native pointer to shared editor
//! state. It can submit a clear-only frame or a small retained render plan to
//! an attached layer, but does not create a window or own editor state.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use yu_assets::{
    DecodedImage, ImageDecodeError, ImageLocation, ImageLocationError, ImagePublication,
    ImageRequest,
};
use yu_core::Revision;
use yu_render::{
    AtlasPageUpload, BackendError, DamageRect, DrawCommand, EmbeddedSvgUpload, FrameConsumer,
    IMAGE_KIND_REGULAR, RenderPlan, RenderUploader, SurfaceConfig, build_damage_rects,
    build_draw_commands, cull_draw_commands, embedded_image_kind, requires_full_clear,
};
use yu_scene::Rgba8;
use yu_workspace::{
    ViewportFrameCache, ViewportFrameError, ViewportFramePublication, ViewportRenderFrame,
};

#[cfg(target_os = "macos")]
mod core_text;

#[cfg(target_os = "macos")]
pub use core_text::{CoreTextViewportFrameBuilder, CoreTextViewportFrameError};

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_void;

    use super::{DamageRect, DrawCommand, NativeImageTextureBinding, NativeTextureBinding};

    unsafe extern "C" {
        pub fn yu_metal_create_device(
            out_device: *mut *mut c_void,
            out_registry_id: *mut u64,
        ) -> i32;
        pub fn yu_metal_create_layer(
            device: *mut c_void,
            pixel_width: f64,
            pixel_height: f64,
            scale: f64,
            out_layer: *mut *mut c_void,
        ) -> i32;
        pub fn yu_metal_attach_layer_to_view(
            layer: *mut c_void,
            view: *mut c_void,
            out_attachment: *mut *mut c_void,
        ) -> i32;
        pub fn yu_metal_detach_layer_from_view(attachment: *mut c_void);
        // Probe-only AppKit host entry points are referenced by the ignored
        // lifecycle test, not by the production backend path.
        #[allow(dead_code)]
        pub fn yu_metal_create_appkit_probe_host(
            width: f64,
            height: f64,
            out_host: *mut *mut c_void,
            out_view: *mut *mut c_void,
        ) -> i32;
        #[allow(dead_code)]
        pub fn yu_metal_destroy_appkit_probe_host(host: *mut c_void);
        #[allow(dead_code)]
        pub fn yu_metal_run_appkit_on_main(
            callback: Option<extern "C" fn(*mut c_void)>,
            context: *mut c_void,
        );
        pub fn yu_metal_resize_layer(
            layer: *mut c_void,
            pixel_width: f64,
            pixel_height: f64,
            scale: f64,
        ) -> i32;
        pub fn yu_metal_upload_alpha_texture(
            device: *mut c_void,
            width: u32,
            height: u32,
            pixels: *const u8,
            pixel_length: usize,
            out_texture: *mut *mut c_void,
        ) -> i32;
        pub fn yu_metal_upload_rgba_texture(
            device: *mut c_void,
            width: u32,
            height: u32,
            pixels: *const u8,
            pixel_length: usize,
            out_texture: *mut *mut c_void,
        ) -> i32;
        pub fn yu_macos_image_decode_file(
            path_bytes: *const u8,
            path_length: usize,
            out_width: *mut u32,
            out_height: *mut u32,
            out_pixels: *mut *mut c_void,
            out_pixel_length: *mut usize,
        ) -> i32;
        pub fn yu_macos_image_free_bytes(pixels: *mut c_void);
        pub fn yu_macos_svg_rasterize(
            markup_bytes: *const u8,
            markup_length: usize,
            width: u32,
            height: u32,
            out_pixels: *mut *mut c_void,
            out_pixel_length: *mut usize,
        ) -> i32;
        pub fn yu_metal_create_render_target(
            device: *mut c_void,
            width: u32,
            height: u32,
            out_target: *mut *mut c_void,
        ) -> i32;
        pub fn yu_metal_release_render_target(target: *mut c_void);
        pub fn yu_metal_create_command_queue(
            device: *mut c_void,
            out_queue: *mut *mut c_void,
        ) -> i32;
        pub fn yu_metal_clear_and_present(
            queue: *mut c_void,
            layer: *mut c_void,
            red: f32,
            green: f32,
            blue: f32,
            alpha: f32,
        ) -> i32;
        pub fn yu_metal_create_pipeline(
            device: *mut c_void,
            source: *const std::ffi::c_char,
            source_length: usize,
            out_pipeline: *mut *mut c_void,
        ) -> i32;
        pub fn yu_metal_render_plan(
            queue: *mut c_void,
            layer: *mut c_void,
            pipeline: *mut c_void,
            target: *mut c_void,
            viewport_width: f32,
            viewport_height: f32,
            scale: f32,
            full_clear: i32,
            commands: *const DrawCommand,
            command_count: usize,
            damage: *const DamageRect,
            damage_count: usize,
            textures: *const NativeTextureBinding,
            texture_count: usize,
            image_textures: *const NativeImageTextureBinding,
            image_texture_count: usize,
        ) -> i32;
        pub fn yu_metal_release_pipeline(pipeline: *mut c_void);
        pub fn yu_metal_release(object: *mut c_void);
    }
}

/// Errors raised by the macOS ImageIO decoder boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacosImageDecodeError {
    UnsupportedPlatform,
    InvalidPath,
    Location(ImageLocationError),
    NativeDecodeFailed,
    Decode(ImageDecodeError),
    WorkerClosed,
}

impl fmt::Display for MacosImageDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("macOS ImageIO decoding is unavailable on this platform")
            }
            Self::InvalidPath => formatter.write_str("image path is not valid UTF-8"),
            Self::Location(error) => error.fmt(formatter),
            Self::NativeDecodeFailed => formatter.write_str("ImageIO could not decode the image"),
            Self::Decode(error) => error.fmt(formatter),
            Self::WorkerClosed => formatter.write_str("macOS image decode worker is closed"),
        }
    }
}

impl Error for MacosImageDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Location(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::UnsupportedPlatform
            | Self::InvalidPath
            | Self::NativeDecodeFailed
            | Self::WorkerClosed => None,
        }
    }
}

/// Upper bounds for the native SVG rasterization handoff. Embedded markup is
/// untrusted document content, so a renderer cannot request an unbounded
/// bitmap or make the AppKit decoder retain arbitrarily large source data.
pub const MACOS_EMBEDDED_SVG_MAX_DIMENSION: u32 = 4096;
pub const MACOS_EMBEDDED_SVG_MAX_MARKUP_BYTES: usize = 4 * 1024 * 1024;
pub const MACOS_EMBEDDED_SVG_MAX_PIXEL_BYTES: usize = 64 * 1024 * 1024;

/// Errors raised while converting a bounded embedded SVG to RGBA8 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacosEmbeddedSvgError {
    UnsupportedPlatform,
    InvalidMarkup,
    MarkupTooLarge {
        actual: usize,
        maximum: usize,
    },
    InvalidDimensions,
    DimensionsTooLarge {
        width: u32,
        height: u32,
        maximum: u32,
    },
    PixelBufferTooLarge {
        actual: usize,
        maximum: usize,
    },
    NativeRasterizeFailed,
    InvalidPixelBuffer {
        expected: usize,
        actual: usize,
    },
    Decode(ImageDecodeError),
}

impl fmt::Display for MacosEmbeddedSvgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("macOS SVG rasterization is unavailable on this platform")
            }
            Self::InvalidMarkup => formatter.write_str("embedded SVG markup is empty"),
            Self::MarkupTooLarge { actual, maximum } => {
                write!(
                    formatter,
                    "embedded SVG markup has {actual} bytes, maximum is {maximum}"
                )
            }
            Self::InvalidDimensions => {
                formatter.write_str("embedded SVG dimensions must be positive")
            }
            Self::DimensionsTooLarge {
                width,
                height,
                maximum,
            } => write!(
                formatter,
                "embedded SVG dimensions {width}x{height} exceed maximum {maximum}"
            ),
            Self::PixelBufferTooLarge { actual, maximum } => write!(
                formatter,
                "embedded SVG pixel buffer has {actual} bytes, maximum is {maximum}"
            ),
            Self::NativeRasterizeFailed => {
                formatter.write_str("AppKit could not rasterize the embedded SVG")
            }
            Self::InvalidPixelBuffer { expected, actual } => write!(
                formatter,
                "embedded SVG rasterizer returned {actual} bytes, expected {expected}"
            ),
            Self::Decode(error) => error.fmt(formatter),
        }
    }
}

impl Error for MacosEmbeddedSvgError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::UnsupportedPlatform
            | Self::InvalidMarkup
            | Self::MarkupTooLarge { .. }
            | Self::InvalidDimensions
            | Self::DimensionsTooLarge { .. }
            | Self::PixelBufferTooLarge { .. }
            | Self::NativeRasterizeFailed
            | Self::InvalidPixelBuffer { .. } => None,
        }
    }
}

impl From<ImageDecodeError> for MacosEmbeddedSvgError {
    fn from(error: ImageDecodeError) -> Self {
        Self::Decode(error)
    }
}

/// AppKit-backed SVG rasterizer used only at the macOS resource-consumer
/// boundary. The type is stateless; the retained cache lives in
/// [`MetalImageAtlas`], so rasterization can be retried without coupling
/// native object lifetime to the editor or scene.
#[derive(Clone, Copy, Debug, Default)]
pub struct MacosEmbeddedSvgRasterizer;

impl MacosEmbeddedSvgRasterizer {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn rasterize_upload(
        &self,
        upload: &EmbeddedSvgUpload,
    ) -> Result<DecodedImage, MacosEmbeddedSvgError> {
        self.rasterize(upload.markup(), upload.width(), upload.height())
    }

    pub fn rasterize(
        &self,
        markup: &str,
        width: u32,
        height: u32,
    ) -> Result<DecodedImage, MacosEmbeddedSvgError> {
        if markup.trim().is_empty() {
            return Err(MacosEmbeddedSvgError::InvalidMarkup);
        }
        if markup.len() > MACOS_EMBEDDED_SVG_MAX_MARKUP_BYTES {
            return Err(MacosEmbeddedSvgError::MarkupTooLarge {
                actual: markup.len(),
                maximum: MACOS_EMBEDDED_SVG_MAX_MARKUP_BYTES,
            });
        }
        if width == 0 || height == 0 {
            return Err(MacosEmbeddedSvgError::InvalidDimensions);
        }
        if width > MACOS_EMBEDDED_SVG_MAX_DIMENSION || height > MACOS_EMBEDDED_SVG_MAX_DIMENSION {
            return Err(MacosEmbeddedSvgError::DimensionsTooLarge {
                width,
                height,
                maximum: MACOS_EMBEDDED_SVG_MAX_DIMENSION,
            });
        }
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(MacosEmbeddedSvgError::PixelBufferTooLarge {
                actual: usize::MAX,
                maximum: MACOS_EMBEDDED_SVG_MAX_PIXEL_BYTES,
            })?;
        if expected > MACOS_EMBEDDED_SVG_MAX_PIXEL_BYTES {
            return Err(MacosEmbeddedSvgError::PixelBufferTooLarge {
                actual: expected,
                maximum: MACOS_EMBEDDED_SVG_MAX_PIXEL_BYTES,
            });
        }

        #[cfg(target_os = "macos")]
        {
            let mut raw_pixels = std::ptr::null_mut();
            let mut pixel_length = 0_usize;
            let rasterized = unsafe {
                native::yu_macos_svg_rasterize(
                    markup.as_bytes().as_ptr(),
                    markup.len(),
                    width,
                    height,
                    &mut raw_pixels,
                    &mut pixel_length,
                )
            };
            let raw_pixels =
                NonNull::new(raw_pixels).ok_or(MacosEmbeddedSvgError::NativeRasterizeFailed)?;
            if rasterized == 0 {
                unsafe { native::yu_macos_image_free_bytes(raw_pixels.as_ptr()) };
                return Err(MacosEmbeddedSvgError::NativeRasterizeFailed);
            }
            if pixel_length > MACOS_EMBEDDED_SVG_MAX_PIXEL_BYTES {
                unsafe { native::yu_macos_image_free_bytes(raw_pixels.as_ptr()) };
                return Err(MacosEmbeddedSvgError::PixelBufferTooLarge {
                    actual: pixel_length,
                    maximum: MACOS_EMBEDDED_SVG_MAX_PIXEL_BYTES,
                });
            }
            let pixels = unsafe {
                let slice =
                    std::slice::from_raw_parts(raw_pixels.as_ptr().cast::<u8>(), pixel_length);
                let copied = std::sync::Arc::<[u8]>::from(slice);
                native::yu_macos_image_free_bytes(raw_pixels.as_ptr());
                copied
            };
            if pixels.len() != expected {
                return Err(MacosEmbeddedSvgError::InvalidPixelBuffer {
                    expected,
                    actual: pixels.len(),
                });
            }
            return DecodedImage::new(width, height, pixels).map_err(Into::into);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (markup, width, height, expected);
            Err(MacosEmbeddedSvgError::UnsupportedPlatform)
        }
    }
}

impl From<ImageDecodeError> for MacosImageDecodeError {
    fn from(error: ImageDecodeError) -> Self {
        Self::Decode(error)
    }
}

impl From<ImageLocationError> for MacosImageDecodeError {
    fn from(error: ImageLocationError) -> Self {
        Self::Location(error)
    }
}

/// One result returned by [`MacosImageDecodeWorker`]. The request is returned
/// unchanged so the owner can publish it through `yu-assets::ImageCache` and
/// retain the source Revision/key association.
#[derive(Debug)]
pub struct MacosImageDecodeResult {
    request: ImageRequest,
    result: Result<DecodedImage, MacosImageDecodeError>,
}

impl MacosImageDecodeResult {
    #[must_use]
    pub fn request(&self) -> &ImageRequest {
        &self.request
    }

    #[must_use = "inspect or publish the decode result"]
    pub fn result(&self) -> &Result<DecodedImage, MacosImageDecodeError> {
        &self.result
    }

    pub fn into_parts(self) -> (ImageRequest, Result<DecodedImage, MacosImageDecodeError>) {
        (self.request, self.result)
    }
}

/// Small ImageIO-backed decoder used by a platform host or worker thread.
///
/// The decoder owns no cache and performs no editor mutation. It resolves a
/// source-backed request against the Markdown document path and returns owned
/// RGBA8 pixels suitable for `yu-assets::ImageCache::publish_decoded`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MacosImageDecoder;

impl MacosImageDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn decode_request(
        &self,
        request: &ImageRequest,
        document_path: impl AsRef<Path>,
    ) -> Result<DecodedImage, MacosImageDecodeError> {
        let location = ImageLocation::resolve(document_path, request.key().destination())?;
        self.decode_file(location.path())
    }

    pub fn decode_file(&self, path: &Path) -> Result<DecodedImage, MacosImageDecodeError> {
        #[cfg(target_os = "macos")]
        {
            let path = path.to_str().ok_or(MacosImageDecodeError::InvalidPath)?;
            let mut width = 0_u32;
            let mut height = 0_u32;
            let mut raw_pixels = std::ptr::null_mut();
            let mut pixel_length = 0_usize;
            let decoded = unsafe {
                native::yu_macos_image_decode_file(
                    path.as_bytes().as_ptr(),
                    path.len(),
                    &mut width,
                    &mut height,
                    &mut raw_pixels,
                    &mut pixel_length,
                )
            };
            let raw_pixels =
                NonNull::new(raw_pixels).ok_or(MacosImageDecodeError::NativeDecodeFailed)?;
            let pixels = unsafe {
                let slice =
                    std::slice::from_raw_parts(raw_pixels.as_ptr().cast::<u8>(), pixel_length);
                let copied = std::sync::Arc::<[u8]>::from(slice);
                native::yu_macos_image_free_bytes(raw_pixels.as_ptr());
                copied
            };
            if decoded == 0 {
                return Err(MacosImageDecodeError::NativeDecodeFailed);
            }
            return DecodedImage::new(width, height, pixels).map_err(Into::into);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            Err(MacosImageDecodeError::UnsupportedPlatform)
        }
    }
}

struct DecodeJob {
    request: ImageRequest,
    document_path: PathBuf,
}

/// Background ImageIO worker. The worker communicates only through owned
/// requests/results; the owner thread remains responsible for cache
/// publication and Revision validation.
pub struct MacosImageDecodeWorker {
    sender: Option<Sender<DecodeJob>>,
    receiver: Receiver<MacosImageDecodeResult>,
    join: Option<JoinHandle<()>>,
}

impl fmt::Debug for MacosImageDecodeWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MacosImageDecodeWorker")
    }
}

impl MacosImageDecodeWorker {
    pub fn new() -> Result<Self, MacosImageDecodeError> {
        let (sender, jobs) = mpsc::channel::<DecodeJob>();
        let (results, receiver) = mpsc::channel::<MacosImageDecodeResult>();
        let join = thread::Builder::new()
            .name("yu-imageio".to_owned())
            .spawn(move || {
                let decoder = MacosImageDecoder::new();
                while let Ok(job) = jobs.recv() {
                    let result = decoder.decode_request(&job.request, &job.document_path);
                    if results
                        .send(MacosImageDecodeResult {
                            request: job.request,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|_| MacosImageDecodeError::WorkerClosed)?;
        Ok(Self {
            sender: Some(sender),
            receiver,
            join: Some(join),
        })
    }

    pub fn submit(
        &self,
        request: ImageRequest,
        document_path: impl Into<PathBuf>,
    ) -> Result<(), MacosImageDecodeError> {
        self.sender
            .as_ref()
            .ok_or(MacosImageDecodeError::WorkerClosed)?
            .send(DecodeJob {
                request,
                document_path: document_path.into(),
            })
            .map_err(|_| MacosImageDecodeError::WorkerClosed)
    }

    pub fn try_recv(&self) -> Result<Option<MacosImageDecodeResult>, MacosImageDecodeError> {
        match self.receiver.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(MacosImageDecodeError::WorkerClosed),
        }
    }
}

impl Drop for MacosImageDecodeWorker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

const METAL_SHADER_SOURCE: &str = include_str!("../native/yu_shaders.metal");

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NativeTextureBinding {
    page: u32,
    texture: *mut std::ffi::c_void,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NativeImageTextureBinding {
    resource: u64,
    image_kind: u32,
    texture: *mut std::ffi::c_void,
}

/// Errors raised by the macOS Metal boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetalRenderError {
    UnsupportedPlatform,
    DeviceUnavailable,
    CommandQueueUnavailable,
    DrawableUnavailable,
    CommandBufferUnavailable,
    RenderEncoderUnavailable,
    PipelineUnavailable,
    RenderTargetUnavailable,
    DrawableSizeMismatch,
    BlitEncoderUnavailable,
    MissingAtlasPage(u32),
    InvalidRenderCommand(&'static str),
    ViewAttachmentUnavailable,
    InvalidDamageRect(&'static str),
    DeviceMismatch,
    InvalidSurfaceConfig(&'static str),
    InvalidPixelBuffer {
        expected: usize,
        actual: usize,
    },
    NativeFailure(&'static str),
    GenerationOverflow,
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for MetalRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("Metal surface is only available on macOS")
            }
            Self::DeviceUnavailable => formatter.write_str("Metal did not provide a system device"),
            Self::CommandQueueUnavailable => {
                formatter.write_str("Metal command queue creation failed")
            }
            Self::DrawableUnavailable => {
                formatter.write_str("CAMetalLayer did not provide a drawable")
            }
            Self::CommandBufferUnavailable => {
                formatter.write_str("Metal command buffer creation failed")
            }
            Self::RenderEncoderUnavailable => {
                formatter.write_str("Metal render encoder creation failed")
            }
            Self::PipelineUnavailable => {
                formatter.write_str("Metal render pipeline creation failed")
            }
            Self::RenderTargetUnavailable => {
                formatter.write_str("Metal retained render target creation failed")
            }
            Self::DrawableSizeMismatch => {
                formatter.write_str("Metal drawable size does not match the retained render target")
            }
            Self::BlitEncoderUnavailable => {
                formatter.write_str("Metal blit encoder creation failed")
            }
            Self::MissingAtlasPage(page) => {
                write!(
                    formatter,
                    "render plan references missing Metal atlas page {page}"
                )
            }
            Self::InvalidRenderCommand(message) => formatter.write_str(message),
            Self::ViewAttachmentUnavailable => {
                formatter.write_str("CAMetalLayer could not be attached to the NSView")
            }
            Self::InvalidDamageRect(message) => formatter.write_str(message),
            Self::DeviceMismatch => {
                formatter.write_str("surface and command queue use different Metal devices")
            }
            Self::InvalidSurfaceConfig(message) => formatter.write_str(message),
            Self::InvalidPixelBuffer { expected, actual } => {
                write!(
                    formatter,
                    "alpha page has {actual} bytes, expected {expected}"
                )
            }
            Self::NativeFailure(message) => formatter.write_str(message),
            Self::GenerationOverflow => formatter.write_str("Metal surface generation overflowed"),
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "Metal frame revision {actual:?} is stale for current {expected:?}"
            ),
        }
    }
}

impl Error for MetalRenderError {}

/// 后端无关那一半的错误是这个类型的子集。
///
/// 中立逻辑住在 `yu-render` 之后，它只认识 [`BackendError`]；平台这一侧还有
/// device / drawable / encoder 之类的失败，所以两个类型不合并，用一次映射
/// 把子集抬进超集。Display 文本按变体一一对应，没有第二份措辞。
impl From<BackendError> for MetalRenderError {
    fn from(error: BackendError) -> Self {
        match error {
            BackendError::MissingAtlasPage(page) => Self::MissingAtlasPage(page),
            BackendError::InvalidRenderCommand(message) => Self::InvalidRenderCommand(message),
            BackendError::InvalidDamageRect(message) => Self::InvalidDamageRect(message),
            BackendError::InvalidSurfaceConfig(message) => Self::InvalidSurfaceConfig(message),
            BackendError::StaleRevision { expected, actual } => {
                Self::StaleRevision { expected, actual }
            }
        }
    }
}

#[cfg(target_os = "macos")]
struct DeviceInner {
    raw: NonNull<std::ffi::c_void>,
    registry_id: u64,
}

#[cfg(target_os = "macos")]
impl Drop for DeviceInner {
    fn drop(&mut self) {
        // The bridge returns MTLCreateSystemDefaultDevice's retained object.
        unsafe { native::yu_metal_release(self.raw.as_ptr()) };
    }
}

/// A retained system Metal device. The native pointer never crosses the
/// shared scene or editor crates.
#[derive(Clone)]
pub struct MetalDevice {
    #[cfg(target_os = "macos")]
    inner: Rc<DeviceInner>,
}

impl fmt::Debug for MetalDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetalDevice")
            .field("registry_id", &self.registry_id())
            .finish()
    }
}

impl MetalDevice {
    /// Creates the system default Metal device on macOS.
    pub fn system_default() -> Result<Self, MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let mut raw = std::ptr::null_mut();
            let mut registry_id = 0_u64;
            let created = unsafe { native::yu_metal_create_device(&mut raw, &mut registry_id) };
            let raw = NonNull::new(raw).ok_or(MetalRenderError::DeviceUnavailable)?;
            if created == 0 {
                unsafe { native::yu_metal_release(raw.as_ptr()) };
                return Err(MetalRenderError::DeviceUnavailable);
            }
            return Ok(Self {
                inner: Rc::new(DeviceInner { raw, registry_id }),
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    /// Returns Apple's stable registry id for diagnostics and device-loss
    /// correlation. It is not a GPU handle and is safe to copy.
    #[must_use]
    pub fn registry_id(&self) -> u64 {
        #[cfg(target_os = "macos")]
        {
            self.inner.registry_id
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }

    #[cfg(target_os = "macos")]
    fn raw(&self) -> *mut std::ffi::c_void {
        self.inner.raw.as_ptr()
    }
}

/// A configured CAMetalLayer that can be attached to an AppKit NSView by a
/// scoped [`MetalViewAttachment`]. The surface itself does not own a window.
pub struct MetalSurface {
    device: MetalDevice,
    #[cfg(target_os = "macos")]
    raw_layer: NonNull<std::ffi::c_void>,
    config: SurfaceConfig,
    generation: u64,
}

/// An AppKit-owned `NSView` temporarily backed by a `MetalSurface` layer.
///
/// The lifetime ties the attachment to its surface so a caller cannot drop
/// the layer owner while AppKit still points at it. Dropping the attachment
/// restores the view's previous backing layer when it is still installed.
pub struct MetalViewAttachment<'surface> {
    #[cfg(target_os = "macos")]
    raw: NonNull<std::ffi::c_void>,
    _surface: PhantomData<&'surface MetalSurface>,
}

/// An explicitly owned AppKit attachment for a backend adapter that stores a
/// surface and its attachment in the same state object. Unlike
/// [`MetalViewAttachment`], this type does not encode the surface lifetime;
/// callers must drop it before the `MetalSurface` it was created from. It is
/// intentionally kept separate from the scoped public API so ordinary callers
/// retain the compile-time lifetime guard.
#[cfg(target_os = "macos")]
pub struct MetalViewAttachmentOwned {
    raw: NonNull<std::ffi::c_void>,
}

#[cfg(target_os = "macos")]
impl fmt::Debug for MetalViewAttachmentOwned {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MetalViewAttachmentOwned")
    }
}

#[cfg(target_os = "macos")]
impl Drop for MetalViewAttachmentOwned {
    fn drop(&mut self) {
        unsafe { native::yu_metal_detach_layer_from_view(self.raw.as_ptr()) };
    }
}

impl fmt::Debug for MetalViewAttachment<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MetalViewAttachment")
    }
}

#[cfg(target_os = "macos")]
impl Drop for MetalViewAttachment<'_> {
    fn drop(&mut self) {
        unsafe { native::yu_metal_detach_layer_from_view(self.raw.as_ptr()) };
    }
}

impl fmt::Debug for MetalSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetalSurface")
            .field("config", &self.config)
            .field("generation", &self.generation)
            .finish()
    }
}

impl MetalSurface {
    pub fn new(device: MetalDevice, config: SurfaceConfig) -> Result<Self, MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let mut raw_layer = std::ptr::null_mut();
            let created = unsafe {
                native::yu_metal_create_layer(
                    device.raw(),
                    f64::from(config.pixel_width()),
                    f64::from(config.pixel_height()),
                    config.scale(),
                    &mut raw_layer,
                )
            };
            let raw_layer = NonNull::new(raw_layer).ok_or(MetalRenderError::NativeFailure(
                "CAMetalLayer allocation failed",
            ))?;
            if created == 0 {
                unsafe { native::yu_metal_release(raw_layer.as_ptr()) };
                return Err(MetalRenderError::NativeFailure(
                    "CAMetalLayer configuration failed",
                ));
            }
            return Ok(Self {
                device,
                raw_layer,
                config,
                generation: 0,
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (device, config);
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    /// Attaches this surface's layer to an existing AppKit `NSView`.
    ///
    /// # Safety
    ///
    /// `view` must be a valid, main-thread-owned `NSView` pointer for the
    /// entire call, and the call must run on AppKit's main thread. The view
    /// remains owned by AppKit; the returned attachment only restores its
    /// previous layer when dropped.
    pub unsafe fn attach_to_view(
        &self,
        view: NonNull<std::ffi::c_void>,
    ) -> Result<MetalViewAttachment<'_>, MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let mut raw_attachment = std::ptr::null_mut();
            let attached = unsafe {
                native::yu_metal_attach_layer_to_view(
                    self.raw_layer(),
                    view.as_ptr(),
                    &mut raw_attachment,
                )
            };
            let raw_attachment =
                NonNull::new(raw_attachment).ok_or(MetalRenderError::ViewAttachmentUnavailable)?;
            if attached == 0 {
                unsafe { native::yu_metal_detach_layer_from_view(raw_attachment.as_ptr()) };
                return Err(MetalRenderError::ViewAttachmentUnavailable);
            }
            return Ok(MetalViewAttachment {
                raw: raw_attachment,
                _surface: PhantomData,
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = view;
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    /// Attaches this surface for an explicitly owned backend adapter.
    ///
    /// # Safety
    ///
    /// `view` must be a valid, main-thread-owned `NSView` pointer for the
    /// lifetime of the returned attachment. The caller must drop the returned
    /// attachment before dropping this surface.
    #[cfg(target_os = "macos")]
    pub unsafe fn attach_to_view_owned(
        &self,
        view: NonNull<std::ffi::c_void>,
    ) -> Result<MetalViewAttachmentOwned, MetalRenderError> {
        let mut raw_attachment = std::ptr::null_mut();
        let attached = unsafe {
            native::yu_metal_attach_layer_to_view(
                self.raw_layer(),
                view.as_ptr(),
                &mut raw_attachment,
            )
        };
        let raw_attachment =
            NonNull::new(raw_attachment).ok_or(MetalRenderError::ViewAttachmentUnavailable)?;
        if attached == 0 {
            unsafe { native::yu_metal_detach_layer_from_view(raw_attachment.as_ptr()) };
            return Err(MetalRenderError::ViewAttachmentUnavailable);
        }
        Ok(MetalViewAttachmentOwned {
            raw: raw_attachment,
        })
    }

    pub fn resize(&mut self, config: SurfaceConfig) -> Result<(), MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let resized = unsafe {
                native::yu_metal_resize_layer(
                    self.raw_layer.as_ptr(),
                    f64::from(config.pixel_width()),
                    f64::from(config.pixel_height()),
                    config.scale(),
                )
            };
            if resized == 0 {
                return Err(MetalRenderError::NativeFailure(
                    "CAMetalLayer resize failed",
                ));
            }
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or(MetalRenderError::GenerationOverflow)?;
            self.config = config;
            return Ok(());
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = config;
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    #[must_use]
    pub const fn device(&self) -> &MetalDevice {
        &self.device
    }

    #[must_use]
    pub const fn config(&self) -> SurfaceConfig {
        self.config
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(target_os = "macos")]
    fn raw_layer(&self) -> *mut std::ffi::c_void {
        self.raw_layer.as_ptr()
    }
}

#[cfg(target_os = "macos")]
impl Drop for MetalSurface {
    fn drop(&mut self) {
        unsafe { native::yu_metal_release(self.raw_layer.as_ptr()) };
    }
}

/// A retained alpha texture returned by the Metal uploader.
pub struct MetalTexture {
    #[cfg(target_os = "macos")]
    raw: NonNull<std::ffi::c_void>,
    width: u32,
    height: u32,
}

impl fmt::Debug for MetalTexture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetalTexture")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl MetalTexture {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[cfg(target_os = "macos")]
    fn raw(&self) -> *mut std::ffi::c_void {
        self.raw.as_ptr()
    }
}

#[cfg(target_os = "macos")]
impl Drop for MetalTexture {
    fn drop(&mut self) {
        unsafe { native::yu_metal_release(self.raw.as_ptr()) };
    }
}

/// Implements `yu-render::RenderUploader` with `MTLTexture` alpha pages.
#[derive(Clone, Debug)]
pub struct MetalUploader {
    device: MetalDevice,
}

impl MetalUploader {
    #[must_use]
    pub const fn device(&self) -> &MetalDevice {
        &self.device
    }

    pub fn new(device: MetalDevice) -> Self {
        Self { device }
    }

    /// Uploads an owned RGBA8 image into a backend texture. The same retained
    /// texture wrapper is used for alpha atlas pages and image resources; the
    /// native bridge chooses the Metal pixel format from this entry point.
    pub fn upload_rgba_image(
        &mut self,
        image: &DecodedImage,
    ) -> Result<MetalTexture, MetalRenderError> {
        let expected = usize::try_from(image.width())
            .ok()
            .and_then(|width| {
                usize::try_from(image.height())
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(MetalRenderError::InvalidPixelBuffer {
                expected: usize::MAX,
                actual: image.pixels().len(),
            })?;
        if image.pixels().len() != expected {
            return Err(MetalRenderError::InvalidPixelBuffer {
                expected,
                actual: image.pixels().len(),
            });
        }

        #[cfg(target_os = "macos")]
        {
            let mut raw = std::ptr::null_mut();
            let uploaded = unsafe {
                native::yu_metal_upload_rgba_texture(
                    self.device.raw(),
                    image.width(),
                    image.height(),
                    image.pixels().as_ptr(),
                    image.pixels().len(),
                    &mut raw,
                )
            };
            let raw = NonNull::new(raw).ok_or(MetalRenderError::NativeFailure(
                "MTLTexture allocation failed",
            ))?;
            if uploaded == 0 {
                unsafe { native::yu_metal_release(raw.as_ptr()) };
                return Err(MetalRenderError::NativeFailure(
                    "MTLTexture RGBA upload failed",
                ));
            }
            return Ok(MetalTexture {
                raw,
                width: image.width(),
                height: image.height(),
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = image;
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }
}

impl RenderUploader for MetalUploader {
    type Texture = MetalTexture;
    type Error = MetalRenderError;

    fn upload_alpha_page(&mut self, page: &AtlasPageUpload) -> Result<Self::Texture, Self::Error> {
        let expected = usize::try_from(page.width())
            .ok()
            .and_then(|width| {
                usize::try_from(page.height())
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(MetalRenderError::InvalidPixelBuffer {
                expected: usize::MAX,
                actual: page.pixels().len(),
            })?;
        if page.pixels().len() != expected {
            return Err(MetalRenderError::InvalidPixelBuffer {
                expected,
                actual: page.pixels().len(),
            });
        }

        #[cfg(target_os = "macos")]
        {
            let mut raw = std::ptr::null_mut();
            let uploaded = unsafe {
                native::yu_metal_upload_alpha_texture(
                    self.device.raw(),
                    page.width(),
                    page.height(),
                    page.pixels().as_ptr(),
                    page.pixels().len(),
                    &mut raw,
                )
            };
            let raw = NonNull::new(raw).ok_or(MetalRenderError::NativeFailure(
                "MTLTexture allocation failed",
            ))?;
            if uploaded == 0 {
                unsafe { native::yu_metal_release(raw.as_ptr()) };
                return Err(MetalRenderError::NativeFailure(
                    "MTLTexture alpha upload failed",
                ));
            }
            return Ok(MetalTexture {
                raw,
                width: page.width(),
                height: page.height(),
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = page;
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }
}

/// GPU-resident atlas pages owned by the macOS backend.
///
/// The atlas is deliberately separate from [`RenderPlan`]. A device reset can
/// discard it and force the shared plan builder to emit page uploads again,
/// while scene/layout/editor state remains unchanged.
#[derive(Debug, Default)]
pub struct MetalAtlas {
    pages: BTreeMap<u32, MetalTexture>,
    fingerprints: BTreeMap<u32, AtlasPageIdentity>,
    device_registry_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AtlasPageIdentity {
    width: u32,
    height: u32,
    fingerprint: u64,
}

impl MetalAtlas {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Uploads the page payloads emitted by one render plan.
    pub fn sync_plan(
        &mut self,
        uploader: &mut MetalUploader,
        plan: &RenderPlan,
    ) -> Result<usize, MetalRenderError> {
        let device_registry_id = uploader.device().registry_id();
        if let Some(existing) = self.device_registry_id
            && existing != device_registry_id
        {
            return Err(MetalRenderError::DeviceMismatch);
        }
        let mut staged = Vec::with_capacity(plan.uploads().len());
        for page in plan.uploads() {
            let identity = AtlasPageIdentity {
                width: page.width(),
                height: page.height(),
                fingerprint: page.fingerprint(),
            };
            if self.pages.contains_key(&page.page())
                && self.fingerprints.get(&page.page()) == Some(&identity)
            {
                continue;
            }
            let texture = uploader.upload_alpha_page(page)?;
            staged.push((page.page(), identity, texture));
        }
        let uploaded = staged.len();
        for (page, identity, texture) in staged {
            self.pages.insert(page, texture);
            self.fingerprints.insert(page, identity);
        }
        self.device_registry_id = Some(device_registry_id);
        Ok(uploaded)
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_sizes(&self) -> BTreeMap<u32, (u32, u32)> {
        self.pages
            .iter()
            .map(|(page, texture)| (*page, (texture.width(), texture.height())))
            .collect()
    }

    #[cfg(target_os = "macos")]
    fn native_bindings(&self) -> Vec<NativeTextureBinding> {
        self.pages
            .iter()
            .map(|(page, texture)| NativeTextureBinding {
                page: *page,
                texture: texture.raw(),
            })
            .collect()
    }
}

/// GPU-resident decoded images keyed by the source-backed resource identity.
///
/// This cache is intentionally separate from [`MetalAtlas`]: glyph page
/// invalidation and image resource publication have different lifetimes, and
/// a device reset can clear either cache without touching the editor scene.
#[derive(Debug, Default)]
pub struct MetalImageAtlas {
    images: BTreeMap<u64, MetalTexture>,
    identities: BTreeMap<u64, ImageTextureIdentity>,
    embedded_images: BTreeMap<(u64, u32), MetalTexture>,
    embedded_identities: BTreeMap<(u64, u32), ImageTextureIdentity>,
    device_registry_id: Option<u64>,
    evictions: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImageTextureIdentity {
    width: u32,
    height: u32,
    generation: u64,
}

impl MetalImageAtlas {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stages one decoded publication. Re-publishing the same generation is
    /// a no-op; a newer generation atomically replaces the old texture only
    /// after the upload succeeds.
    pub fn sync_publication(
        &mut self,
        uploader: &mut MetalUploader,
        publication: &yu_assets::ImagePublication,
    ) -> Result<bool, MetalRenderError> {
        let device_registry_id = uploader.device().registry_id();
        if let Some(existing) = self.device_registry_id
            && existing != device_registry_id
        {
            return Err(MetalRenderError::DeviceMismatch);
        }
        let resource = publication.key().fingerprint();
        let identity = ImageTextureIdentity {
            width: publication.image().width(),
            height: publication.image().height(),
            generation: publication.generation(),
        };
        if self.identities.get(&resource) == Some(&identity) {
            return Ok(false);
        }
        let texture = uploader.upload_rgba_image(publication.image())?;
        self.images.insert(resource, texture);
        self.identities.insert(resource, identity);
        self.device_registry_id = Some(device_registry_id);
        Ok(true)
    }

    /// Rasterizes and uploads one revision-bound embedded SVG. The key space
    /// is explicitly separate from ordinary image resources so a hash
    /// collision cannot make an embedded command sample a Markdown image.
    pub fn sync_embedded_svg(
        &mut self,
        uploader: &mut MetalUploader,
        upload: &EmbeddedSvgUpload,
    ) -> Result<bool, MetalRenderError> {
        let device_registry_id = uploader.device().registry_id();
        if let Some(existing) = self.device_registry_id
            && existing != device_registry_id
        {
            return Err(MetalRenderError::DeviceMismatch);
        }
        let resource = (upload.resource(), upload.kind().into());
        let identity = ImageTextureIdentity {
            width: upload.width(),
            height: upload.height(),
            generation: upload.generation(),
        };
        if self.embedded_identities.get(&resource) == Some(&identity) {
            return Ok(false);
        }
        let image = MacosEmbeddedSvgRasterizer::new()
            .rasterize_upload(upload)
            .map_err(|_| MetalRenderError::NativeFailure("embedded SVG rasterization failed"))?;
        let texture = uploader.upload_rgba_image(&image)?;
        self.embedded_images.insert(resource, texture);
        self.embedded_identities.insert(resource, identity);
        self.device_registry_id = Some(device_registry_id);
        Ok(true)
    }

    /// Synchronizes all first-seen embedded uploads from one render plan.
    pub fn sync_embedded_plan(
        &mut self,
        uploader: &mut MetalUploader,
        plan: &RenderPlan,
    ) -> Result<usize, MetalRenderError> {
        let mut uploaded = 0_usize;
        for upload in plan.embedded_uploads() {
            if self.sync_embedded_svg(uploader, upload)? {
                uploaded = uploaded.saturating_add(1);
            }
        }
        Ok(uploaded)
    }

    #[must_use]
    pub fn resource_count(&self) -> usize {
        self.images.len()
    }

    #[must_use]
    pub fn embedded_resource_count(&self) -> usize {
        self.embedded_images.len()
    }

    #[must_use]
    pub fn total_resource_count(&self) -> usize {
        self.images.len().saturating_add(self.embedded_images.len())
    }

    /// Retains only publications that can be referenced by the current
    /// viewport RenderPlan. Resources leaving the visible publication set are
    /// dropped before native command conversion, so the GPU cache cannot grow
    /// with every image visited during a long document scroll.
    pub fn retain_publications(&mut self, publications: &[ImagePublication]) -> usize {
        let retained = publications
            .iter()
            .map(|publication| publication.key().fingerprint())
            .collect::<BTreeSet<_>>();
        let before = self.images.len();
        self.images
            .retain(|resource, _| retained.contains(resource));
        self.identities
            .retain(|resource, _| retained.contains(resource));
        let evicted = before.saturating_sub(self.images.len());
        self.evictions = self
            .evictions
            .saturating_add(u64::try_from(evicted).unwrap_or(u64::MAX));
        if self.images.is_empty() && self.embedded_images.is_empty() {
            self.device_registry_id = None;
        }
        evicted
    }

    /// Retains only embedded resources referenced by the current scene. The
    /// caller supplies `(resource fingerprint, kind tag)` pairs; generations
    /// are cache identity details and are replaced atomically for one pair.
    pub fn retain_embedded_resources(&mut self, resources: &[(u64, u8)]) -> usize {
        let retained = resources
            .iter()
            .map(|(resource, kind)| (*resource, u32::from(*kind)))
            .collect::<BTreeSet<_>>();
        let before = self.embedded_images.len();
        self.embedded_images
            .retain(|resource, _| retained.contains(resource));
        self.embedded_identities
            .retain(|resource, _| retained.contains(resource));
        let evicted = before.saturating_sub(self.embedded_images.len());
        self.evictions = self
            .evictions
            .saturating_add(u64::try_from(evicted).unwrap_or(u64::MAX));
        if self.images.is_empty() && self.embedded_images.is_empty() {
            self.device_registry_id = None;
        }
        evicted
    }

    #[must_use]
    pub const fn eviction_count(&self) -> u64 {
        self.evictions
    }

    fn resource_sizes(&self) -> BTreeMap<u64, (u32, u32)> {
        self.images
            .iter()
            .map(|(resource, texture)| (*resource, (texture.width(), texture.height())))
            .collect()
    }

    fn embedded_resource_sizes(&self) -> BTreeMap<(u64, u32), (u32, u32)> {
        self.embedded_images
            .iter()
            .map(|(resource, texture)| (*resource, (texture.width(), texture.height())))
            .collect()
    }

    #[cfg(target_os = "macos")]
    fn native_bindings(&self) -> Vec<NativeImageTextureBinding> {
        self.images
            .iter()
            .map(|(resource, texture)| NativeImageTextureBinding {
                resource: *resource,
                image_kind: IMAGE_KIND_REGULAR,
                texture: texture.raw(),
            })
            .chain(self.embedded_images.iter().map(|(resource, texture)| {
                NativeImageTextureBinding {
                    resource: resource.0,
                    image_kind: embedded_image_kind(resource.1 as u8),
                    texture: texture.raw(),
                }
            }))
            .collect()
    }
}

#[cfg(target_os = "macos")]
struct CommandQueueInner {
    raw: NonNull<std::ffi::c_void>,
}

#[cfg(target_os = "macos")]
impl Drop for CommandQueueInner {
    fn drop(&mut self) {
        unsafe { native::yu_metal_release(self.raw.as_ptr()) };
    }
}

/// A command queue bound to one `MetalDevice`.
pub struct MetalCommandQueue {
    device: MetalDevice,
    #[cfg(target_os = "macos")]
    inner: Rc<CommandQueueInner>,
}

impl fmt::Debug for MetalCommandQueue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetalCommandQueue")
            .field("device_registry_id", &self.device.registry_id())
            .finish()
    }
}

impl MetalCommandQueue {
    pub fn new(device: MetalDevice) -> Result<Self, MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let mut raw = std::ptr::null_mut();
            let created = unsafe { native::yu_metal_create_command_queue(device.raw(), &mut raw) };
            let raw = NonNull::new(raw).ok_or(MetalRenderError::CommandQueueUnavailable)?;
            if created == 0 {
                unsafe { native::yu_metal_release(raw.as_ptr()) };
                return Err(MetalRenderError::CommandQueueUnavailable);
            }
            return Ok(Self {
                device,
                inner: Rc::new(CommandQueueInner { raw }),
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = device;
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    #[must_use]
    pub const fn device(&self) -> &MetalDevice {
        &self.device
    }
}

#[cfg(target_os = "macos")]
struct PipelineInner {
    raw: NonNull<std::ffi::c_void>,
}

#[cfg(target_os = "macos")]
impl Drop for PipelineInner {
    fn drop(&mut self) {
        unsafe { native::yu_metal_release_pipeline(self.raw.as_ptr()) };
    }
}

/// Two small Metal render pipeline states: one for solid rectangles and one
/// for alpha-sampled glyph quads. Pipeline creation is isolated here so the
/// native shader compiler and Objective-C objects never enter shared crates.
pub struct MetalPipeline {
    device: MetalDevice,
    #[cfg(target_os = "macos")]
    inner: Rc<PipelineInner>,
}

impl fmt::Debug for MetalPipeline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetalPipeline")
            .field("device_registry_id", &self.device.registry_id())
            .finish()
    }
}

impl MetalPipeline {
    pub fn new(device: MetalDevice) -> Result<Self, MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let source = METAL_SHADER_SOURCE.as_bytes();
            let source = std::ffi::CString::new(source).map_err(|_| {
                MetalRenderError::InvalidRenderCommand("Metal shader source contains NUL")
            })?;
            let mut raw = std::ptr::null_mut();
            let created = unsafe {
                native::yu_metal_create_pipeline(
                    device.raw(),
                    source.as_ptr(),
                    METAL_SHADER_SOURCE.len(),
                    &mut raw,
                )
            };
            let raw = NonNull::new(raw).ok_or(MetalRenderError::PipelineUnavailable)?;
            if created == 0 {
                unsafe { native::yu_metal_release_pipeline(raw.as_ptr()) };
                return Err(MetalRenderError::PipelineUnavailable);
            }
            return Ok(Self {
                device,
                inner: Rc::new(PipelineInner { raw }),
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = device;
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    #[must_use]
    pub const fn device(&self) -> &MetalDevice {
        &self.device
    }
}

#[cfg(target_os = "macos")]
struct MetalRenderTargetInner {
    raw: NonNull<std::ffi::c_void>,
}

#[cfg(target_os = "macos")]
impl Drop for MetalRenderTargetInner {
    fn drop(&mut self) {
        unsafe { native::yu_metal_release_render_target(self.raw.as_ptr()) };
    }
}

/// Backend-owned color storage that keeps frame contents valid while
/// `CAMetalLayer` rotates drawable textures. It is never exposed to shared
/// scene or render-plan state.
pub struct MetalRenderTarget {
    device: MetalDevice,
    width: u32,
    height: u32,
    #[cfg(target_os = "macos")]
    inner: Rc<MetalRenderTargetInner>,
}

impl fmt::Debug for MetalRenderTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetalRenderTarget")
            .field("device_registry_id", &self.device.registry_id())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl MetalRenderTarget {
    fn new(device: MetalDevice, config: SurfaceConfig) -> Result<Self, MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let mut raw = std::ptr::null_mut();
            let created = unsafe {
                native::yu_metal_create_render_target(
                    device.raw(),
                    config.pixel_width(),
                    config.pixel_height(),
                    &mut raw,
                )
            };
            let raw = NonNull::new(raw).ok_or(MetalRenderError::RenderTargetUnavailable)?;
            if created == 0 {
                unsafe { native::yu_metal_release_render_target(raw.as_ptr()) };
                return Err(MetalRenderError::RenderTargetUnavailable);
            }
            return Ok(Self {
                device,
                width: config.pixel_width(),
                height: config.pixel_height(),
                inner: Rc::new(MetalRenderTargetInner { raw }),
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (device, config);
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    #[must_use]
    const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    const fn height(&self) -> u32 {
        self.height
    }
}

/// Frame submission for clear-only validation and retained rectangle/glyph
/// commands. The first frame after creation or resize clears the retained
/// target; later frames clear and redraw only the plan's damage regions before
/// blitting to a drawable. Window creation and AppKit ownership remain outside
/// this crate.
pub struct MetalFrameRenderer {
    queue: MetalCommandQueue,
    pipeline: MetalPipeline,
    target: Option<MetalRenderTarget>,
    needs_full_clear: bool,
    last_surface_generation: Option<u64>,
    /// 上一帧提交时的 render plan viewport。
    ///
    /// damage 描述的是**内容**的变化，无法表达 viewport 自身的位移：滚动时
    /// 每个 block 的内容都没变，damage 因此可能是空的，但屏幕上所有字形的
    /// 位置都变了。沿用局部重绘会把旧字形留在 retained target 上，表现为
    /// 滚动后字形互相重叠。
    last_viewport: Option<yu_scene::Rect>,
    frame_consumer: FrameConsumer,
}

/// Owned scalar result from one host-level frame submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetalFrameSubmission {
    revision: Revision,
    uploaded_pages: usize,
    uploaded_embedded: usize,
}

impl MetalFrameSubmission {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn uploaded_pages(self) -> usize {
        self.uploaded_pages
    }

    #[must_use]
    pub const fn uploaded_embedded(self) -> usize {
        self.uploaded_embedded
    }
}

/// Errors raised by the macOS viewport host session before or during frame
/// submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetalViewportHostError {
    Frame(ViewportFrameError),
    PublicationRevisionMismatch {
        publication: Revision,
        frame: Revision,
    },
    PublicationSerialRegression {
        current: u64,
        actual: u64,
    },
    RevisionRegression {
        current: Revision,
        actual: Revision,
    },
    NoCurrentFrame {
        revision: Revision,
    },
    SurfaceGenerationRegression {
        current: u64,
        actual: u64,
    },
    SurfaceGenerationMismatch {
        expected: u64,
        actual: u64,
    },
    FrameSerialOverflow,
    Render(MetalRenderError),
}

impl fmt::Display for MetalViewportHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(error) => error.fmt(formatter),
            Self::PublicationRevisionMismatch { publication, frame } => write!(
                formatter,
                "viewport publication {publication:?} contains frame {frame:?}"
            ),
            Self::PublicationSerialRegression { current, actual } => write!(
                formatter,
                "viewport publication serial moved backwards from {current} to {actual}"
            ),
            Self::RevisionRegression { current, actual } => write!(
                formatter,
                "viewport host revision moved backwards from {current:?} to {actual:?}"
            ),
            Self::NoCurrentFrame { revision } => {
                write!(formatter, "no viewport frame is available for {revision:?}")
            }
            Self::SurfaceGenerationRegression { current, actual } => write!(
                formatter,
                "Metal surface generation moved backwards from {current} to {actual}"
            ),
            Self::SurfaceGenerationMismatch { expected, actual } => write!(
                formatter,
                "Metal surface generation {actual} is not synchronized with host generation {expected}"
            ),
            Self::FrameSerialOverflow => formatter.write_str("viewport frame serial overflowed"),
            Self::Render(error) => error.fmt(formatter),
        }
    }
}

impl Error for MetalViewportHostError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Frame(error) => Some(error),
            Self::Render(error) => Some(error),
            Self::PublicationRevisionMismatch { .. }
            | Self::PublicationSerialRegression { .. }
            | Self::RevisionRegression { .. }
            | Self::NoCurrentFrame { .. }
            | Self::SurfaceGenerationRegression { .. }
            | Self::SurfaceGenerationMismatch { .. }
            | Self::FrameSerialOverflow => None,
        }
    }
}

impl From<ViewportFrameError> for MetalViewportHostError {
    fn from(error: ViewportFrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<MetalRenderError> for MetalViewportHostError {
    fn from(error: MetalRenderError) -> Self {
        Self::Render(error)
    }
}

/// Owned scalar state returned after a host session submits a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetalViewportHostSubmission {
    revision: Revision,
    surface_generation: u64,
    frame_serial: u64,
    uploaded_pages: usize,
    uploaded_embedded: usize,
}

impl MetalViewportHostSubmission {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn surface_generation(self) -> u64 {
        self.surface_generation
    }

    #[must_use]
    pub const fn frame_serial(self) -> u64 {
        self.frame_serial
    }

    #[must_use]
    pub const fn uploaded_pages(self) -> usize {
        self.uploaded_pages
    }

    #[must_use]
    pub const fn uploaded_embedded(self) -> usize {
        self.uploaded_embedded
    }
}

/// A platform-host state machine for one document viewport.
///
/// The session deliberately owns only a revision-bound frame cache and owned
/// scalar lifecycle state. It does not own an `EditorDocument`, source text,
/// layout cache, AppKit object or Metal handle. A real host can therefore
/// advance the current Revision after an edit, acknowledge a surface resize,
/// publish a complete workspace frame, and submit it through one ordered API.
#[derive(Clone, Debug)]
pub struct MetalViewportHostSession {
    current_revision: Revision,
    surface_generation: u64,
    next_frame_serial: u64,
    frame_serial: Option<u64>,
    frame_cache: ViewportFrameCache,
    last_submission: Option<MetalViewportHostSubmission>,
}

impl MetalViewportHostSession {
    #[must_use]
    pub fn new(current_revision: Revision, surface_generation: u64) -> Self {
        Self {
            current_revision,
            surface_generation,
            next_frame_serial: 0,
            frame_serial: None,
            frame_cache: ViewportFrameCache::new(),
            last_submission: None,
        }
    }

    #[must_use]
    pub fn current_revision(&self) -> Revision {
        self.current_revision
    }

    #[must_use]
    pub const fn surface_generation(&self) -> u64 {
        self.surface_generation
    }

    #[must_use]
    pub fn frame_revision(&self) -> Option<Revision> {
        self.frame_cache.current_revision()
    }

    #[must_use]
    pub fn frame_handle(&self) -> Option<std::sync::Arc<ViewportRenderFrame>> {
        self.frame_cache.current_frame_handle(self.current_revision)
    }

    #[must_use]
    pub const fn frame_serial(&self) -> Option<u64> {
        self.frame_serial
    }

    #[must_use]
    pub const fn last_submission(&self) -> Option<MetalViewportHostSubmission> {
        self.last_submission
    }

    /// Advances the host's canonical current Revision after an edit or reset.
    /// Any cached frame from another Revision is discarded before returning.
    pub fn advance_revision(&mut self, revision: Revision) -> Result<bool, MetalViewportHostError> {
        if revision < self.current_revision {
            return Err(MetalViewportHostError::RevisionRegression {
                current: self.current_revision,
                actual: revision,
            });
        }
        let changed = self.current_revision != revision;
        self.current_revision = revision;
        if changed {
            self.frame_cache.invalidate_stale(revision);
            self.frame_serial = None;
            self.last_submission = None;
        }
        Ok(changed)
    }

    /// Acknowledges a successful native surface resize. The frame remains
    /// reusable, but the next submit must target the new surface generation.
    pub fn sync_surface_generation(
        &mut self,
        generation: u64,
    ) -> Result<bool, MetalViewportHostError> {
        if generation < self.surface_generation {
            return Err(MetalViewportHostError::SurfaceGenerationRegression {
                current: self.surface_generation,
                actual: generation,
            });
        }
        let changed = self.surface_generation != generation;
        self.surface_generation = generation;
        if changed {
            self.last_submission = None;
        }
        Ok(changed)
    }

    /// Publishes a complete frame and assigns it a monotonic host-local serial.
    pub fn publish_frame(
        &mut self,
        frame: ViewportRenderFrame,
    ) -> Result<u64, MetalViewportHostError> {
        let serial = self
            .next_frame_serial
            .checked_add(1)
            .ok_or(MetalViewportHostError::FrameSerialOverflow)?;
        self.frame_cache
            .publish_if_current(self.current_revision, frame)
            .map_err(MetalViewportHostError::Frame)?;
        self.next_frame_serial = serial;
        self.frame_serial = Some(serial);
        self.last_submission = None;
        Ok(self.next_frame_serial)
    }

    /// Accepts an owned publication produced by `yu_workspace`.
    ///
    /// The publication's Revision and serial become the host-visible frame
    /// identity. The immutable frame handle is shared with the host's
    /// revision-aware cache only after both identities have passed validation,
    /// so a stale or reordered publication cannot disturb a currently
    /// submit-able frame or trigger a scene/plan deep copy.
    pub fn accept_publication(
        &mut self,
        publication: ViewportFramePublication,
    ) -> Result<u64, MetalViewportHostError> {
        if publication.revision() != self.current_revision {
            return Err(MetalViewportHostError::Frame(ViewportFrameError::Stale {
                expected: self.current_revision,
                actual: publication.revision(),
            }));
        }
        if publication.frame().revision() != publication.revision() {
            return Err(MetalViewportHostError::PublicationRevisionMismatch {
                publication: publication.revision(),
                frame: publication.frame().revision(),
            });
        }
        if let Some(current) = self.frame_serial
            && publication.serial() <= current
        {
            return Err(MetalViewportHostError::PublicationSerialRegression {
                current,
                actual: publication.serial(),
            });
        }

        let serial = publication.serial();
        let frame = publication.frame_handle();
        self.frame_cache
            .publish_shared_if_current(self.current_revision, frame)
            .map_err(MetalViewportHostError::Frame)?;
        self.next_frame_serial = self.next_frame_serial.max(serial);
        self.frame_serial = Some(serial);
        self.last_submission = None;
        Ok(serial)
    }

    /// Submits the currently published frame through the ordered Metal host
    /// path. Failed render/upload work never updates `last_submission`.
    pub fn submit(
        &mut self,
        renderer: &mut MetalFrameRenderer,
        surface: &MetalSurface,
        uploader: &mut MetalUploader,
        atlas: &mut MetalAtlas,
    ) -> Result<MetalViewportHostSubmission, MetalViewportHostError> {
        let mut images = MetalImageAtlas::new();
        self.submit_with_images(renderer, surface, uploader, atlas, &mut images)
    }

    /// Submits the current frame while reusing a host-owned image atlas.
    /// Image textures are deliberately supplied separately from the glyph
    /// atlas so a decoded publication can arrive without rebuilding the
    /// source-backed frame or blocking the editor thread.
    pub fn submit_with_images(
        &mut self,
        renderer: &mut MetalFrameRenderer,
        surface: &MetalSurface,
        uploader: &mut MetalUploader,
        atlas: &mut MetalAtlas,
        images: &mut MetalImageAtlas,
    ) -> Result<MetalViewportHostSubmission, MetalViewportHostError> {
        self.validate_surface_generation(surface.generation())?;
        let frame = self.frame_cache.get(self.current_revision).ok_or(
            MetalViewportHostError::NoCurrentFrame {
                revision: self.current_revision,
            },
        )?;
        let frame_serial = self
            .frame_serial
            .ok_or(MetalViewportHostError::NoCurrentFrame {
                revision: self.current_revision,
            })?;
        let result = renderer.submit_viewport_frame_with_images(
            surface,
            self.current_revision,
            frame,
            uploader,
            atlas,
            images,
        )?;
        let submission = MetalViewportHostSubmission {
            revision: result.revision(),
            surface_generation: self.surface_generation,
            frame_serial,
            uploaded_pages: result.uploaded_pages(),
            uploaded_embedded: result.uploaded_embedded(),
        };
        self.last_submission = Some(submission);
        Ok(submission)
    }

    pub fn validate_surface_generation(
        &self,
        actual_generation: u64,
    ) -> Result<(), MetalViewportHostError> {
        if actual_generation != self.surface_generation {
            return Err(MetalViewportHostError::SurfaceGenerationMismatch {
                expected: self.surface_generation,
                actual: actual_generation,
            });
        }
        Ok(())
    }
}

impl fmt::Debug for MetalFrameRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetalFrameRenderer")
            .field("queue", &self.queue)
            .field("pipeline", &self.pipeline)
            .field("target", &self.target)
            .field("needs_full_clear", &self.needs_full_clear)
            .field("last_surface_generation", &self.last_surface_generation)
            .field("frame_consumer", &self.frame_consumer)
            .finish()
    }
}

impl MetalFrameRenderer {
    pub fn new(device: MetalDevice) -> Result<Self, MetalRenderError> {
        let pipeline = MetalPipeline::new(device.clone())?;
        Ok(Self {
            queue: MetalCommandQueue::new(device)?,
            pipeline,
            target: None,
            needs_full_clear: true,
            last_surface_generation: None,
            last_viewport: None,
            frame_consumer: FrameConsumer::new(),
        })
    }

    fn ensure_target(&mut self, surface: &MetalSurface) -> Result<bool, MetalRenderError> {
        let config = surface.config();
        let recreate = self.target.as_ref().is_none_or(|target| {
            target.width() != config.pixel_width() || target.height() != config.pixel_height()
        });
        if recreate {
            self.target = Some(MetalRenderTarget::new(
                self.pipeline.device().clone(),
                config,
            )?);
        }
        Ok(recreate)
    }

    /// Acquires one drawable, records a clear render pass, presents it and
    /// commits the command buffer. The surface must already be attached to a
    /// live platform view for `nextDrawable` to succeed.
    pub fn present_clear(
        &mut self,
        surface: &MetalSurface,
        color: Rgba8,
    ) -> Result<(), MetalRenderError> {
        if self.queue.device().registry_id() != surface.device().registry_id() {
            return Err(MetalRenderError::DeviceMismatch);
        }

        #[cfg(target_os = "macos")]
        {
            let status = unsafe {
                native::yu_metal_clear_and_present(
                    self.queue.inner.raw.as_ptr(),
                    surface.raw_layer(),
                    f32::from(color.red()) / 255.0,
                    f32::from(color.green()) / 255.0,
                    f32::from(color.blue()) / 255.0,
                    f32::from(color.alpha()) / 255.0,
                )
            };
            return match status {
                1 => {
                    // This path clears only the layer drawable. The retained
                    // target is intentionally left dirty so the next plan
                    // submission performs a full clear into its own storage.
                    self.needs_full_clear = true;
                    self.last_surface_generation = None;
                    self.last_viewport = None;
                    Ok(())
                }
                2 => Err(MetalRenderError::DrawableUnavailable),
                3 => Err(MetalRenderError::CommandBufferUnavailable),
                4 => Err(MetalRenderError::RenderEncoderUnavailable),
                _ => Err(MetalRenderError::NativeFailure(
                    "Metal clear/present bridge failed",
                )),
            };
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (surface, color);
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    /// Upload-independent retained rendering entry point. The caller first
    /// synchronizes `plan.uploads()` into a [`MetalAtlas`], then this method
    /// converts commands and damage into short native ABI arrays. The shared
    /// `RenderPlan` itself never contains a Metal pointer.
    pub fn render_plan(
        &mut self,
        surface: &MetalSurface,
        plan: &RenderPlan,
        atlas: &MetalAtlas,
    ) -> Result<(), MetalRenderError> {
        let images = MetalImageAtlas::new();
        self.render_plan_with_images(surface, plan, atlas, &images)
    }

    /// Retained rendering entry point with decoded image resources. Missing
    /// image resources are converted to their command fallback rectangle;
    /// the render thread never waits for ImageIO.
    pub fn render_plan_with_images(
        &mut self,
        surface: &MetalSurface,
        plan: &RenderPlan,
        atlas: &MetalAtlas,
        images: &MetalImageAtlas,
    ) -> Result<(), MetalRenderError> {
        if self.queue.device().registry_id() != surface.device().registry_id()
            || self.pipeline.device().registry_id() != surface.device().registry_id()
        {
            return Err(MetalRenderError::DeviceMismatch);
        }

        let recreated_target = self.ensure_target(surface)?;
        let viewport = plan.viewport();
        let all_commands = build_draw_commands(
            plan,
            &atlas.page_sizes(),
            &images.resource_sizes(),
            &images.embedded_resource_sizes(),
        )?;
        let damage = build_damage_rects(plan)?;
        let full_clear = requires_full_clear(
            recreated_target,
            self.needs_full_clear,
            self.last_viewport,
            viewport,
            self.last_surface_generation,
            surface.generation(),
        );
        let scale = surface.config().scale() as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return Err(MetalRenderError::InvalidRenderCommand(
                "Metal surface scale is not representable as f32",
            ));
        }
        let viewport_width = viewport.width();
        let viewport_height = viewport.height();
        if !viewport_width.is_finite()
            || !viewport_height.is_finite()
            || viewport_width <= 0.0
            || viewport_height <= 0.0
        {
            return Err(MetalRenderError::InvalidRenderCommand(
                "render plan viewport must be finite and positive",
            ));
        }
        if !full_clear && damage.is_empty() {
            return Ok(());
        }
        let commands = if full_clear {
            all_commands
        } else {
            cull_draw_commands(all_commands, &damage)
        };

        #[cfg(target_os = "macos")]
        {
            let bindings = atlas.native_bindings();
            let image_bindings = images.native_bindings();
            let target = self
                .target
                .as_ref()
                .ok_or(MetalRenderError::RenderTargetUnavailable)?;
            let status = unsafe {
                native::yu_metal_render_plan(
                    self.queue.inner.as_ref().raw.as_ptr(),
                    surface.raw_layer(),
                    self.pipeline.inner.raw.as_ptr(),
                    target.inner.raw.as_ptr(),
                    viewport_width,
                    viewport_height,
                    scale,
                    i32::from(full_clear),
                    commands.as_ptr(),
                    commands.len(),
                    damage.as_ptr(),
                    damage.len(),
                    bindings.as_ptr(),
                    bindings.len(),
                    image_bindings.as_ptr(),
                    image_bindings.len(),
                )
            };
            return match status {
                1 => {
                    self.needs_full_clear = false;
                    self.last_surface_generation = Some(surface.generation());
                    self.last_viewport = Some(viewport);
                    Ok(())
                }
                2 => Err(MetalRenderError::DrawableUnavailable),
                3 => Err(MetalRenderError::CommandBufferUnavailable),
                4 => Err(MetalRenderError::RenderEncoderUnavailable),
                5 => Err(MetalRenderError::NativeFailure(
                    "Metal command references an unavailable texture or kind",
                )),
                6 => Err(MetalRenderError::DrawableSizeMismatch),
                7 => Err(MetalRenderError::BlitEncoderUnavailable),
                _ => Err(MetalRenderError::NativeFailure(
                    "Metal render plan bridge failed",
                )),
            };
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (
                surface,
                plan,
                atlas,
                images,
                viewport_width,
                viewport_height,
                scale,
                full_clear,
                commands,
                damage,
            );
            Err(MetalRenderError::UnsupportedPlatform)
        }
    }

    /// Consumes a workspace frame only when it still belongs to the native
    /// host's current source Revision. The revision gate runs before command
    /// conversion; successful Metal submission records the accepted Revision
    /// only after `render_plan` succeeds.
    pub fn render_viewport_frame(
        &mut self,
        surface: &MetalSurface,
        current_revision: Revision,
        frame: &ViewportRenderFrame,
        atlas: &MetalAtlas,
    ) -> Result<(), MetalRenderError> {
        let images = MetalImageAtlas::new();
        self.render_viewport_frame_with_images(surface, current_revision, frame, atlas, &images)
    }

    pub fn render_viewport_frame_with_images(
        &mut self,
        surface: &MetalSurface,
        current_revision: Revision,
        frame: &ViewportRenderFrame,
        atlas: &MetalAtlas,
        images: &MetalImageAtlas,
    ) -> Result<(), MetalRenderError> {
        self.frame_consumer
            .validate_revision(current_revision, frame.revision())?;
        self.render_plan_with_images(surface, frame.plan(), atlas, images)?;
        self.frame_consumer
            .commit_revision(current_revision, frame.revision())?;
        Ok(())
    }

    /// Host-level submission for one workspace frame.
    ///
    /// The order is intentional: stale frames are rejected before any atlas
    /// upload or native command conversion; atlas uploads are staged before
    /// they become visible; successful render submission is the only point at
    /// which the consumer advances its accepted Revision.
    pub fn submit_viewport_frame(
        &mut self,
        surface: &MetalSurface,
        current_revision: Revision,
        frame: &ViewportRenderFrame,
        uploader: &mut MetalUploader,
        atlas: &mut MetalAtlas,
    ) -> Result<MetalFrameSubmission, MetalRenderError> {
        let mut images = MetalImageAtlas::new();
        self.submit_viewport_frame_with_images(
            surface,
            current_revision,
            frame,
            uploader,
            atlas,
            &mut images,
        )
    }

    pub fn submit_viewport_frame_with_images(
        &mut self,
        surface: &MetalSurface,
        current_revision: Revision,
        frame: &ViewportRenderFrame,
        uploader: &mut MetalUploader,
        atlas: &mut MetalAtlas,
        images: &mut MetalImageAtlas,
    ) -> Result<MetalFrameSubmission, MetalRenderError> {
        self.frame_consumer
            .validate_revision(current_revision, frame.revision())?;
        let uploaded_pages = atlas.sync_plan(uploader, frame.plan())?;
        let uploaded_embedded = images.sync_embedded_plan(uploader, frame.plan())?;
        self.render_plan_with_images(surface, frame.plan(), atlas, images)?;
        self.frame_consumer
            .commit_revision(current_revision, frame.revision())?;
        Ok(MetalFrameSubmission {
            revision: frame.revision(),
            uploaded_pages,
            uploaded_embedded,
        })
    }

    #[must_use]
    pub const fn queue(&self) -> &MetalCommandQueue {
        &self.queue
    }

    #[must_use]
    pub fn last_consumed_revision(&self) -> Option<Revision> {
        self.frame_consumer.last_revision()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_host_session_tracks_revision_generation_and_frame_serial() {
        let revision_one = Revision::new(1);
        let revision_two = Revision::new(2);
        let mut session = MetalViewportHostSession::new(revision_one, 7);

        assert_eq!(session.current_revision(), revision_one);
        assert_eq!(session.surface_generation(), 7);
        assert_eq!(session.frame_revision(), None);
        assert_eq!(session.frame_serial(), None);
        assert_eq!(session.last_submission(), None);
        assert_eq!(session.validate_surface_generation(7), Ok(()));
        assert_eq!(
            session.validate_surface_generation(8),
            Err(MetalViewportHostError::SurfaceGenerationMismatch {
                expected: 7,
                actual: 8,
            })
        );

        assert_eq!(session.advance_revision(revision_two), Ok(true));
        assert_eq!(session.current_revision(), revision_two);
        assert_eq!(session.frame_revision(), None);
        assert_eq!(session.frame_serial(), None);
        assert_eq!(session.last_submission(), None);
        assert_eq!(session.advance_revision(revision_two), Ok(false));
        assert_eq!(session.sync_surface_generation(8), Ok(true));
        assert_eq!(session.surface_generation(), 8);
        assert_eq!(session.sync_surface_generation(8), Ok(false));
        assert_eq!(
            session.advance_revision(revision_one),
            Err(MetalViewportHostError::RevisionRegression {
                current: revision_two,
                actual: revision_one,
            })
        );
        assert_eq!(
            session.sync_surface_generation(7),
            Err(MetalViewportHostError::SurfaceGenerationRegression {
                current: 8,
                actual: 7,
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn viewport_host_session_accepts_workspace_publication_once_in_order() {
        use std::sync::Arc;

        let publication = appkit_probe_publication();
        let revision = publication.revision();
        let serial = publication.serial();
        let publication_handle = publication.frame_handle();
        let mut session = MetalViewportHostSession::new(revision, 0);

        assert_eq!(session.accept_publication(publication.clone()), Ok(serial));
        assert_eq!(session.current_revision(), revision);
        assert_eq!(session.frame_revision(), Some(revision));
        assert_eq!(session.frame_serial(), Some(serial));
        assert!(Arc::ptr_eq(
            &session.frame_handle().expect("host frame handle"),
            &publication_handle
        ));
        assert_eq!(
            session.accept_publication(publication),
            Err(MetalViewportHostError::PublicationSerialRegression {
                current: serial,
                actual: serial,
            })
        );

        let next_revision = revision.next().expect("publication revision successor");
        session
            .advance_revision(next_revision)
            .expect("advance host revision");
        assert_eq!(session.frame_revision(), None);
        assert_eq!(session.frame_serial(), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn metal_device_is_explicitly_unsupported_off_macos() {
        assert_eq!(
            MetalDevice::system_default().expect_err("unsupported platform"),
            MetalRenderError::UnsupportedPlatform
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn imageio_decoder_returns_owned_rgba_pixels_for_a_local_png() {
        let path = std::env::temp_dir().join(format!(
            "yu-imageio-{}-{}.png",
            std::process::id(),
            Revision::INITIAL.get()
        ));
        let png: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15,
            0, 1, 5, 1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        std::fs::write(&path, png).expect("write PNG fixture");
        let image = MacosImageDecoder::new()
            .decode_file(&path)
            .expect("decode PNG fixture");
        let _ = std::fs::remove_file(&path);
        assert_eq!((image.width(), image.height()), (1, 1));
        assert_eq!(image.pixels().len(), 4);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_svg_rasterizer_returns_bounded_rgba_pixels() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3">
            <rect width="4" height="3" fill="#ff0000"/>
        </svg>"##;
        let image = MacosEmbeddedSvgRasterizer::new()
            .rasterize(svg, 4, 3)
            .expect("AppKit SVG rasterization");
        assert_eq!((image.width(), image.height()), (4, 3));
        assert_eq!(image.pixels().len(), 4 * 3 * 4);
        assert!(
            image.pixels().chunks_exact(4).any(|pixel| {
                pixel[0] > 200 && pixel[1] < 32 && pixel[2] < 32 && pixel[3] > 200
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn embedded_svg_rasterizer_rejects_oversized_requests_before_appkit() {
        let error = MacosEmbeddedSvgRasterizer::new()
            .rasterize("<svg/>", MACOS_EMBEDDED_SVG_MAX_DIMENSION + 1, 1)
            .expect_err("oversized dimensions");
        assert_eq!(
            error,
            MacosEmbeddedSvgError::DimensionsTooLarge {
                width: MACOS_EMBEDDED_SVG_MAX_DIMENSION + 1,
                height: 1,
                maximum: MACOS_EMBEDDED_SVG_MAX_DIMENSION,
            }
        );
    }

    #[cfg(target_os = "macos")]
    #[ignore = "requires a macOS session with a Metal-capable device"]
    #[test]
    fn macos_device_surface_and_atlas_upload_are_live() {
        use yu_assets::{DecodedImage, ImageCache, ImageRequest};
        use yu_core::{ByteOffset, TextRange};
        use yu_editor::{EditorDocument, LayoutConfig, ViewportConfig, ViewportSpan};
        use yu_font::{FontRequest, GlyphAtlasConfig};
        use yu_render::RenderPlanBuilder;
        use yu_scene::{ImagePrimitive, Rect, Rgba8, SceneBuilder};
        use yu_workspace::ViewportRenderConfig;

        let device = MetalDevice::system_default().expect("Metal device");
        assert_ne!(device.registry_id(), 0);
        let config = SurfaceConfig::new(320.0, 180.0, 2.0).expect("surface config");
        let mut surface = MetalSurface::new(device.clone(), config).expect("Metal surface");
        assert_eq!(surface.generation(), 0);
        surface
            .resize(SurfaceConfig::new(640.0, 360.0, 2.0).expect("resize config"))
            .expect("surface resize");
        assert_eq!(surface.generation(), 1);

        let font_size = 14.0;
        let shaper = yu_font_macos::CoreTextShaper::from_system_ui(
            FontRequest::new(".SFNS-Regular", font_size).expect("font request"),
        )
        .expect("CoreText shaper");
        let metrics = shaper.viewport_metrics("A羽🙂").expect("CoreText metrics");
        let mut document = EditorDocument::new("# Yu Metal\n\nhello **viewport**");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(280.0, metrics.line_height()),
                metrics.line_height(),
                0.0,
            ))
            .expect("viewport config");
        let mut builder = CoreTextViewportFrameBuilder::with_shaper(
            shaper,
            ViewportRenderConfig::new(
                ViewportSpan::new(0.0, 160.0),
                font_size,
                Rect::new(0.0, 0.0, 320.0, 180.0).expect("scene viewport"),
                Rgba8::white(),
            ),
            GlyphAtlasConfig::new(1024, 1024, 2).expect("atlas config"),
        )
        .expect("CoreText viewport builder");
        let publication = builder
            .publish(&mut document)
            .expect("CoreText render plan");
        let plan = publication.frame().plan();
        assert!(!plan.commands().is_empty());
        assert!(!plan.uploads().is_empty());
        let mut uploader = MetalUploader::new(device.clone());
        let mut gpu_atlas = MetalAtlas::new();
        assert_eq!(
            gpu_atlas
                .sync_plan(&mut uploader, plan)
                .expect("alpha texture"),
            1
        );
        assert_eq!(gpu_atlas.page_count(), 1);

        let image_source =
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(5)).expect("image source range");
        let image_request = ImageRequest::new(Revision::INITIAL, image_source, "fixture.png")
            .expect("image request");
        let mut image_cache = ImageCache::new();
        let publication = image_cache
            .publish_decoded(
                image_request,
                Revision::INITIAL,
                DecodedImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255])
                    .expect("decoded image"),
            )
            .expect("image publication");
        let mut image_atlas = MetalImageAtlas::new();
        assert!(
            image_atlas
                .sync_publication(&mut uploader, &publication)
                .expect("RGBA texture upload")
        );
        assert!(
            !image_atlas
                .sync_publication(&mut uploader, &publication)
                .expect("duplicate RGBA texture upload")
        );
        assert_eq!(image_atlas.resource_count(), 1);

        let mut image_scene = SceneBuilder::new(
            Revision::INITIAL,
            Rect::new(0.0, 0.0, 320.0, 180.0).expect("image viewport"),
        )
        .expect("image scene");
        image_scene
            .image(ImagePrimitive::new(
                publication.key().fingerprint(),
                Rect::new(16.0, 16.0, 128.0, 64.0).expect("image bounds"),
                Rgba8::new(230, 232, 236, 255),
            ))
            .expect("image primitive");
        let image_plan = RenderPlanBuilder::new()
            .build(
                &image_scene.finish(),
                &yu_font::GlyphAtlas::new(
                    GlyphAtlasConfig::new(16, 16, 1).expect("image atlas config"),
                ),
            )
            .expect("image render plan");

        let mut frame_renderer = MetalFrameRenderer::new(device).expect("command queue/pipeline");
        let result =
            frame_renderer.render_plan_with_images(&surface, &image_plan, &gpu_atlas, &image_atlas);
        assert!(matches!(
            result,
            Ok(()) | Err(MetalRenderError::DrawableUnavailable)
        ));
        let result = frame_renderer.render_plan(&surface, plan, &gpu_atlas);
        assert!(matches!(
            result,
            Ok(()) | Err(MetalRenderError::DrawableUnavailable)
        ));
        let result = frame_renderer.present_clear(&surface, Rgba8::new(12, 24, 48, 255));
        assert!(matches!(
            result,
            Ok(()) | Err(MetalRenderError::DrawableUnavailable)
        ));
    }

    #[cfg(target_os = "macos")]
    struct AppKitProbeState {
        surface: MetalSurface,
        renderer: MetalFrameRenderer,
        publication: ViewportFramePublication,
        session: MetalViewportHostSession,
        uploader: MetalUploader,
        atlas: MetalAtlas,
        stale: Option<Result<u64, MetalViewportHostError>>,
        first: Option<Result<MetalViewportHostSubmission, MetalViewportHostError>>,
        second: Option<Result<MetalViewportHostSubmission, MetalViewportHostError>>,
        attachment_error: Option<MetalRenderError>,
        host_created: bool,
    }

    #[cfg(target_os = "macos")]
    fn appkit_probe_publication() -> ViewportFramePublication {
        use yu_editor::{EditorDocument, LayoutConfig, ViewportConfig, ViewportSpan};
        use yu_font::{FontRequest, GlyphAtlasConfig};
        use yu_scene::{Rect, Rgba8};
        use yu_workspace::ViewportRenderConfig;

        let font_size = 14.0;
        let shaper = yu_font_macos::CoreTextShaper::from_system_ui(
            FontRequest::new(".SFNS-Regular", font_size).expect("probe font request"),
        )
        .expect("CoreText probe shaper");
        let metrics = shaper
            .viewport_metrics("A羽🙂")
            .expect("CoreText probe metrics");
        let viewport = ViewportSpan::new(0.0, 160.0);
        let mut document = EditorDocument::new("# Yu Metal\n\nhello **viewport**");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(280.0, metrics.line_height()),
                metrics.line_height(),
                0.0,
            ))
            .expect("probe viewport config");
        let mut builder = CoreTextViewportFrameBuilder::with_shaper(
            shaper,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 320.0, 180.0).expect("probe scene viewport"),
                Rgba8::new(24, 28, 36, 255),
            ),
            GlyphAtlasConfig::new(1024, 1024, 2).expect("atlas config"),
        )
        .expect("CoreText viewport builder");
        builder
            .publish(&mut document)
            .expect("CoreText workspace frame")
    }

    #[cfg(target_os = "macos")]
    extern "C" fn run_appkit_probe(context: *mut std::ffi::c_void) {
        use std::ptr::NonNull;

        let state = unsafe { &mut *(context.cast::<AppKitProbeState>()) };
        let mut host = std::ptr::null_mut();
        let mut view = std::ptr::null_mut();
        let created = unsafe {
            native::yu_metal_create_appkit_probe_host(320.0, 180.0, &mut host, &mut view)
        };
        let Some(view) = NonNull::new(view) else {
            return;
        };
        if created == 0 || host.is_null() {
            return;
        }
        state.host_created = true;

        match unsafe { state.surface.attach_to_view(view) } {
            Ok(attachment) => {
                let stale_revision = state
                    .publication
                    .revision()
                    .next()
                    .expect("probe revision successor");
                let mut stale_session =
                    MetalViewportHostSession::new(stale_revision, state.surface.generation());
                state.stale = Some(stale_session.accept_publication(state.publication.clone()));
                state.first = Some(state.session.submit(
                    &mut state.renderer,
                    &state.surface,
                    &mut state.uploader,
                    &mut state.atlas,
                ));
                drop(attachment);
            }
            Err(error) => {
                state.attachment_error = Some(error);
            }
        }

        if state.attachment_error.is_none() {
            let resize = SurfaceConfig::new(300.0, 160.0, 2.0)
                .map_err(MetalRenderError::from)
                .and_then(|config| state.surface.resize(config).map(|()| config));
            if resize.is_ok() {
                let generation = state.surface.generation();
                let _ = state.session.sync_surface_generation(generation);
                match unsafe { state.surface.attach_to_view(view) } {
                    Ok(attachment) => {
                        state.second = Some(state.session.submit(
                            &mut state.renderer,
                            &state.surface,
                            &mut state.uploader,
                            &mut state.atlas,
                        ));
                        drop(attachment);
                    }
                    Err(error) => {
                        state.attachment_error = Some(error);
                    }
                }
            }
        }

        unsafe { native::yu_metal_destroy_appkit_probe_host(host) };
    }

    #[cfg(target_os = "macos")]
    #[ignore = "requires a macOS AppKit session with a Metal-capable device"]
    #[test]
    fn macos_appkit_attachment_resize_and_drawable_probe_are_live() {
        let device = MetalDevice::system_default().expect("Metal device");
        let surface = MetalSurface::new(
            device.clone(),
            SurfaceConfig::new(320.0, 180.0, 2.0).expect("surface config"),
        )
        .expect("surface");
        let publication = appkit_probe_publication();
        let revision = publication.revision();
        let mut session = MetalViewportHostSession::new(revision, 0);
        session
            .accept_publication(publication.clone())
            .expect("probe frame publish");
        let state = AppKitProbeState {
            surface,
            renderer: MetalFrameRenderer::new(device.clone()).expect("renderer"),
            publication,
            session,
            uploader: MetalUploader::new(device),
            atlas: MetalAtlas::new(),
            stale: None,
            first: None,
            second: None,
            attachment_error: None,
            host_created: false,
        };
        let mut state = state;
        unsafe {
            native::yu_metal_run_appkit_on_main(
                Some(run_appkit_probe),
                (&mut state as *mut AppKitProbeState).cast::<std::ffi::c_void>(),
            );
        }
        assert!(state.host_created, "AppKit probe host was not created");
        assert!(state.attachment_error.is_none());
        assert_eq!(
            state.stale,
            Some(Err(MetalViewportHostError::Frame(
                yu_workspace::ViewportFrameError::Stale {
                    expected: revision.next().expect("probe revision successor"),
                    actual: revision,
                },
            )))
        );
        assert!(
            matches!(state.first, Some(Ok(MetalViewportHostSubmission { revision: submitted, .. })) if submitted == revision)
                || matches!(
                    state.first,
                    Some(Err(MetalViewportHostError::Render(
                        MetalRenderError::DrawableUnavailable
                    )))
                )
        );
        assert!(
            matches!(state.second, Some(Ok(MetalViewportHostSubmission { revision: submitted, .. })) if submitted == revision)
                || matches!(
                    state.second,
                    Some(Err(MetalViewportHostError::Render(
                        MetalRenderError::DrawableUnavailable
                    )))
                )
        );
        assert_eq!(state.atlas.page_count(), 1);
        if matches!(state.first, Some(Ok(_))) || matches!(state.second, Some(Ok(_))) {
            assert_eq!(state.renderer.last_consumed_revision(), Some(revision));
        }
    }
}
