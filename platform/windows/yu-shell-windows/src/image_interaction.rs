//! Native Windows image-selection chrome and the modal image-properties editor.
//!
//! Markdown syntax and mutations stay in yu-editor.  This module only owns
//! transient Win32 controls, mirroring the macOS inspector/property-sheet
//! interaction without introducing a second Markdown parser.
#![cfg(target_os = "windows")]

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawFocusRect, DrawTextW, FillRect, HDC,
    InvalidateRect, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::SystemServices::SS_OWNERDRAW;
use windows::Win32::UI::Controls::{BST_CHECKED, DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_OWNERDRAW, CreateWindowExW, DestroyWindow, DispatchMessageW,
    EN_KILLFOCUS, ES_AUTOHSCROLL, GetMessageW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    HMENU, IsDialogMessageW, MSG, MoveWindow, SW_HIDE, SW_SHOW, SendMessageW, SetWindowPos,
    SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_NCDESTROY, WM_SETFONT, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS,
    WS_DISABLED, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
};
use windows::core::{PCWSTR, w};
use yu_assets::ImageDimensions;
use yu_core::{Revision, TextRange};
use yu_editor::ImageProperties;
use yu_scene::Rect;

use crate::chrome::{Brush, Font, pointer_inside, round_fill, tab_subclass};
use crate::{Locale, ShellError, Strings, WindowMetrics};

pub(crate) const ID_IMAGE_SOURCE: u16 = 2201;
pub(crate) const ID_IMAGE_REPLACE: u16 = 2203;
pub(crate) const ID_IMAGE_SIZE: u16 = 2204;
pub(crate) const ID_IMAGE_MORE: u16 = 2205;

const ID_IMAGE_BORDER_TOP: u16 = 2280;
const ID_IMAGE_BORDER_BOTTOM: u16 = 2281;
const ID_IMAGE_BORDER_LEFT: u16 = 2282;
const ID_IMAGE_BORDER_RIGHT: u16 = 2283;

const ID_PROPERTIES_DESTINATION: u16 = 2251;
const ID_PROPERTIES_ALTERNATIVE: u16 = 2252;
const ID_PROPERTIES_WIDTH: u16 = 2253;
const ID_PROPERTIES_HEIGHT: u16 = 2254;
const ID_PROPERTIES_LOCK: u16 = 2255;
const ID_PROPERTIES_APPLY: u16 = 2256;
const ID_PROPERTIES_CANCEL: u16 = 2257;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn error(error: impl std::fmt::Display) -> ShellError {
    ShellError::Platform(error.to_string())
}

fn child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    id: u16,
    style: u32,
    tabstop: bool,
) -> Result<HWND, ShellError> {
    let text = wide(text);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD
                | WS_CLIPSIBLINGS
                | WINDOW_STYLE(style)
                | if tabstop { WS_TABSTOP } else { WINDOW_STYLE(0) },
            0,
            0,
            1,
            1,
            parent,
            HMENU(id as usize as *mut _),
            None,
            None,
        )
    }
    .map_err(error)
}

fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

pub(crate) fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0_u16; length as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..written])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageScalePreset {
    Percent(u32),
    Original,
    FitColumn,
}

#[derive(Clone, Debug)]
pub(crate) struct ImageInteractionState {
    pub revision: Revision,
    pub source: TextRange,
    pub document_bounds: Rect,
    pub source_text: String,
    pub properties: ImageProperties,
    pub displayed_destination: String,
    pub intrinsic: Option<ImageDimensions>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImagePropertyDraft {
    pub destination: String,
    pub alternative: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

pub(crate) fn is_inspector_control(id: u16) -> bool {
    matches!(
        id,
        ID_IMAGE_SOURCE | ID_IMAGE_REPLACE | ID_IMAGE_SIZE | ID_IMAGE_MORE
    )
}

pub(crate) struct ImageInspector {
    borders: [HWND; 4],
    panel: HWND,
    source: HWND,
    replace: HWND,
    size: HWND,
    more: HWND,
    identity: Option<(Revision, TextRange)>,
    visible: bool,
    locale: Locale,
    font: Option<Font>,
    source_font: Option<Font>,
    font_dpi: u32,
    palette: InspectorPalette,
    appearance: yu_workspace::Appearance,
    contrast: Option<yu_scene::ContrastPalette>,
}

struct InspectorPalette {
    canvas: Brush,
    background: Brush,
    field: Brush,
    field_border: Brush,
    border: Brush,
    hover: Brush,
    accent: Brush,
    shadow_far: Brush,
    shadow_mid: Brush,
    shadow_near: Brush,
    text: windows::Win32::Foundation::COLORREF,
    muted: windows::Win32::Foundation::COLORREF,
    field_color: windows::Win32::Foundation::COLORREF,
}

impl InspectorPalette {
    fn new(
        appearance: yu_workspace::Appearance,
        contrast: Option<yu_scene::ContrastPalette>,
    ) -> Self {
        let dark = matches!(
            appearance,
            yu_workspace::Appearance::Dark | yu_workspace::Appearance::YuDark
        );
        let color =
            |light, dark_color| crate::contrast::colorref(if dark { dark_color } else { light });
        let background = contrast.map_or_else(
            || {
                color(
                    yu_scene::Rgba8::new(241, 245, 250, 255),
                    yu_scene::Rgba8::new(46, 51, 61, 255),
                )
            },
            |value| crate::contrast::colorref(value.background),
        );
        let field = contrast.map_or_else(
            || {
                color(
                    yu_scene::Rgba8::new(255, 255, 255, 255),
                    yu_scene::Rgba8::new(32, 36, 44, 255),
                )
            },
            |value| crate::contrast::colorref(value.background),
        );
        let text = contrast.map_or_else(
            || {
                color(
                    yu_scene::Rgba8::new(52, 58, 68, 255),
                    yu_scene::Rgba8::new(232, 234, 238, 255),
                )
            },
            |value| crate::contrast::colorref(value.foreground),
        );
        let muted = contrast.map_or_else(
            || {
                color(
                    yu_scene::Rgba8::new(83, 96, 115, 255),
                    yu_scene::Rgba8::new(185, 195, 212, 255),
                )
            },
            |value| crate::contrast::colorref(value.foreground),
        );
        let border = contrast.map_or_else(
            || {
                color(
                    yu_scene::Rgba8::new(180, 194, 214, 255),
                    yu_scene::Rgba8::new(94, 107, 127, 255),
                )
            },
            |value| crate::contrast::colorref(value.foreground),
        );
        let hover = contrast.map_or_else(
            || {
                color(
                    yu_scene::Rgba8::new(222, 233, 248, 255),
                    yu_scene::Rgba8::new(66, 79, 101, 255),
                )
            },
            |value| crate::contrast::colorref(value.selection),
        );
        let accent = contrast.map_or_else(
            || {
                color(
                    yu_scene::Rgba8::new(76, 127, 206, 255),
                    yu_scene::Rgba8::new(125, 168, 237, 255),
                )
            },
            |value| crate::contrast::colorref(value.foreground),
        );
        Self {
            canvas: Brush::new(contrast.map_or_else(
                || crate::contrast::colorref(appearance.background()),
                |value| crate::contrast::colorref(value.background),
            )),
            background: Brush::new(background),
            field: Brush::new(field),
            field_border: Brush::new(contrast.map_or_else(
                || {
                    color(
                        yu_scene::Rgba8::new(208, 218, 232, 255),
                        yu_scene::Rgba8::new(78, 91, 111, 255),
                    )
                },
                |value| crate::contrast::colorref(value.foreground),
            )),
            border: Brush::new(border),
            hover: Brush::new(hover),
            accent: Brush::new(accent),
            shadow_far: Brush::new(color(
                yu_scene::Rgba8::new(244, 247, 251, 255),
                yu_scene::Rgba8::new(24, 27, 33, 255),
            )),
            shadow_mid: Brush::new(color(
                yu_scene::Rgba8::new(234, 239, 246, 255),
                yu_scene::Rgba8::new(17, 20, 26, 255),
            )),
            shadow_near: Brush::new(color(
                yu_scene::Rgba8::new(220, 229, 241, 255),
                yu_scene::Rgba8::new(10, 13, 19, 255),
            )),
            text,
            muted,
            field_color: field,
        }
    }
}

impl ImageInspector {
    pub(crate) fn new(parent: HWND, locale: Locale) -> Result<Self, ShellError> {
        let border = |id| {
            child(
                parent,
                w!("STATIC"),
                "",
                id,
                SS_OWNERDRAW.0 | WS_DISABLED.0,
                false,
            )
        };
        let panel = child(
            parent,
            w!("STATIC"),
            "",
            2270,
            SS_OWNERDRAW.0 | WS_DISABLED.0,
            false,
        )?;
        let source = child(
            parent,
            w!("EDIT"),
            "",
            ID_IMAGE_SOURCE,
            ES_AUTOHSCROLL as u32,
            true,
        )?;
        if !unsafe { SetWindowSubclass(source, Some(source_subclass), 1, 0) }.as_bool() {
            return Err(error(windows::core::Error::from_win32()));
        }
        let replace = child(
            parent,
            w!("BUTTON"),
            replace_label(locale),
            ID_IMAGE_REPLACE,
            BS_OWNERDRAW as u32,
            true,
        )?;
        let size = child(
            parent,
            w!("BUTTON"),
            "100% ▾",
            ID_IMAGE_SIZE,
            BS_OWNERDRAW as u32,
            true,
        )?;
        let more = child(
            parent,
            w!("BUTTON"),
            "···",
            ID_IMAGE_MORE,
            BS_OWNERDRAW as u32,
            true,
        )?;
        for button in [replace, size, more] {
            if !unsafe { SetWindowSubclass(button, Some(tab_subclass), 1, 0) }.as_bool() {
                return Err(error(windows::core::Error::from_win32()));
            }
        }
        let appearance = yu_workspace::Appearance::YuLight;
        let result = Self {
            borders: [
                border(ID_IMAGE_BORDER_TOP)?,
                border(ID_IMAGE_BORDER_BOTTOM)?,
                border(ID_IMAGE_BORDER_LEFT)?,
                border(ID_IMAGE_BORDER_RIGHT)?,
            ],
            panel,
            source,
            replace,
            size,
            more,
            identity: None,
            visible: false,
            locale,
            font: None,
            source_font: None,
            font_dpi: 0,
            palette: InspectorPalette::new(appearance, None),
            appearance,
            contrast: None,
        };
        result.hide_windows();
        Ok(result)
    }

    pub(crate) fn source_text(&self) -> Option<String> {
        self.visible.then(|| window_text(self.source))
    }

    pub(crate) fn is_source(&self, hwnd: HWND) -> bool {
        self.visible && hwnd == self.source && !self.is_composing(hwnd)
    }

    pub(crate) fn is_composing(&self, hwnd: HWND) -> bool {
        hwnd == self.source
            && unsafe {
                !windows::Win32::UI::WindowsAndMessaging::GetPropW(hwnd, w!("YuImageComposition"))
                    .0
                    .is_null()
            }
    }

    pub(crate) const fn button(&self, id: u16) -> Option<HWND> {
        match id {
            ID_IMAGE_REPLACE => Some(self.replace),
            ID_IMAGE_SIZE => Some(self.size),
            ID_IMAGE_MORE => Some(self.more),
            _ => None,
        }
    }

    pub(crate) fn hide(&mut self) {
        self.identity = None;
        self.hide_for_scroll();
    }

    pub(crate) fn hide_for_scroll(&mut self) {
        self.visible = false;
        self.hide_windows();
    }

    fn hide_windows(&self) {
        unsafe {
            for hwnd in self.borders.iter().copied().chain([
                self.panel,
                self.source,
                self.replace,
                self.size,
                self.more,
            ]) {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    pub(crate) fn present(
        &mut self,
        state: &ImageInteractionState,
        image: RECT,
        viewport: RECT,
        inspector_bounds: RECT,
        metrics: WindowMetrics,
        theme: (yu_workspace::Appearance, Option<yu_scene::ContrastPalette>),
    ) {
        let (appearance, contrast) = theme;
        if self.appearance != appearance || self.contrast != contrast {
            self.palette = InspectorPalette::new(appearance, contrast);
            self.appearance = appearance;
            self.contrast = contrast;
        }
        if self.font_dpi != metrics.dpi()
            && let (Ok(font), Ok(source_font)) = (
                Font::for_dpi(metrics.dpi(), 12.0, false, self.locale),
                Font::monospace(metrics.dpi(), 11.0),
            )
        {
            unsafe {
                for hwnd in [self.replace, self.size, self.more] {
                    SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0.0 as usize), LPARAM(1));
                }
                SendMessageW(
                    self.source,
                    WM_SETFONT,
                    WPARAM(source_font.0.0 as usize),
                    LPARAM(1),
                );
            }
            self.font = Some(font);
            self.source_font = Some(source_font);
            self.font_dpi = metrics.dpi();
        }
        let identity = Some((state.revision, state.source));
        if self.identity != identity {
            set_text(self.source, &state.source_text);
        }
        self.identity = identity;
        let percent = state.intrinsic.map(|size| {
            (state.document_bounds.width() / size.width() as f32 * 100.0).round() as u32
        });
        set_text(
            self.size,
            &percent.map_or_else(|| "↔ ▾".to_owned(), |percent| format!("{percent}% ▾")),
        );
        let thickness = metrics.px(1.0).max(1);
        let border_rects = [
            child_rect(image.left, image.top, image.right - image.left, thickness),
            child_rect(
                image.left,
                image.bottom - thickness,
                image.right - image.left,
                thickness,
            ),
            child_rect(image.left, image.top, thickness, image.bottom - image.top),
            child_rect(
                image.right - thickness,
                image.top,
                thickness,
                image.bottom - image.top,
            ),
        ];
        unsafe {
            for (hwnd, rect) in self.borders.iter().zip(border_rects) {
                if let Some(rect) = intersect_rect(rect, viewport) {
                    place(*hwnd, rect);
                    let _ = ShowWindow(*hwnd, SW_SHOW);
                } else {
                    let _ = ShowWindow(*hwnd, SW_HIDE);
                }
            }
        }
        let panel = inspector_panel_rect(image, inspector_bounds, metrics);
        if panel.bottom - panel.top < metrics.px(24.0)
            || panel.right - panel.left < metrics.px(100.0)
        {
            self.hide_for_scroll();
            return;
        }
        let padding = metrics.px(4.0);
        let row_h = metrics
            .px(28.0)
            .min(panel.bottom - panel.top - padding * 2)
            .max(1);
        let top = panel.top + (panel.bottom - panel.top - row_h) / 2;
        let content = panel.right - panel.left - padding * 2;
        let compact = content < metrics.px(300.0);
        let more_w = metrics.px(28.0);
        let size_w = if compact { 0 } else { metrics.px(68.0) };
        let replace_w = if compact { 0 } else { metrics.px(58.0) };
        let label_w = if compact { 0 } else { metrics.px(34.0) };
        let field_w = (content - label_w - replace_w - size_w - more_w - padding * 3).max(1);
        let source_left = panel.left + padding + label_w;
        unsafe {
            place(
                self.panel,
                RECT {
                    right: panel.right + metrics.px(4.0),
                    bottom: panel.bottom + metrics.px(4.0),
                    ..panel
                },
            );
            place(
                self.source,
                child_rect(
                    source_left + padding,
                    top + metrics.px(4.0),
                    field_w - padding * 2,
                    row_h - metrics.px(8.0),
                ),
            );
            place(
                self.replace,
                child_rect(
                    source_left + field_w + padding,
                    top,
                    replace_w.max(1),
                    row_h,
                ),
            );
            place(
                self.size,
                child_rect(
                    source_left + field_w + padding + replace_w,
                    top,
                    size_w.max(1),
                    row_h,
                ),
            );
            place(
                self.more,
                child_rect(panel.right - padding - more_w, top, more_w, row_h),
            );
            for hwnd in [self.panel, self.source, self.more] {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            for hwnd in [self.replace, self.size] {
                let _ = ShowWindow(hwnd, if compact { SW_HIDE } else { SW_SHOW });
            }
            let _ = InvalidateRect(self.panel, None, false);
        }
        self.visible = true;
    }

    pub(crate) fn control_color(&self, dc: HDC, hwnd: HWND) -> Option<LRESULT> {
        if hwnd != self.source {
            return None;
        }
        unsafe {
            SetTextColor(dc, self.palette.text);
            SetBkColor(dc, self.palette.field_color);
        }
        Some(LRESULT(self.palette.field.0.0 as isize))
    }

    pub(crate) fn draw(&self, item: &DRAWITEMSTRUCT) -> bool {
        let hwnd = item.hwndItem;
        let metrics = WindowMetrics::new(0, 0, self.font_dpi.max(96));
        let mut rect = item.rcItem;
        unsafe {
            if self.borders.contains(&hwnd) {
                FillRect(item.hDC, &rect, self.palette.accent.0);
                return true;
            }
            if hwnd == self.panel {
                FillRect(item.hDC, &rect, self.palette.canvas.0);
                if self.contrast.is_none() {
                    for (offset, brush) in [
                        (4.0, &self.palette.shadow_far),
                        (3.0, &self.palette.shadow_mid),
                        (2.0, &self.palette.shadow_near),
                    ] {
                        let shadow = RECT {
                            left: rect.left + metrics.px(offset),
                            top: rect.top + metrics.px(offset),
                            right: rect.right - metrics.px(4.0 - offset),
                            bottom: rect.bottom - metrics.px(4.0 - offset),
                        };
                        round_fill(item.hDC, &shadow, metrics.px(8.0), brush.0);
                    }
                }
                rect.right -= metrics.px(4.0);
                rect.bottom -= metrics.px(4.0);
                round_fill(item.hDC, &rect, metrics.px(8.0), self.palette.border.0);
                let stroke = metrics.px(1.0).max(1);
                rect.left += stroke;
                rect.top += stroke;
                rect.right -= stroke;
                rect.bottom -= stroke;
                round_fill(item.hDC, &rect, metrics.px(8.0), self.palette.background.0);
                let compact = rect.right - rect.left < metrics.px(312.0);
                let field = RECT {
                    left: metrics.px(if compact { 4.0 } else { 38.0 }),
                    top: metrics.px(6.0),
                    right: rect.right + stroke - metrics.px(if compact { 44.0 } else { 170.0 }),
                    bottom: rect.bottom + stroke - metrics.px(6.0),
                };
                round_fill(
                    item.hDC,
                    &field,
                    metrics.px(5.0),
                    self.palette.field_border.0,
                );
                let field_inner = RECT {
                    left: field.left + stroke,
                    top: field.top + stroke,
                    right: field.right - stroke,
                    bottom: field.bottom - stroke,
                };
                round_fill(
                    item.hDC,
                    &field_inner,
                    metrics.px(5.0),
                    self.palette.field.0,
                );
                if !compact {
                    let mut label = RECT {
                        left: metrics.px(7.0),
                        top: 0,
                        right: metrics.px(39.0),
                        bottom: rect.bottom,
                    };
                    let mut text = wide(image_label(self.locale));
                    let previous = self
                        .font
                        .as_ref()
                        .map(|font| SelectObject(item.hDC, font.0));
                    SetBkMode(item.hDC, TRANSPARENT);
                    SetTextColor(item.hDC, self.palette.muted);
                    let length = text.len() - 1;
                    DrawTextW(
                        item.hDC,
                        &mut text[..length],
                        &mut label,
                        DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                    );
                    if let Some(previous) = previous {
                        SelectObject(item.hDC, previous);
                    }
                }
                return true;
            }
            if ![self.replace, self.size, self.more].contains(&hwnd) {
                return false;
            }
            FillRect(item.hDC, &rect, self.palette.background.0);
            if pointer_inside(hwnd) || item.itemState.0 & ODS_SELECTED.0 != 0 {
                round_fill(item.hDC, &rect, metrics.px(5.0), self.palette.hover.0);
            }
            let previous = self
                .font
                .as_ref()
                .map(|font| SelectObject(item.hDC, font.0));
            SetBkMode(item.hDC, TRANSPARENT);
            SetTextColor(item.hDC, self.palette.text);
            let mut text = wide(&window_text(hwnd));
            let length = text.len() - 1;
            DrawTextW(
                item.hDC,
                &mut text[..length],
                &mut rect,
                DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
            if item.itemState.0 & ODS_FOCUS.0 != 0 {
                let _ = DrawFocusRect(item.hDC, &rect);
            }
            if let Some(previous) = previous {
                SelectObject(item.hDC, previous);
            }
        }
        true
    }
}

unsafe extern "system" fn source_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        RemovePropW, SetPropW, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION,
    };
    unsafe {
        if message == WM_IME_STARTCOMPOSITION {
            let _ = SetPropW(
                hwnd,
                w!("YuImageComposition"),
                windows::Win32::Foundation::HANDLE(std::ptr::dangling_mut()),
            );
        } else if message == WM_IME_ENDCOMPOSITION || message == WM_NCDESTROY {
            let _ = RemovePropW(hwnd, w!("YuImageComposition"));
            if message == WM_NCDESTROY {
                let _ = RemoveWindowSubclass(hwnd, Some(source_subclass), 1);
            }
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

unsafe fn place(hwnd: HWND, rect: RECT) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::HWND_TOP,
            rect.left,
            rect.top,
            (rect.right - rect.left).max(1),
            (rect.bottom - rect.top).max(1),
            windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        );
    }
}

fn replace_label(locale: Locale) -> &'static str {
    match locale {
        Locale::English => "Replace",
        Locale::SimplifiedChinese => "替换",
        Locale::TraditionalChinese => "替換",
        Locale::Japanese => "変更",
        Locale::Korean => "교체",
    }
}

fn image_label(locale: Locale) -> &'static str {
    match locale {
        Locale::English => "Image",
        Locale::SimplifiedChinese => "图片",
        Locale::TraditionalChinese => "圖片",
        Locale::Japanese => "画像",
        Locale::Korean => "이미지",
    }
}

fn intersect_rect(rect: RECT, viewport: RECT) -> Option<RECT> {
    let clipped = RECT {
        left: rect.left.max(viewport.left),
        top: rect.top.max(viewport.top),
        right: rect.right.min(viewport.right),
        bottom: rect.bottom.min(viewport.bottom),
    };
    (clipped.right > clipped.left && clipped.bottom > clipped.top).then_some(clipped)
}

fn inspector_panel_rect(image: RECT, viewport: RECT, metrics: WindowMetrics) -> RECT {
    let width = (viewport.right - viewport.left).max(1);
    let height = (viewport.bottom - viewport.top).max(1);
    let padding = metrics.px(2.0).min((width - 1) / 2).min((height - 1) / 2);
    let shadow = metrics.px(4.0).min((width - 1) / 2).min((height - 1) / 2);
    let panel_height = metrics.px(32.0).min((height - padding * 2 - shadow).max(1));
    let panel_width = metrics.px(640.0).min((width - padding * 2 - shadow).max(1));
    let left = image.left.clamp(
        viewport.left + padding,
        viewport.right - padding - shadow - panel_width,
    );
    let top = (image.top - metrics.px(8.0) - panel_height).clamp(
        viewport.top + padding,
        viewport.bottom - padding - shadow - panel_height,
    );
    child_rect(left, top, panel_width, panel_height)
}

struct PropertiesDialogState {
    strings: Strings,
    destination: HWND,
    alternative: HWND,
    width: HWND,
    height: HWND,
    lock: HWND,
    ratio: Option<f64>,
    done: bool,
    result: Option<ImagePropertyDraft>,
}

fn dimension(text: &str) -> Option<Option<u32>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Some(None)
    } else {
        trimmed
            .parse::<u32>()
            .ok()
            .filter(|value| (1..=100_000).contains(value))
            .map(Some)
    }
}

fn checked(hwnd: HWND) -> bool {
    unsafe { SendMessageW(hwnd, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 as u32 == BST_CHECKED.0 }
}

fn synchronize_dimension(state: &PropertiesDialogState, id: u16) {
    if !checked(state.lock) {
        return;
    }
    let Some(ratio) = state
        .ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
    else {
        return;
    };
    let (source, target, divide) = if id == ID_PROPERTIES_WIDTH {
        (state.width, state.height, true)
    } else if id == ID_PROPERTIES_HEIGHT {
        (state.height, state.width, false)
    } else {
        return;
    };
    let Some(Some(value)) = dimension(&window_text(source)) else {
        return;
    };
    let matched = if divide {
        f64::from(value) / ratio
    } else {
        f64::from(value) * ratio
    }
    .round();
    if (1.0..=100_000.0).contains(&matched) {
        set_text(target, &format!("{}", matched as u32));
    }
}

unsafe extern "system" fn properties_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let state = unsafe { &mut *(data as *mut PropertiesDialogState) };
    match message {
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as u16;
            let notification = (wparam.0 >> 16) as u16;
            if matches!(id, ID_PROPERTIES_WIDTH | ID_PROPERTIES_HEIGHT)
                && u32::from(notification) == EN_KILLFOCUS
            {
                synchronize_dimension(state, id);
                return LRESULT(0);
            }
            if matches!(id, ID_PROPERTIES_APPLY | 1) {
                let destination = window_text(state.destination);
                let alternative = window_text(state.alternative);
                let Some(mut width) = dimension(&window_text(state.width)) else {
                    show_invalid_properties(hwnd, state.strings);
                    return LRESULT(0);
                };
                let Some(mut height) = dimension(&window_text(state.height)) else {
                    show_invalid_properties(hwnd, state.strings);
                    return LRESULT(0);
                };
                if destination.trim().is_empty() {
                    show_invalid_properties(hwnd, state.strings);
                    return LRESULT(0);
                }
                if checked(state.lock)
                    && let Some(ratio) = state
                        .ratio
                        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
                {
                    match (width, height) {
                        (Some(w), None) => {
                            let matched = (f64::from(w) / ratio).round();
                            if (1.0..=100_000.0).contains(&matched) {
                                height = Some(matched as u32);
                            }
                        }
                        (None, Some(h)) => {
                            let matched = (f64::from(h) * ratio).round();
                            if (1.0..=100_000.0).contains(&matched) {
                                width = Some(matched as u32);
                            }
                        }
                        _ => {}
                    }
                }
                state.result = Some(ImagePropertyDraft {
                    destination,
                    alternative,
                    width,
                    height,
                });
                state.done = true;
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                return LRESULT(0);
            }
            if matches!(id, ID_PROPERTIES_CANCEL | 2) {
                state.done = true;
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            state.done = true;
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(properties_subclass), 1);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn show_invalid_properties(owner: HWND, strings: Strings) {
    let text = wide(strings.invalid_image_properties());
    let title = wide(strings.image_properties());
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            owner,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::MB_OK
                | windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING,
        );
    }
}

fn dialog_child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    id: u16,
    rect: RECT,
    style: u32,
    tabstop: bool,
) -> Result<HWND, ShellError> {
    let hwnd = child(parent, class, text, id, style, tabstop)?;
    let metrics = WindowMetrics::new(0, 0, unsafe { GetDpiForWindow(parent) });
    unsafe {
        let _ = MoveWindow(
            hwnd,
            metrics.px(rect.left as f32),
            metrics.px(rect.top as f32),
            metrics.px((rect.right - rect.left) as f32),
            metrics.px((rect.bottom - rect.top) as f32),
            true,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    Ok(hwnd)
}

const fn child_rect(x: i32, y: i32, width: i32, height: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

pub(crate) fn edit_image_properties(
    owner: HWND,
    locale: Locale,
    properties: &ImageProperties,
    displayed_destination: &str,
    intrinsic: Option<ImageDimensions>,
) -> Result<Option<ImagePropertyDraft>, ShellError> {
    let strings = locale.strings();
    let dpi = unsafe { GetDpiForWindow(owner) };
    let metrics = WindowMetrics::new(0, 0, dpi);
    let font = Font::for_dpi(dpi, 13.0, false, locale)?;
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let ex_style = windows::Win32::UI::WindowsAndMessaging::WS_EX_DLGMODALFRAME;
    let mut frame = child_rect(0, 0, metrics.px(560.0), metrics.px(300.0));
    unsafe { AdjustWindowRectExForDpi(&mut frame, style, false, ex_style, dpi) }.map_err(error)?;
    let mut owner_rect = RECT::default();
    unsafe { GetWindowRect(owner, &mut owner_rect) }.map_err(error)?;
    let window_w = frame.right - frame.left;
    let window_h = frame.bottom - frame.top;
    let title = wide(strings.image_properties());
    let dialog = unsafe {
        CreateWindowExW(
            ex_style,
            w!("#32770"),
            PCWSTR(title.as_ptr()),
            style,
            owner_rect.left + (owner_rect.right - owner_rect.left - window_w) / 2,
            owner_rect.top + (owner_rect.bottom - owner_rect.top - window_h) / 2,
            window_w,
            window_h,
            owner,
            None,
            None,
            None,
        )
    }
    .map_err(error)?;

    let label_w = 112;
    let field_x = 132;
    let field_w = 392;
    let line_h = 26;
    let mut y = 28;
    dialog_child(
        dialog,
        w!("STATIC"),
        strings.image_address(),
        0,
        child_rect(20, y, label_w, line_h),
        0,
        false,
    )?;
    let destination = dialog_child(
        dialog,
        w!("EDIT"),
        displayed_destination,
        ID_PROPERTIES_DESTINATION,
        child_rect(field_x, y, field_w, line_h),
        WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        true,
    )?;
    y += 42;
    dialog_child(
        dialog,
        w!("STATIC"),
        strings.alternative_text(),
        0,
        child_rect(20, y, label_w, line_h),
        0,
        false,
    )?;
    let alternative = dialog_child(
        dialog,
        w!("EDIT"),
        &properties.alternative,
        ID_PROPERTIES_ALTERNATIVE,
        child_rect(field_x, y, field_w, line_h),
        WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        true,
    )?;
    y += 42;
    dialog_child(
        dialog,
        w!("STATIC"),
        strings.width(),
        0,
        child_rect(20, y, label_w, line_h),
        0,
        false,
    )?;
    let width = dialog_child(
        dialog,
        w!("EDIT"),
        &properties
            .width
            .map_or_else(String::new, |value| value.to_string()),
        ID_PROPERTIES_WIDTH,
        child_rect(field_x, y, 150, line_h),
        WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        true,
    )?;
    y += 42;
    dialog_child(
        dialog,
        w!("STATIC"),
        strings.height(),
        0,
        child_rect(20, y, label_w, line_h),
        0,
        false,
    )?;
    let height = dialog_child(
        dialog,
        w!("EDIT"),
        &properties
            .height
            .map_or_else(String::new, |value| value.to_string()),
        ID_PROPERTIES_HEIGHT,
        child_rect(field_x, y, 150, line_h),
        WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        true,
    )?;
    y += 38;
    let lock = dialog_child(
        dialog,
        w!("BUTTON"),
        strings.lock_aspect_ratio(),
        ID_PROPERTIES_LOCK,
        child_rect(field_x, y, 210, line_h),
        windows::Win32::UI::WindowsAndMessaging::BS_AUTOCHECKBOX as u32,
        true,
    )?;
    unsafe {
        SendMessageW(lock, BM_SETCHECK, WPARAM(BST_CHECKED.0 as usize), LPARAM(0));
    }
    y += 44;
    let cancel = dialog_child(
        dialog,
        w!("BUTTON"),
        strings.cancel(),
        ID_PROPERTIES_CANCEL,
        child_rect(346, y, 84, 30),
        0,
        true,
    )?;
    let apply = dialog_child(
        dialog,
        w!("BUTTON"),
        strings.apply(),
        ID_PROPERTIES_APPLY,
        child_rect(440, y, 84, 30),
        windows::Win32::UI::WindowsAndMessaging::BS_DEFPUSHBUTTON as u32,
        true,
    )?;
    let _ = (cancel, apply);
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::EnumChildWindows(
            dialog,
            Some(set_dialog_font),
            LPARAM(font.0.0 as isize),
        );
    }

    let ratio = match (properties.width, properties.height) {
        (Some(w), Some(h)) if h > 0 => Some(f64::from(w) / f64::from(h)),
        _ => intrinsic.map(|size| f64::from(size.width()) / f64::from(size.height())),
    };
    let mut state = Box::new(PropertiesDialogState {
        strings,
        destination,
        alternative,
        width,
        height,
        lock,
        ratio,
        done: false,
        result: None,
    });
    if !unsafe {
        SetWindowSubclass(
            dialog,
            Some(properties_subclass),
            1,
            std::ptr::from_mut(state.as_mut()) as usize,
        )
    }
    .as_bool()
    {
        unsafe {
            let _ = DestroyWindow(dialog);
        }
        return Err(error(windows::core::Error::from_win32()));
    }
    unsafe {
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(dialog, SW_SHOW);
        let _ = SetFocus(alternative);
    }
    while !state.done {
        let mut message = MSG::default();
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 <= 0 {
            state.done = true;
            break;
        }
        if !unsafe { IsDialogMessageW(dialog, &message) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    unsafe {
        let _ = EnableWindow(owner, true);
        let _ = SetFocus(owner);
    }
    Ok(state.result.clone())
}

unsafe extern "system" fn set_dialog_font(
    hwnd: HWND,
    font: LPARAM,
) -> windows::Win32::Foundation::BOOL {
    unsafe {
        SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    }
    windows::Win32::Foundation::BOOL(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partially_visible_image_borders_are_clipped_to_the_surface() {
        let viewport = child_rect(400, 300, 800, 600);
        assert!(intersect_rect(child_rect(400, 240, 320, 4), viewport).is_none());
        let side = intersect_rect(child_rect(400, 240, 4, 150), viewport)
            .expect("the visible side intersects the viewport");
        assert_eq!((side.top, side.bottom), (300, 390));
        assert!(intersect_rect(child_rect(400, 100, 320, 150), viewport).is_none());
    }

    #[test]
    fn inspector_stays_inside_narrow_and_short_viewports_at_all_dpis() {
        for dpi in [96, 120, 144, 192] {
            for width in [1, 100, 250, 640, 1200] {
                for height in [1, 100, 600] {
                    let viewport = child_rect(400, 120, width, height);
                    let image = child_rect(500, 250, 160, 120);
                    let panel =
                        inspector_panel_rect(image, viewport, WindowMetrics::new(0, 0, dpi));
                    assert!(panel.left >= viewport.left && panel.right <= viewport.right);
                    assert!(panel.top >= viewport.top && panel.bottom <= viewport.bottom);
                    assert!(panel.right > panel.left && panel.bottom > panel.top);
                }
            }
        }
    }

    #[test]
    fn inspector_tracks_image_above_with_a_dpi_scaled_gap() {
        for dpi in [96, 120, 144, 192] {
            let metrics = WindowMetrics::new(0, 0, dpi);
            let viewport = child_rect(100, 100, 1800, 1500);
            let image = child_rect(300, 600, 200, 180);
            let panel = inspector_panel_rect(image, viewport, metrics);
            assert_eq!(panel.left, image.left);
            assert_eq!(panel.bottom, image.top - metrics.px(8.0));
            assert_eq!(panel.right - panel.left, metrics.px(640.0));
            let scrolled = child_rect(image.left, image.top - 80, 200, 180);
            let moved = inspector_panel_rect(scrolled, viewport, metrics);
            assert_eq!(moved.top, panel.top - 80);
            assert_eq!(moved.bottom, panel.bottom - 80);
        }
    }

    #[test]
    fn inspector_avoids_top_and_right_edges() {
        for dpi in [96, 120, 144, 192] {
            let metrics = WindowMetrics::new(0, 0, dpi);
            let viewport = child_rect(100, 100, 1000, 700);
            let image = child_rect(980, 80, 100, 160);
            let panel = inspector_panel_rect(image, viewport, metrics);
            assert_eq!(panel.top, viewport.top + metrics.px(2.0));
            assert_eq!(panel.right, viewport.right - metrics.px(6.0));
            assert!(panel.left >= viewport.left);
            assert!(panel.bottom <= viewport.bottom);
        }
    }

    #[test]
    fn inspector_shadow_fits_viewport() {
        for dpi in [96, 120, 144, 192] {
            let metrics = WindowMetrics::new(0, 0, dpi);
            for width in [100, 250, 640, 1200] {
                let viewport = child_rect(100, 100, metrics.px(width as f32), metrics.px(300.0));
                let image = child_rect(viewport.right - 30, viewport.bottom + 100, 100, 160);
                let panel = inspector_panel_rect(image, viewport, metrics);
                assert!(panel.right + metrics.px(4.0) <= viewport.right);
                assert!(panel.bottom + metrics.px(4.0) <= viewport.bottom);
                assert!(panel.left >= viewport.left && panel.top >= viewport.top);
            }
        }
    }
}
