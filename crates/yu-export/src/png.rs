//! Bounded continuous-image settings. Limits are fixed before raster allocation.
use crate::document::{HtmlDocument, HtmlOptions};
use crate::paged::{PageSettings, prepare_flow_packet};
use serde_json::{Value, json};

pub const MAX_PNG_SIDE: u32 = 32_768;
pub const MAX_PNG_PIXELS: u64 = 16 * 1024 * 1024;
pub const MAX_PNG_TOTAL_PIXELS: u64 = 128 * 1024 * 1024;
pub const MAX_PNG_SEGMENTS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct PngSettings {
    pub width: u32,
    pub scale: u32,
}
impl PngSettings {
    pub fn from_config(config: &Value) -> Result<Self, String> {
        let number = |key, default| match config.get(key) {
            None => Ok(default),
            Some(value) => value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("PNG 尺寸参数无效".to_owned()),
        };
        let width = number("width", 800)?;
        let scale = number("scale", 1)?;
        if !(320..=2048).contains(&width) || !(1..=2).contains(&scale) {
            return Err("PNG 宽度必须为 320–2048，倍率必须为 1× 或 2×".into());
        }
        Ok(Self { width, scale })
    }
    pub fn pixel_width(self) -> u32 {
        self.width * self.scale
    }
    pub fn pixel_height_limit(self) -> u32 {
        (MAX_PNG_PIXELS / u64::from(self.pixel_width())).min(u64::from(MAX_PNG_SIDE)) as u32
    }
    pub fn validate_sizes(self, sizes: &[[u32; 2]]) -> Result<(), String> {
        if sizes.is_empty() || sizes.len() > MAX_PNG_SEGMENTS {
            return Err("PNG 分段数量超过 64 段预算".into());
        }
        let mut total = 0u64;
        for &[width, height] in sizes {
            let pixels = u64::from(width)
                .checked_mul(u64::from(height))
                .ok_or("PNG 像素乘法溢出")?;
            if width != self.pixel_width()
                || height == 0
                || height > MAX_PNG_SIDE
                || pixels > MAX_PNG_PIXELS
            {
                return Err("PNG 单段超过 32768 边长或 16 Mi 像素预算".into());
            }
            total = total.checked_add(pixels).ok_or("PNG 总像素溢出")?;
            if total > MAX_PNG_TOTAL_PIXELS {
                return Err("PNG 超过 128 Mi 总像素预算".into());
            }
        }
        Ok(())
    }
}
pub fn prepare_png_packet(
    document: &HtmlDocument,
    settings: PngSettings,
    style: &HtmlOptions,
) -> Result<Vec<u8>, String> {
    // The existing flow uses 0.75pt per logical pixel. Raster output reverses
    // that conversion, without a screen scale or an intermediate PDF page.
    let page = PageSettings {
        width: f64::from(settings.width) * 0.75,
        height: f64::from(settings.pixel_height_limit() / settings.scale) * 0.75,
        margin: 18.0,
        page_numbers: false,
    };
    prepare_flow_packet(
        document,
        page,
        Some(
            json!({"scale":settings.scale,"pixelWidth":settings.pixel_width(),
        "foreground":style.foreground,"background":style.background,"link":style.link,"fontSize":style.font_size,
        "maxPixels":MAX_PNG_PIXELS,"maxTotalPixels":MAX_PNG_TOTAL_PIXELS,"maxSide":MAX_PNG_SIDE,"maxSegments":MAX_PNG_SEGMENTS}),
        ),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_options_and_pixel_boundaries() {
        let s = PngSettings::from_config(&json!({"width":1024,"scale":2})).expect("settings");
        assert_eq!(s.pixel_width(), 2048);
        assert_eq!(s.pixel_height_limit(), 8192);
        assert!(s.validate_sizes(&[[2048, 8192]]).is_ok());
        assert!(s.validate_sizes(&[[2048, 8193]]).is_err());
        assert!(s.validate_sizes(&[[2048, 8192]; 8]).is_ok());
        assert!(s.validate_sizes(&[[2048, 8192]; 9]).is_err());
        for value in [
            json!({"scale":3}),
            json!({"scale":0}),
            json!({"width":319}),
            json!({"width":2049}),
            json!({"width":-1}),
        ] {
            assert!(PngSettings::from_config(&value).is_err());
        }
        assert!(s.validate_sizes(&[]).is_err());
        assert!(s.validate_sizes(&[[u32::MAX, u32::MAX]]).is_err());
    }
}
