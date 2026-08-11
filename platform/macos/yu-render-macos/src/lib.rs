#![allow(clippy::missing_const_for_fn)]
// cfg-gated platform branches use explicit returns so the macOS and stub
// implementations remain structurally parallel.
#![allow(clippy::needless_return)]

//! macOS Metal boundary for Yu's backend-neutral render plan.
//!
//! The Objective-C bridge in `native/metal_bridge.m` owns only the calls that
//! require Apple framework types. Rust owns device/surface/texture lifetime,
//! validates all dimensions, and exposes no native pointer to shared editor
//! state. This crate deliberately stops before drawable acquisition,
//! command encoding, presentation, or window creation.

use std::error::Error;
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;

use yu_render::{AtlasPageUpload, RenderUploader};

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_void;

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
        pub fn yu_metal_release(object: *mut c_void);
    }
}

/// Errors raised by the macOS Metal boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetalRenderError {
    UnsupportedPlatform,
    DeviceUnavailable,
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

/// A configured CAMetalLayer that is not attached to an NSView yet.
pub struct MetalSurface {
    device: MetalDevice,
    #[cfg(target_os = "macos")]
    raw_layer: NonNull<std::ffi::c_void>,
    config: MetalSurfaceConfig,
    generation: u64,
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
        let mut uploader = MetalUploader::new(device);
        let texture = uploader
            .upload_alpha_page(&plan.uploads()[0])
            .expect("alpha texture");
        assert_eq!(texture.width(), 8);
        assert_eq!(texture.height(), 8);
    }
}
