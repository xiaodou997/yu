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

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use yu_render::{AtlasPageUpload, RenderCommand, RenderPlan, RenderUploader};
use yu_scene::Rgba8;

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_void;

    use super::{NativeDamageRect, NativeDrawCommand, NativeTextureBinding};

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
            commands: *const NativeDrawCommand,
            command_count: usize,
            damage: *const NativeDamageRect,
            damage_count: usize,
            textures: *const NativeTextureBinding,
            texture_count: usize,
        ) -> i32;
        pub fn yu_metal_release_pipeline(pipeline: *mut c_void);
        pub fn yu_metal_release(object: *mut c_void);
    }
}

const METAL_SHADER_SOURCE: &str = include_str!("../native/yu_shaders.metal");

const DRAW_FILL_RECT: u32 = 0;
const DRAW_GLYPH: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct NativeDrawCommand {
    kind: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    page: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct NativeDamageRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NativeTextureBinding {
    page: u32,
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
    InvalidPixelBuffer { expected: usize, actual: usize },
    NativeFailure(&'static str),
    GenerationOverflow,
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
        }
    }
}

impl Error for MetalRenderError {}

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

/// Logical surface dimensions and their drawable pixel size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetalSurfaceConfig {
    logical_width: f64,
    logical_height: f64,
    scale: f64,
    pixel_width: u32,
    pixel_height: u32,
}

impl MetalSurfaceConfig {
    pub fn new(width: f64, height: f64, scale: f64) -> Result<Self, MetalRenderError> {
        if !width.is_finite() || width <= 0.0 {
            return Err(MetalRenderError::InvalidSurfaceConfig(
                "surface width must be finite and positive",
            ));
        }
        if !height.is_finite() || height <= 0.0 {
            return Err(MetalRenderError::InvalidSurfaceConfig(
                "surface height must be finite and positive",
            ));
        }
        if !scale.is_finite() || scale <= 0.0 {
            return Err(MetalRenderError::InvalidSurfaceConfig(
                "surface scale must be finite and positive",
            ));
        }
        let pixel_width = pixels(width, scale)?;
        let pixel_height = pixels(height, scale)?;
        Ok(Self {
            logical_width: width,
            logical_height: height,
            scale,
            pixel_width,
            pixel_height,
        })
    }

    #[must_use]
    pub const fn logical_width(self) -> f64 {
        self.logical_width
    }

    #[must_use]
    pub const fn logical_height(self) -> f64 {
        self.logical_height
    }

    #[must_use]
    pub const fn scale(self) -> f64 {
        self.scale
    }

    #[must_use]
    pub const fn pixel_width(self) -> u32 {
        self.pixel_width
    }

    #[must_use]
    pub const fn pixel_height(self) -> u32 {
        self.pixel_height
    }
}

fn pixels(value: f64, scale: f64) -> Result<u32, MetalRenderError> {
    let value = (value * scale).ceil();
    if !value.is_finite() || value <= 0.0 || value > f64::from(u32::MAX) {
        return Err(MetalRenderError::InvalidSurfaceConfig(
            "surface pixel dimensions overflow u32",
        ));
    }
    Ok(value as u32)
}

/// A configured CAMetalLayer that can be attached to an AppKit NSView by a
/// scoped [`MetalViewAttachment`]. The surface itself does not own a window.
pub struct MetalSurface {
    device: MetalDevice,
    #[cfg(target_os = "macos")]
    raw_layer: NonNull<std::ffi::c_void>,
    config: MetalSurfaceConfig,
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
    pub fn new(device: MetalDevice, config: MetalSurfaceConfig) -> Result<Self, MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let mut raw_layer = std::ptr::null_mut();
            let created = unsafe {
                native::yu_metal_create_layer(
                    device.raw(),
                    f64::from(config.pixel_width),
                    f64::from(config.pixel_height),
                    config.scale,
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

    pub fn resize(&mut self, config: MetalSurfaceConfig) -> Result<(), MetalRenderError> {
        #[cfg(target_os = "macos")]
        {
            let resized = unsafe {
                native::yu_metal_resize_layer(
                    self.raw_layer.as_ptr(),
                    f64::from(config.pixel_width),
                    f64::from(config.pixel_height),
                    config.scale,
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
    pub const fn config(&self) -> MetalSurfaceConfig {
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
        let mut staged = Vec::with_capacity(plan.uploads().len());
        for page in plan.uploads() {
            let texture = uploader.upload_alpha_page(page)?;
            staged.push((page.page(), texture));
        }
        let uploaded = staged.len();
        for (page, texture) in staged {
            self.pages.insert(page, texture);
        }
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
    fn new(device: MetalDevice, config: MetalSurfaceConfig) -> Result<Self, MetalRenderError> {
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
        if self.queue.device().registry_id() != surface.device().registry_id()
            || self.pipeline.device().registry_id() != surface.device().registry_id()
        {
            return Err(MetalRenderError::DeviceMismatch);
        }

        let recreated_target = self.ensure_target(surface)?;
        let viewport = plan.viewport();
        let commands = build_native_commands(plan, &atlas.page_sizes())?;
        let damage = build_native_damage(plan)?;
        let full_clear = recreated_target
            || self.needs_full_clear
            || self.last_surface_generation != Some(surface.generation());
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

        #[cfg(target_os = "macos")]
        {
            let bindings = atlas.native_bindings();
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
                )
            };
            return match status {
                1 => {
                    self.needs_full_clear = false;
                    self.last_surface_generation = Some(surface.generation());
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

    #[must_use]
    pub const fn queue(&self) -> &MetalCommandQueue {
        &self.queue
    }
}

fn normalized_channel(channel: u8) -> f32 {
    f32::from(channel) / 255.0
}

fn build_native_commands(
    plan: &RenderPlan,
    page_sizes: &BTreeMap<u32, (u32, u32)>,
) -> Result<Vec<NativeDrawCommand>, MetalRenderError> {
    let viewport = plan.viewport();
    let mut commands = Vec::with_capacity(plan.commands().len());
    for command in plan.commands() {
        match *command {
            RenderCommand::FillRect { bounds, color } => {
                if !bounds.x().is_finite()
                    || !bounds.y().is_finite()
                    || !bounds.width().is_finite()
                    || !bounds.height().is_finite()
                {
                    return Err(MetalRenderError::InvalidRenderCommand(
                        "fill rectangle geometry is not finite",
                    ));
                }
                if bounds.width() == 0.0 || bounds.height() == 0.0 {
                    continue;
                }
                let x = bounds.x() - viewport.x();
                let y = bounds.y() - viewport.y();
                if !x.is_finite() || !y.is_finite() {
                    return Err(MetalRenderError::InvalidRenderCommand(
                        "fill rectangle position is not finite",
                    ));
                }
                commands.push(NativeDrawCommand {
                    kind: DRAW_FILL_RECT,
                    x,
                    y,
                    width: bounds.width(),
                    height: bounds.height(),
                    u0: 0.0,
                    v0: 0.0,
                    u1: 0.0,
                    v1: 0.0,
                    red: normalized_channel(color.red()),
                    green: normalized_channel(color.green()),
                    blue: normalized_channel(color.blue()),
                    alpha: normalized_channel(color.alpha()),
                    page: u32::MAX,
                });
            }
            RenderCommand::Glyph {
                page,
                rect,
                origin,
                metrics,
                color,
            } => {
                let Some(page) = page else {
                    // Empty glyphs keep their advance in layout but have no
                    // coverage pixels to submit to Metal.
                    continue;
                };
                let Some(&(page_width, page_height)) = page_sizes.get(&page) else {
                    return Err(MetalRenderError::MissingAtlasPage(page));
                };
                if page_width == 0 || page_height == 0 {
                    return Err(MetalRenderError::InvalidRenderCommand(
                        "atlas page dimensions must be positive",
                    ));
                }
                let rect_right = u64::from(rect.x()) + u64::from(rect.width());
                let rect_bottom = u64::from(rect.y()) + u64::from(rect.height());
                if rect_right > u64::from(page_width) || rect_bottom > u64::from(page_height) {
                    return Err(MetalRenderError::InvalidRenderCommand(
                        "glyph atlas rectangle exceeds its page",
                    ));
                }
                if rect.width() == 0 || rect.height() == 0 {
                    continue;
                }
                let x = origin.x() + metrics.bearing_x() - viewport.x();
                let y = origin.y() - metrics.bearing_y() - viewport.y();
                let width = rect.width() as f32;
                let height = rect.height() as f32;
                if !x.is_finite() || !y.is_finite() {
                    return Err(MetalRenderError::InvalidRenderCommand(
                        "glyph origin is not finite",
                    ));
                }
                commands.push(NativeDrawCommand {
                    kind: DRAW_GLYPH,
                    x,
                    y,
                    width,
                    height,
                    u0: rect.x() as f32 / page_width as f32,
                    v0: rect.y() as f32 / page_height as f32,
                    u1: rect_right as f32 / page_width as f32,
                    v1: rect_bottom as f32 / page_height as f32,
                    red: normalized_channel(color.red()),
                    green: normalized_channel(color.green()),
                    blue: normalized_channel(color.blue()),
                    alpha: normalized_channel(color.alpha()),
                    page,
                });
            }
        }
    }
    Ok(commands)
}

fn build_native_damage(plan: &RenderPlan) -> Result<Vec<NativeDamageRect>, MetalRenderError> {
    let viewport = plan.viewport();
    if !viewport.x().is_finite()
        || !viewport.y().is_finite()
        || !viewport.width().is_finite()
        || !viewport.height().is_finite()
        || viewport.width() <= 0.0
        || viewport.height() <= 0.0
    {
        return Err(MetalRenderError::InvalidDamageRect(
            "render plan viewport is not finite and positive",
        ));
    }

    let mut damage = Vec::with_capacity(plan.damage().len());
    for rect in plan.damage() {
        if !rect.x().is_finite()
            || !rect.y().is_finite()
            || !rect.width().is_finite()
            || !rect.height().is_finite()
            || rect.width() < 0.0
            || rect.height() < 0.0
        {
            return Err(MetalRenderError::InvalidDamageRect(
                "damage rectangle must be finite and non-negative",
            ));
        }
        let x = rect.x() - viewport.x();
        let y = rect.y() - viewport.y();
        let right = x + rect.width();
        let bottom = y + rect.height();
        if !x.is_finite() || !y.is_finite() || !right.is_finite() || !bottom.is_finite() {
            return Err(MetalRenderError::InvalidDamageRect(
                "damage rectangle overflowed viewport coordinates",
            ));
        }
        let left = x.max(0.0);
        let top = y.max(0.0);
        let right = right.min(viewport.width());
        let bottom = bottom.min(viewport.height());
        if right <= left || bottom <= top {
            continue;
        }
        damage.push(NativeDamageRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        });
    }
    Ok(damage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_config_rounds_logical_size_to_drawable_pixels() {
        let config = MetalSurfaceConfig::new(640.0, 479.5, 2.0).expect("config");
        assert_eq!(config.pixel_width(), 1280);
        assert_eq!(config.pixel_height(), 959);
    }

    #[test]
    fn surface_config_rejects_invalid_dimensions() {
        assert!(MetalSurfaceConfig::new(0.0, 10.0, 2.0).is_err());
        assert!(MetalSurfaceConfig::new(10.0, f64::NAN, 2.0).is_err());
        assert!(MetalSurfaceConfig::new(10.0, 10.0, 0.0).is_err());
    }

    #[test]
    fn native_command_conversion_keeps_painter_order_and_atlas_uvs() {
        use std::collections::BTreeMap;

        use yu_core::Revision;
        use yu_font::{
            FontFaceId, GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics,
            GlyphRasterKey, RasterizedGlyph,
        };
        use yu_render::RenderPlanBuilder;
        use yu_scene::{GlyphPrimitive, Point, Rect, SceneBuilder};

        let key =
            GlyphRasterKey::new(FontFaceId::from_raw(3), GlyphId::from_raw(11), 14.0).expect("key");
        let bitmap = GlyphBitmap::new(4, 6, 4, vec![255; 24]).expect("bitmap");
        let metrics = GlyphMetrics::new(1.0, 7.0, 5.0).expect("metrics");
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        let entry = atlas
            .insert(RasterizedGlyph::new(key, metrics, bitmap))
            .expect("entry");
        let viewport = Rect::new(10.0, 20.0, 80.0, 40.0).expect("viewport");
        let mut scene = SceneBuilder::new(Revision::INITIAL, viewport).expect("scene");
        scene
            .fill_rect(
                Rect::new(10.0, 20.0, 20.0, 5.0).expect("rect"),
                Rgba8::new(10, 20, 30, 255),
            )
            .expect("fill");
        scene
            .glyph(GlyphPrimitive::new(
                entry,
                Point::new(14.0, 32.0),
                Rgba8::white(),
            ))
            .expect("glyph");
        let plan = RenderPlanBuilder::new()
            .build(&scene.finish(), &atlas)
            .expect("plan");
        let mut page_sizes = BTreeMap::new();
        page_sizes.insert(0, (16, 16));

        let commands = build_native_commands(&plan, &page_sizes).expect("native commands");
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].kind, DRAW_FILL_RECT);
        assert_eq!(commands[0].x, 0.0);
        assert_eq!(commands[0].y, 0.0);
        assert_eq!(commands[1].kind, DRAW_GLYPH);
        assert_eq!(commands[1].x, 5.0);
        assert_eq!(commands[1].y, 5.0);
        assert_eq!(commands[1].width, 4.0);
        assert_eq!(commands[1].height, 6.0);
        assert_eq!(commands[1].u0, entry.rect().x() as f32 / 16.0);
        assert_eq!(commands[1].v0, entry.rect().y() as f32 / 16.0);
    }

    #[test]
    fn native_command_conversion_rejects_missing_atlas_page() {
        use std::collections::BTreeMap;

        use yu_core::Revision;
        use yu_font::{
            FontFaceId, GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics,
            GlyphRasterKey, RasterizedGlyph,
        };
        use yu_render::RenderPlanBuilder;
        use yu_scene::{GlyphPrimitive, Point, Rect, SceneBuilder};

        let key =
            GlyphRasterKey::new(FontFaceId::from_raw(3), GlyphId::from_raw(11), 14.0).expect("key");
        let bitmap = GlyphBitmap::new(2, 2, 2, vec![255; 4]).expect("bitmap");
        let metrics = GlyphMetrics::new(0.0, 2.0, 2.0).expect("metrics");
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(8, 8, 1).expect("config"));
        let entry = atlas
            .insert(RasterizedGlyph::new(key, metrics, bitmap))
            .expect("entry");
        let viewport = Rect::new(0.0, 0.0, 20.0, 20.0).expect("viewport");
        let mut scene = SceneBuilder::new(Revision::INITIAL, viewport).expect("scene");
        scene
            .glyph(GlyphPrimitive::new(
                entry,
                Point::new(2.0, 4.0),
                Rgba8::white(),
            ))
            .expect("glyph");
        let plan = RenderPlanBuilder::new()
            .build(&scene.finish(), &atlas)
            .expect("plan");

        assert_eq!(
            build_native_commands(&plan, &BTreeMap::new()).expect_err("missing page"),
            MetalRenderError::MissingAtlasPage(0)
        );
    }

    #[test]
    fn native_damage_conversion_clips_to_the_plan_viewport() {
        use yu_core::Revision;
        use yu_render::RenderPlanBuilder;
        use yu_scene::{Rect, SceneBuilder};

        let viewport = Rect::new(10.0, 20.0, 40.0, 30.0).expect("viewport");
        let mut scene = SceneBuilder::new(Revision::INITIAL, viewport).expect("scene");
        scene
            .fill_rect(
                Rect::new(5.0, 10.0, 20.0, 20.0).expect("partially visible rect"),
                Rgba8::white(),
            )
            .expect("fill");
        let plan = RenderPlanBuilder::new()
            .build(
                &scene.finish(),
                &yu_font::GlyphAtlas::new(
                    yu_font::GlyphAtlasConfig::new(8, 8, 1).expect("atlas config"),
                ),
            )
            .expect("plan");

        let damage = build_native_damage(&plan).expect("damage");
        assert_eq!(damage.len(), 1);
        assert_eq!(damage[0].x, 0.0);
        assert_eq!(damage[0].y, 0.0);
        assert_eq!(damage[0].width, 15.0);
        assert_eq!(damage[0].height, 10.0);
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
    #[ignore = "requires a macOS session with a Metal-capable device"]
    #[test]
    fn macos_device_surface_and_atlas_upload_are_live() {
        use yu_core::Revision;
        use yu_font::{
            FontFaceId, GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics,
            GlyphRasterKey, RasterizedGlyph,
        };
        use yu_scene::{GlyphPrimitive, Point, Rect, Rgba8, SceneBuilder};

        let device = MetalDevice::system_default().expect("Metal device");
        assert_ne!(device.registry_id(), 0);
        let config = MetalSurfaceConfig::new(320.0, 180.0, 2.0).expect("surface config");
        let mut surface = MetalSurface::new(device.clone(), config).expect("Metal surface");
        assert_eq!(surface.generation(), 0);
        surface
            .resize(MetalSurfaceConfig::new(640.0, 360.0, 2.0).expect("resize config"))
            .expect("surface resize");
        assert_eq!(surface.generation(), 1);

        let key =
            GlyphRasterKey::new(FontFaceId::from_raw(1), GlyphId::from_raw(7), 14.0).expect("key");
        let bitmap = GlyphBitmap::new(2, 2, 2, vec![255; 4]).expect("bitmap");
        let metrics = GlyphMetrics::new(0.0, 2.0, 2.0).expect("metrics");
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(8, 8, 1).expect("atlas config"));
        let entry = atlas
            .insert(RasterizedGlyph::new(key, metrics, bitmap))
            .expect("atlas entry");
        let mut scene_builder = SceneBuilder::new(
            Revision::INITIAL,
            Rect::new(0.0, 0.0, 32.0, 32.0).expect("viewport"),
        )
        .expect("scene builder");
        scene_builder
            .glyph(GlyphPrimitive::new(
                entry,
                Point::new(4.0, 12.0),
                Rgba8::white(),
            ))
            .expect("glyph");
        let plan = yu_render::RenderPlanBuilder::new()
            .build(&scene_builder.finish(), &atlas)
            .expect("render plan");
        let mut uploader = MetalUploader::new(device.clone());
        let mut gpu_atlas = MetalAtlas::new();
        assert_eq!(
            gpu_atlas
                .sync_plan(&mut uploader, &plan)
                .expect("alpha texture"),
            1
        );
        assert_eq!(gpu_atlas.page_count(), 1);

        let mut frame_renderer = MetalFrameRenderer::new(device).expect("command queue/pipeline");
        let result = frame_renderer.render_plan(&surface, &plan, &gpu_atlas);
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
        plan: RenderPlan,
        atlas: MetalAtlas,
        first: Option<Result<(), MetalRenderError>>,
        second: Option<Result<(), MetalRenderError>>,
        attachment_error: Option<MetalRenderError>,
        host_created: bool,
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
                state.first = Some(state.renderer.render_plan(
                    &state.surface,
                    &state.plan,
                    &state.atlas,
                ));
                drop(attachment);
            }
            Err(error) => {
                state.attachment_error = Some(error);
            }
        }

        if state.attachment_error.is_none() {
            let resize = MetalSurfaceConfig::new(300.0, 160.0, 2.0)
                .and_then(|config| state.surface.resize(config).map(|()| config));
            if resize.is_ok() {
                match unsafe { state.surface.attach_to_view(view) } {
                    Ok(attachment) => {
                        state.second = Some(state.renderer.render_plan(
                            &state.surface,
                            &state.plan,
                            &state.atlas,
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
        use yu_core::Revision;
        use yu_render::RenderPlanBuilder;
        use yu_scene::{Rect, SceneBuilder};

        let device = MetalDevice::system_default().expect("Metal device");
        let surface = MetalSurface::new(
            device.clone(),
            MetalSurfaceConfig::new(320.0, 180.0, 2.0).expect("surface config"),
        )
        .expect("surface");
        let mut scene = SceneBuilder::new(
            Revision::INITIAL,
            Rect::new(0.0, 0.0, 320.0, 180.0).expect("viewport"),
        )
        .expect("scene");
        scene
            .fill_rect(
                Rect::new(0.0, 0.0, 320.0, 180.0).expect("background"),
                Rgba8::new(24, 28, 36, 255),
            )
            .expect("background primitive");
        scene
            .fill_rect(
                Rect::new(24.0, 24.0, 120.0, 48.0).expect("accent"),
                Rgba8::new(220, 230, 240, 255),
            )
            .expect("accent primitive");
        let atlas = yu_font::GlyphAtlas::new(
            yu_font::GlyphAtlasConfig::new(8, 8, 1).expect("atlas config"),
        );
        let plan = RenderPlanBuilder::new()
            .build(&scene.finish(), &atlas)
            .expect("render plan");
        let state = AppKitProbeState {
            surface,
            renderer: MetalFrameRenderer::new(device).expect("renderer"),
            plan,
            atlas: MetalAtlas::new(),
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
        assert!(matches!(
            state.first,
            Some(Ok(())) | Some(Err(MetalRenderError::DrawableUnavailable))
        ));
        assert!(matches!(
            state.second,
            Some(Ok(())) | Some(Err(MetalRenderError::DrawableUnavailable))
        ));
    }
}
