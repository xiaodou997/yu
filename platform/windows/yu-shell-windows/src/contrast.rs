//! Native contrast facts; this module never changes system settings.
use std::mem::size_of;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use yu_scene::{ContrastPalette, Rgba8};
pub(crate) fn rgba(color: COLORREF) -> Rgba8 {
    Rgba8::new(
        color.0 as u8,
        (color.0 >> 8) as u8,
        (color.0 >> 16) as u8,
        255,
    )
}
pub(crate) fn colorref(color: Rgba8) -> COLORREF {
    COLORREF(
        u32::from(color.red()) | (u32::from(color.green()) << 8) | (u32::from(color.blue()) << 16),
    )
}
pub(crate) fn system_contrast() -> Option<ContrastPalette> {
    let mut settings = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    if unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            settings.cbSize,
            Some(std::ptr::from_mut(&mut settings).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_err()
        || settings.dwFlags & HCF_HIGHCONTRASTON == Default::default()
    {
        return None;
    }
    Some(ContrastPalette {
        background: rgba(COLORREF(unsafe { GetSysColor(COLOR_WINDOW) })),
        foreground: rgba(COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) })),
        selection: rgba(COLORREF(unsafe { GetSysColor(COLOR_HIGHLIGHT) })),
        selected_text: rgba(COLORREF(unsafe { GetSysColor(COLOR_HIGHLIGHTTEXT) })),
    })
}
