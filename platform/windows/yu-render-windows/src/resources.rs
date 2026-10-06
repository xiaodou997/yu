#![cfg(target_os = "windows")]

//! CPU resource preparation. COM objects stay on their owning worker thread.
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::{Interface, PCWSTR};
use yu_assets::{DecodedImage, EmbeddedRenderPayload, ImageDimensions};

pub struct ResourceRasterizer {
    wic: IWICImagingFactory,
    svg: Option<resvg::usvg::Options<'static>>,
}

impl ResourceRasterizer {
    /// The caller initializes COM before constructing this worker-local object.
    pub fn new() -> Result<Self, String> {
        let wic = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| error.to_string())?;
        Ok(Self { wic, svg: None })
    }

    pub fn decode_local(
        &mut self,
        path: &Path,
        max_dimension: u32,
    ) -> Result<DecodedImage, String> {
        let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
            return Err("Image exceeds the 32 MiB encoded resource budget".into());
        }
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
        {
            let markup = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
            return self.raster_svg(&markup, max_dimension, 1.0);
        }
        let filename: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let decoder = unsafe {
            self.wic.CreateDecoderFromFilename(
                PCWSTR(filename.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
        }
        .map_err(|error| error.to_string())?;
        let frame = unsafe { decoder.GetFrame(0) }.map_err(|error| error.to_string())?;
        let mut width = 0;
        let mut height = 0;
        unsafe { frame.GetSize(&mut width, &mut height) }.map_err(|error| error.to_string())?;
        let intrinsic = ImageDimensions::new(width, height).ok_or("Invalid image dimensions")?;
        let (target_width, target_height) = thumbnail_size(width, height, max_dimension)?;
        let source: IWICBitmapSource = if (width, height) == (target_width, target_height) {
            frame.cast().map_err(|error| error.to_string())?
        } else {
            let scaler =
                unsafe { self.wic.CreateBitmapScaler() }.map_err(|error| error.to_string())?;
            unsafe {
                scaler.Initialize(
                    &frame,
                    target_width,
                    target_height,
                    WICBitmapInterpolationModeFant,
                )
            }
            .map_err(|error| error.to_string())?;
            scaler.cast().map_err(|error| error.to_string())?
        };
        let converter =
            unsafe { self.wic.CreateFormatConverter() }.map_err(|error| error.to_string())?;
        unsafe {
            converter.Initialize(
                &source,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }
        .map_err(|error| error.to_string())?;
        let mut pixels = vec![0; target_width as usize * target_height as usize * 4];
        unsafe { converter.CopyPixels(std::ptr::null(), target_width * 4, &mut pixels) }
            .map_err(|error| error.to_string())?;
        DecodedImage::new(target_width, target_height, pixels)
            .map(|image| image.with_intrinsic(intrinsic))
            .map_err(|error| error.to_string())
    }

    pub fn raster_embedded(
        &mut self,
        payload: &EmbeddedRenderPayload,
        max_dimension: u32,
        raster_scale: f32,
    ) -> Result<DecodedImage, String> {
        match payload {
            EmbeddedRenderPayload::Svg { markup, .. } => {
                self.raster_svg(markup, max_dimension, raster_scale)
            }
            EmbeddedRenderPayload::Rgba8 { dimensions, pixels } => {
                DecodedImage::new(dimensions.width(), dimensions.height(), pixels.clone())
                    .map_err(|error| error.to_string())
            }
        }
    }

    fn raster_svg(
        &mut self,
        markup: &str,
        max_dimension: u32,
        raster_scale: f32,
    ) -> Result<DecodedImage, String> {
        if markup.len() > yu_assets::EMBEDDED_SVG_MAX_MARKUP_BYTES {
            return Err("SVG exceeds the 4 MiB markup budget".into());
        }
        let options = self.svg.get_or_insert_with(|| {
            let mut options = resvg::usvg::Options {
                font_family: "Segoe UI".into(),
                ..Default::default()
            };
            options.fontdb_mut().load_system_fonts();
            options.fontdb_mut().set_sans_serif_family("Segoe UI");
            // The shared loader only resolves the document's explicit image
            // destination. SVG cannot open additional files or network URLs.
            options.image_href_resolver = resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            };
            options
        });
        let tree =
            resvg::usvg::Tree::from_str(markup, options).map_err(|error| error.to_string())?;
        let size = tree.size();
        let width = size.width().ceil() as u32;
        let height = size.height().ceil() as u32;
        let intrinsic = ImageDimensions::new(width, height).ok_or("Invalid SVG dimensions")?;
        if !raster_scale.is_finite() || !(0.25..=8.0).contains(&raster_scale) {
            return Err("Invalid SVG raster scale".into());
        }
        let (target_width, target_height) = thumbnail_size(
            (size.width() * raster_scale).ceil() as u32,
            (size.height() * raster_scale).ceil() as u32,
            max_dimension,
        )?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(target_width, target_height)
            .ok_or("SVG allocation failed")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(
                target_width as f32 / size.width(),
                target_height as f32 / size.height(),
            ),
            &mut pixmap.as_mut(),
        );
        // resvg produces premultiplied RGBA; image textures use straight RGBA.
        let mut pixels = pixmap.take();
        for pixel in pixels.as_chunks_mut::<4>().0.iter_mut() {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
        DecodedImage::new(target_width, target_height, pixels)
            .map(|image| image.with_intrinsic(intrinsic))
            .map_err(|error| error.to_string())
    }
}

fn thumbnail_size(width: u32, height: u32, max_dimension: u32) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 || width.max(height) > 100_000 {
        return Err("Invalid or excessive intrinsic image dimensions".into());
    }
    let limit = max_dimension.clamp(1, 4096);
    let scale = (limit as f64 / width.max(height) as f64).min(1.0);
    Ok((
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downsampling_preserves_aspect_ratio_and_bounds() {
        assert_eq!(
            thumbnail_size(8000, 4000, 2048).expect("valid native resource test"),
            (2048, 1024)
        );
        assert_eq!(
            thumbnail_size(1, 9000, 2048).expect("valid native resource test"),
            (1, 2048)
        );
        assert!(thumbnail_size(0, 3, 2048).is_err());
        assert!(thumbnail_size(u32::MAX, 3, 2048).is_err());
    }
    #[test]
    fn svg_alpha_is_straight_and_external_images_are_not_loaded() {
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
            .ok()
            .expect("valid native resource test");
        }
        {
            let mut rasterizer = ResourceRasterizer::new().expect("valid native resource test");
            let image = rasterizer.raster_svg(r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#ff0000" fill-opacity="0.5"/></svg>"##, 32, 1.0).expect("valid native resource test");
            assert_eq!(&image.pixels()[..3], &[255, 0, 0]);
            assert!((127..=128).contains(&image.pixels()[3]));
            let empty = rasterizer.raster_svg(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><image href="file:///C:/missing.png" width="8" height="8"/></svg>"#, 32, 1.0).expect("valid native resource test");
            assert!(empty.pixels().iter().all(|byte| *byte == 0));
        }
        unsafe {
            windows::Win32::System::Com::CoUninitialize();
        }
    }
}
