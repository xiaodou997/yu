//! Native Windows image-selection chrome and the modal image-properties editor.
//!
//! Markdown syntax and mutations stay in yu-editor.  This module only owns
//! transient Win32 controls, mirroring the macOS inspector/property-sheet
//! interaction without introducing a second Markdown parser.
#![cfg(target_os = "windows")]

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_HIGHLIGHT, FillRect, GetSysColorBrush};
use windows::Win32::System::SystemServices::SS_OWNERDRAW;
use windows::Win32::UI::Controls::{BST_CHECKED, DRAWITEMSTRUCT, EM_SETCUEBANNER};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, CreateWindowExW, DestroyWindow, DispatchMessageW, EN_KILLFOCUS,
    ES_AUTOHSCROLL, GetMessageW, GetWindowTextLengthW, GetWindowTextW, HMENU, IsDialogMessageW,
    MSG, MoveWindow, SW_HIDE, SW_SHOW, SendMessageW, SetWindowPos, SetWindowTextW, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_NCDESTROY, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS, WS_DISABLED, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
};
use windows::core::{PCWSTR, w};
use yu_assets::ImageDimensions;
use yu_core::{Revision, TextRange};
use yu_editor::ImageProperties;
use yu_scene::Rect;

use crate::{ShellError, Strings, WindowMetrics};

pub(crate) const ID_IMAGE_ALT: u16 = 2201;
pub(crate) const ID_IMAGE_DESTINATION: u16 = 2202;
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
        ID_IMAGE_ALT | ID_IMAGE_DESTINATION | ID_IMAGE_REPLACE | ID_IMAGE_SIZE | ID_IMAGE_MORE
    )
}

pub(crate) struct ImageInspector {
    borders: [HWND; 4],
    panel: HWND,
    source: HWND,
    alternative: HWND,
    destination: HWND,
    replace: HWND,
    size: HWND,
    more: HWND,
    identity: Option<(Revision, TextRange)>,
    visible: bool,
}

impl ImageInspector {
    pub(crate) fn new(parent: HWND, strings: Strings) -> Result<Self, ShellError> {
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
        let panel = child(parent, w!("STATIC"), "", 2270, WS_BORDER.0, false)?;
        let source = child(parent, w!("STATIC"), "", 2271, 0, false)?;
        let alternative = child(
            parent,
            w!("EDIT"),
            "",
            ID_IMAGE_ALT,
            WS_BORDER.0 | ES_AUTOHSCROLL as u32,
            true,
        )?;
        let destination = child(
            parent,
            w!("EDIT"),
            "",
            ID_IMAGE_DESTINATION,
            WS_BORDER.0 | ES_AUTOHSCROLL as u32,
            true,
        )?;
        let replace = child(parent, w!("BUTTON"), "…", ID_IMAGE_REPLACE, 0, true)?;
        let size = child(
            parent,
            w!("BUTTON"),
            strings.image_size(),
            ID_IMAGE_SIZE,
            0,
            true,
        )?;
        let more = child(parent, w!("BUTTON"), "⋯", ID_IMAGE_MORE, 0, true)?;
        let alt_cue = wide(strings.alternative_text());
        let destination_cue = wide(strings.image_address());
        unsafe {
            SendMessageW(
                alternative,
                EM_SETCUEBANNER,
                WPARAM(1),
                LPARAM(alt_cue.as_ptr() as isize),
            );
            SendMessageW(
                destination,
                EM_SETCUEBANNER,
                WPARAM(1),
                LPARAM(destination_cue.as_ptr() as isize),
            );
        }
        let result = Self {
            borders: [
                border(ID_IMAGE_BORDER_TOP)?,
                border(ID_IMAGE_BORDER_BOTTOM)?,
                border(ID_IMAGE_BORDER_LEFT)?,
                border(ID_IMAGE_BORDER_RIGHT)?,
            ],
            panel,
            source,
            alternative,
            destination,
            replace,
            size,
            more,
            identity: None,
            visible: false,
        };
        result.hide_windows();
        Ok(result)
    }

    pub(crate) fn alternative(&self) -> String {
        window_text(self.alternative)
    }

    pub(crate) fn destination(&self) -> String {
        window_text(self.destination)
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
        self.visible = false;
        self.hide_windows();
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
                self.alternative,
                self.destination,
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
        metrics: WindowMetrics,
    ) {
        let same = self.identity == Some((state.revision, state.source));
        self.identity = Some((state.revision, state.source));
        if !same {
            set_text(self.alternative, &state.properties.alternative);
            set_text(self.destination, &state.displayed_destination);
        }
        set_text(self.source, &state.source_text);

        let thickness = metrics.px(2.0).max(2);
        let image_width = (image.right - image.left).max(1);
        let image_height = (image.bottom - image.top).max(1);
        unsafe {
            let _ = MoveWindow(
                self.borders[0],
                image.left - thickness,
                image.top - thickness,
                image_width + thickness * 2,
                thickness,
                true,
            );
            let _ = MoveWindow(
                self.borders[1],
                image.left - thickness,
                image.bottom,
                image_width + thickness * 2,
                thickness,
                true,
            );
            let _ = MoveWindow(
                self.borders[2],
                image.left - thickness,
                image.top,
                thickness,
                image_height,
                true,
            );
            let _ = MoveWindow(
                self.borders[3],
                image.right,
                image.top,
                thickness,
                image_height,
                true,
            );
        }

        let padding = metrics.px(8.0).max(6);
        let gap = metrics.px(6.0).max(4);
        let row_h = metrics.px(28.0).max(24);
        let source_h = metrics.px(20.0).max(18);
        let panel_h = padding * 2 + source_h + gap + row_h;
        let available = (viewport.right - viewport.left - metrics.px(24.0)).max(metrics.px(320.0));
        let panel_w = metrics.px(720.0).min(available);
        let mut panel_x = image.left + image_width / 2 - panel_w / 2;
        panel_x = panel_x.clamp(viewport.left + padding, viewport.right - panel_w - padding);
        let mut panel_y = image.bottom + gap;
        if panel_y + panel_h > viewport.bottom - padding {
            panel_y = image.top - panel_h - gap;
        }
        panel_y = panel_y.clamp(viewport.top + padding, viewport.bottom - panel_h - padding);

        let button_w = metrics.px(34.0).max(30);
        let size_w = metrics.px(94.0).max(84);
        let content_w = panel_w - padding * 2;
        let fixed = button_w * 2 + size_w + gap * 4;
        let fields = (content_w - fixed).max(metrics.px(200.0));
        let alt_w = (fields * 2 / 5).max(metrics.px(110.0));
        let destination_w = (fields - alt_w).max(metrics.px(130.0));
        let row_y = panel_y + padding + source_h + gap;
        let mut x = panel_x + padding;

        unsafe {
            let _ = MoveWindow(self.panel, panel_x, panel_y, panel_w, panel_h, true);
            let _ = MoveWindow(
                self.source,
                panel_x + padding,
                panel_y + padding,
                content_w,
                source_h,
                true,
            );
            let _ = MoveWindow(self.alternative, x, row_y, alt_w, row_h, true);
            x += alt_w + gap;
            let _ = MoveWindow(self.destination, x, row_y, destination_w, row_h, true);
            x += destination_w + gap;
            let _ = MoveWindow(self.replace, x, row_y, button_w, row_h, true);
            x += button_w + gap;
            let _ = MoveWindow(self.size, x, row_y, size_w, row_h, true);
            x += size_w + gap;
            let _ = MoveWindow(self.more, x, row_y, button_w, row_h, true);

            for hwnd in [
                self.panel,
                self.source,
                self.alternative,
                self.destination,
                self.replace,
                self.size,
                self.more,
            ] {
                let _ = SetWindowPos(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                );
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            for hwnd in self.borders {
                let _ = SetWindowPos(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                );
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
        self.visible = true;
    }

    pub(crate) fn draw(&self, item: &DRAWITEMSTRUCT) -> bool {
        if !self.borders.contains(&item.hwndItem) {
            return false;
        }
        unsafe {
            let _ = FillRect(item.hDC, &item.rcItem, GetSysColorBrush(COLOR_HIGHLIGHT));
        }
        true
    }
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
            if id == ID_PROPERTIES_APPLY {
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
            if id == ID_PROPERTIES_CANCEL {
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
    unsafe {
        let _ = MoveWindow(
            hwnd,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
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
    strings: Strings,
    properties: &ImageProperties,
    displayed_destination: &str,
    intrinsic: Option<ImageDimensions>,
) -> Result<Option<ImagePropertyDraft>, ShellError> {
    let title = wide(strings.image_properties());
    let dialog = unsafe {
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WS_EX_DLGMODALFRAME,
            w!("STATIC"),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
            windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
            560,
            330,
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
