#![cfg(target_os = "windows")]

//! Native navigation chrome. Markdown labels and search ranges come from the
//! shared editor; GDI here paints only controls, never the document surface.
use crate::{Locale, ShellError, ShellState, SidebarMode, WindowMetrics};
use std::mem::size_of;
use std::path::PathBuf;
use windows::Win32::Foundation::{COLORREF, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateRoundRectRgn, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS,
    DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteObject, DrawFocusRect,
    DrawTextW, FillRect, FillRgn, FrameRgn, HBRUSH, HDC, HFONT, InvalidateRect, RestoreDC, SaveDC,
    ScreenToClient, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::SystemServices::SS_OWNERDRAW;
use windows::Win32::UI::Accessibility::{CAccPropServices, IAccPropServices, Name_Property_GUID};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_SETCUEBANNER, ODS_FOCUS, ODS_SELECTED, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CreateWindowExW, DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetCursorPos,
    GetWindowTextLengthW, GetWindowTextW, HMENU, HWND_BOTTOM, HWND_TOP, LB_ADDSTRING, LB_GETCURSEL,
    LB_GETTOPINDEX, LB_RESETCONTENT, LB_SETCURSEL, LB_SETITEMHEIGHT, LB_SETTOPINDEX,
    LBS_HASSTRINGS, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LBS_OWNERDRAWFIXED, MoveWindow,
    NONCLIENTMETRICSW, SBS_VERT, SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SendMessageW, SetWindowPos, SetWindowTextW, ShowWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_SETFONT,
    WM_SETREDRAW, WS_CHILD, WS_CLIPSIBLINGS, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{PCWSTR, w};
use yu_core::{Revision, TextRange};
use yu_editor::OutlineTree;
use yu_workspace::{Appearance, ViewportCodeBlockControl};

pub(crate) const ID_FILES: u16 = 2001;
pub(crate) const ID_OUTLINE: u16 = 2002;
pub(crate) const ID_SEARCH: u16 = 2003;
pub(crate) const ID_QUERY: u16 = 2004;
pub(crate) const ID_ROWS: u16 = 2005;
pub(crate) const ID_SEARCH_PREVIOUS: u16 = 2006;
pub(crate) const ID_SEARCH_CLOSE: u16 = 2007;
pub(crate) const ID_SEARCH_NEXT: u16 = 2008;
pub(crate) const ID_REPLACE_TOGGLE: u16 = 2201;
pub(crate) const ID_REPLACEMENT: u16 = 2202;
pub(crate) const ID_REPLACE_CURRENT: u16 = 2203;
pub(crate) const ID_REPLACE_ALL: u16 = 2204;
pub(crate) const ID_FIND_REPLACE: u16 = 2205;
pub(crate) const ID_DOCUMENT_SCROLLBAR: u16 = 2009;
pub(crate) const ID_MENU_FILE: u16 = 2101;
pub(crate) const ID_MENU_EDIT: u16 = 2102;
pub(crate) const ID_MENU_VIEW: u16 = 2103;
pub(crate) const ID_MENU_HELP: u16 = 2104;
pub(crate) const ID_CODE_COPY_BASE: u16 = 18_000;
pub(crate) const MAX_CODE_BLOCK_CONTROLS: usize = 64;
pub(crate) const CODE_COPY_FEEDBACK_TIMER_ID: usize = 3;
pub(crate) const CODE_COPY_FEEDBACK_MS: u32 = 1_200;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn error(error: impl std::fmt::Display) -> ShellError {
    ShellError::Platform(error.to_string())
}

pub(crate) struct Font(pub(crate) HFONT);
impl Font {
    pub(crate) fn for_dpi(
        dpi: u32,
        size: f32,
        bold: bool,
        locale: Locale,
    ) -> Result<Self, ShellError> {
        Self::for_family(dpi, size, bold, ui_font_family(locale))
    }

    pub(crate) fn monospace(dpi: u32, size: f32) -> Result<Self, ShellError> {
        Self::for_family(dpi, size, false, "Consolas")
    }

    fn for_family(dpi: u32, size: f32, bold: bool, family: &str) -> Result<Self, ShellError> {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some(std::ptr::from_mut(&mut metrics).cast()),
                0,
                dpi,
            )
        }
        .map_err(error)?;
        let mut font = metrics.lfMessageFont;
        // Keep the user's text-size scale while giving chrome a clear hierarchy.
        let scale = (font.lfHeight.abs() as f32 / (12.0 * dpi as f32 / 96.0)).max(1.0);
        font.lfHeight = -(size * dpi as f32 / 96.0 * scale).round() as i32;
        font.lfWeight = if bold { 600 } else { 400 };
        font.lfFaceName.fill(0);
        for (target, unit) in font.lfFaceName.iter_mut().zip(family.encode_utf16()) {
            *target = unit;
        }
        let handle = unsafe { CreateFontIndirectW(&font) };
        if handle.is_invalid() {
            return Err(error(windows::core::Error::from_win32()));
        }
        Ok(Self(handle))
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0);
        }
    }
}
pub(crate) struct Brush(pub(crate) HBRUSH);
impl Brush {
    pub(crate) fn new(color: COLORREF) -> Self {
        Self(unsafe { CreateSolidBrush(color) })
    }
}
impl Drop for Brush {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PanelAction {
    Directory(PathBuf),
    File(PathBuf),
    Select(TextRange),
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    label: String,
    identity: String,
    indent: u8,
    action: PanelAction,
}

struct Palette {
    selected_text: COLORREF,
    background: Brush,
    input: Brush,
    selected: Brush,
    hover: Brush,
    border: Brush,
    canvas: Brush,
    track: Brush,
    nav_selected: Brush,
    text: COLORREF,
    muted: COLORREF,
    accent: COLORREF,
    input_color: COLORREF,
    background_color: COLORREF,
}
fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}
impl Palette {
    fn contrast(appearance: Appearance, colors: yu_scene::ContrastPalette) -> Self {
        let mut palette = Self::new(appearance);
        let background = crate::contrast::colorref(colors.background);
        let foreground = crate::contrast::colorref(colors.foreground);
        let selected = crate::contrast::colorref(colors.selection);
        let selected_text = crate::contrast::colorref(colors.selected_text);
        palette.canvas = Brush::new(background);
        palette.background = Brush::new(background);
        palette.input = Brush::new(background);
        palette.track = Brush::new(background);
        palette.hover = Brush::new(background);
        palette.border = Brush::new(foreground);
        palette.selected = Brush::new(selected);
        palette.nav_selected = Brush::new(selected);
        palette.text = foreground;
        palette.muted = foreground;
        palette.accent = selected_text;
        palette.selected_text = selected_text;
        palette.background_color = background;
        palette.input_color = background;
        palette
    }
    fn new(appearance: Appearance) -> Self {
        let dark = matches!(appearance, Appearance::Dark | Appearance::YuDark);
        let background = if dark {
            rgb(35, 37, 41)
        } else {
            rgb(247, 248, 250)
        };
        let input_color = if dark {
            rgb(43, 46, 51)
        } else {
            rgb(255, 255, 255)
        };
        Self {
            selected_text: if dark {
                rgb(232, 234, 238)
            } else {
                rgb(44, 49, 58)
            },
            canvas: Brush::new(rgb(
                appearance.background().red(),
                appearance.background().green(),
                appearance.background().blue(),
            )),
            track: Brush::new(if dark {
                rgb(43, 45, 49)
            } else {
                rgb(235, 237, 241)
            }),
            nav_selected: Brush::new(if dark {
                rgb(66, 69, 75)
            } else {
                rgb(255, 255, 255)
            }),
            background: Brush::new(background),
            background_color: background,
            input: Brush::new(input_color),
            input_color,
            selected: Brush::new(if dark {
                rgb(49, 67, 89)
            } else {
                rgb(226, 236, 250)
            }),
            hover: Brush::new(if dark {
                rgb(48, 51, 57)
            } else {
                rgb(235, 238, 243)
            }),
            border: Brush::new(if dark {
                rgb(62, 65, 70)
            } else {
                rgb(222, 226, 232)
            }),
            text: if dark {
                rgb(232, 234, 238)
            } else {
                rgb(44, 49, 58)
            },
            muted: if dark {
                rgb(169, 175, 185)
            } else {
                rgb(104, 113, 127)
            },
            accent: if dark {
                rgb(144, 188, 248)
            } else {
                rgb(41, 100, 185)
            },
        }
    }
}

struct CodeBlockRow {
    label: HWND,
    button: HWND,
    command: u16,
    revision: Revision,
    code: String,
    copy_label: String,
    copied_label: String,
    copied: bool,
}

pub(crate) struct CodeBlockControls {
    surface: HWND,
    rows: Vec<CodeBlockRow>,
    dpi: u32,
    language_font: Option<Font>,
    button_font: Option<Font>,
    background: Option<Brush>,
    background_color: COLORREF,
    foreground_color: COLORREF,
}

impl CodeBlockControls {
    pub(crate) fn new(surface: HWND) -> Self {
        Self {
            surface,
            rows: Vec::new(),
            dpi: 0,
            language_font: None,
            button_font: None,
            background: None,
            background_color: COLORREF(u32::MAX),
            foreground_color: COLORREF(u32::MAX),
        }
    }

    pub(crate) fn is_copy_command(command: u16) -> bool {
        let end = ID_CODE_COPY_BASE.saturating_add(MAX_CODE_BLOCK_CONTROLS as u16);
        (ID_CODE_COPY_BASE..end).contains(&command)
    }

    pub(crate) fn is_language_label(&self, hwnd: HWND) -> bool {
        self.rows.iter().any(|row| row.label == hwnd)
    }

    pub(crate) fn command_for_button(&self, hwnd: HWND) -> Option<u16> {
        self.rows
            .iter()
            .find(|row| row.button == hwnd)
            .map(|row| row.command)
    }

    pub(crate) fn language_palette(&self) -> Option<(COLORREF, COLORREF, HBRUSH)> {
        self.background
            .as_ref()
            .map(|brush| (self.foreground_color, self.background_color, brush.0))
    }

    pub(crate) fn sync(
        &mut self,
        revision: Revision,
        scroll_y: f32,
        scale: f32,
        dpi: u32,
        locale: Locale,
        controls: &[ViewportCodeBlockControl],
        source: &str,
        copy_label: &str,
        copied_label: &str,
        background: COLORREF,
        foreground: COLORREF,
    ) -> Result<(), ShellError> {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        self.ensure_fonts(dpi.max(96), locale)?;
        self.ensure_palette(background, foreground);
        let visible = controls.len().min(MAX_CODE_BLOCK_CONTROLS);
        while self.rows.len() < visible {
            let index = self.rows.len();
            let command = ID_CODE_COPY_BASE + index as u16;
            let label = child(self.surface, w!("STATIC"), "", 0, command + 100, false)?;
            let button = child(self.surface, w!("BUTTON"), copy_label, 0, command, true)?;
            if let Some(font) = self.language_font.as_ref() {
                unsafe {
                    SendMessageW(label, WM_SETFONT, WPARAM(font.0.0 as usize), LPARAM(1));
                }
            }
            if let Some(font) = self.button_font.as_ref() {
                unsafe {
                    SendMessageW(button, WM_SETFONT, WPARAM(font.0.0 as usize), LPARAM(1));
                }
            }
            self.rows.push(CodeBlockRow {
                label,
                button,
                command,
                revision,
                code: String::new(),
                copy_label: copy_label.to_owned(),
                copied_label: copied_label.to_owned(),
                copied: false,
            });
        }

        let mut client = RECT::default();
        unsafe { GetClientRect(self.surface, &mut client) }.map_err(error)?;
        let client_width = (client.right - client.left).max(1);
        let client_height = (client.bottom - client.top).max(1);

        for (index, control) in controls.iter().take(visible).enumerate() {
            let info = source_text(source, control.info())
                .unwrap_or_default()
                .trim();
            let language = if info.is_empty() { "text" } else { info };
            let code = source_text(source, control.content()).ok_or_else(|| {
                ShellError::Platform("invalid fenced-code source range".to_owned())
            })?;
            let row = &mut self.rows[index];
            let changed = row.revision != revision
                || row.code != code
                || row.copy_label != copy_label
                || row.copied_label != copied_label;
            row.revision = revision;
            row.code.clear();
            row.code.push_str(code);
            row.copy_label.clear();
            row.copy_label.push_str(copy_label);
            row.copied_label.clear();
            row.copied_label.push_str(copied_label);
            if changed {
                row.copied = false;
            }

            set_control_text(row.label, language);
            set_control_text(
                row.button,
                if row.copied { copied_label } else { copy_label },
            );

            let bounds = control.bounds();
            let copy = control.copy_bounds();
            let button_x = logical_px(copy.x(), scale);
            let button_y = logical_px(copy.y() - scroll_y, scale);
            let button_w = logical_px(copy.width(), scale).max(1);
            let button_h = logical_px(copy.height(), scale).max(1);
            let label_x = logical_px(bounds.x() + 12.0, scale);
            let label_y = button_y;
            let label_w = logical_px((copy.x() - bounds.x() - 20.0).max(1.0), scale).max(1);
            let label_h = button_h;
            // A long code block can remain visible after its toolbar has
            // scrolled above the viewport. Keep those HWNDs hidden so
            // IsDialogMessageW does not include an invisible WS_TABSTOP in
            // keyboard navigation.
            let on_screen = button_y + button_h > 0
                && button_y < client_height
                && button_x < client_width
                && button_x + button_w > 0;

            unsafe {
                let _ = MoveWindow(row.label, label_x, label_y, label_w, label_h, true);
                let _ = MoveWindow(row.button, button_x, button_y, button_w, button_h, true);
                let visibility = if on_screen { SW_SHOW } else { SW_HIDE };
                if !on_screen && GetFocus() == row.button {
                    let _ = SetFocus(self.surface);
                }
                let _ = ShowWindow(row.label, visibility);
                let _ = ShowWindow(row.button, visibility);
            }
        }
        for row in self.rows.iter().skip(visible) {
            unsafe {
                let _ = ShowWindow(row.label, SW_HIDE);
                let _ = ShowWindow(row.button, SW_HIDE);
            }
        }
        Ok(())
    }

    fn ensure_fonts(&mut self, dpi: u32, locale: Locale) -> Result<(), ShellError> {
        if self.dpi == dpi {
            return Ok(());
        }
        let language_font = Font::monospace(dpi, 11.0)?;
        let button_font = Font::for_dpi(dpi, 11.0, false, locale)?;
        for row in &self.rows {
            unsafe {
                SendMessageW(
                    row.label,
                    WM_SETFONT,
                    WPARAM(language_font.0.0 as usize),
                    LPARAM(1),
                );
                SendMessageW(
                    row.button,
                    WM_SETFONT,
                    WPARAM(button_font.0.0 as usize),
                    LPARAM(1),
                );
            }
        }
        self.language_font = Some(language_font);
        self.button_font = Some(button_font);
        self.dpi = dpi;
        Ok(())
    }

    fn ensure_palette(&mut self, background: COLORREF, foreground: COLORREF) {
        if self.background_color.0 == background.0 && self.foreground_color.0 == foreground.0 {
            return;
        }
        self.background = Some(Brush::new(background));
        self.background_color = background;
        self.foreground_color = foreground;
        for row in &self.rows {
            unsafe {
                let _ = InvalidateRect(row.label, None, true);
            }
        }
    }

    pub(crate) fn code_for_command(&self, command: u16, revision: Revision) -> Option<String> {
        let index = usize::from(command.checked_sub(ID_CODE_COPY_BASE)?);
        let row = self.rows.get(index)?;
        (row.command == command && row.revision == revision).then(|| row.code.clone())
    }

    pub(crate) fn mark_copied(&mut self, command: u16) {
        let Some(index) = command.checked_sub(ID_CODE_COPY_BASE).map(usize::from) else {
            return;
        };
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        row.copied = true;
        set_control_text(row.button, &row.copied_label);
    }

    pub(crate) fn reset_feedback(&mut self) {
        for row in &mut self.rows {
            if row.copied {
                row.copied = false;
                set_control_text(row.button, &row.copy_label);
            }
        }
    }
}

impl Drop for CodeBlockControls {
    fn drop(&mut self) {
        for row in self.rows.drain(..) {
            unsafe {
                let _ = DestroyWindow(row.label);
                let _ = DestroyWindow(row.button);
            }
        }
    }
}

fn source_text(source: &str, range: TextRange) -> Option<&str> {
    let start = usize::try_from(range.start().get()).ok()?;
    let end = usize::try_from(range.end().get()).ok()?;
    source.get(start..end)
}

fn logical_px(value: f32, scale: f32) -> i32 {
    (value * scale)
        .round()
        .clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

fn set_control_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

pub(crate) struct Chrome {
    accessibility: IAccPropServices,
    pub(super) canvas: HWND,
    pub(crate) background: HWND,
    pub(crate) status: HWND,
    pub(crate) document_scrollbar: HWND,
    pub(crate) query: HWND,
    pub(crate) replacement: HWND,
    replace_toggle: HWND,
    replace_current: HWND,
    replace_all: HWND,
    replacement_frame: HWND,
    pub(crate) list: HWND,
    pub(crate) menu_buttons: [HWND; 4],
    menu_background: HWND,
    search_previous: HWND,
    search_next: HWND,
    search_background: HWND,
    search_caption: HWND,
    search_close: HWND,
    tabs: [HWND; 2],
    caption: HWND,
    empty: HWND,
    query_frame: HWND,
    font: Option<Font>,
    secondary_font: Option<Font>,
    tab_font: Option<Font>,
    locale: Locale,
    dpi: u32,
    palette: Palette,
    contrast: Option<yu_scene::ContrastPalette>,
    appearance: Appearance,
    mode: SidebarMode,
    rows: Vec<Row>,
    search_cache: Option<(PathBuf, Revision, String, bool)>,
    search_caption_text: String,
    cache: Option<(PathBuf, Revision, SidebarMode)>,
    empty_text: String,
    caption_text: String,
    tab_text: [String; 2],
    menu_text: [String; 4],
}

impl Chrome {
    pub(crate) fn set_contrast_palette(&mut self, contrast: Option<yu_scene::ContrastPalette>) {
        if self.contrast == contrast {
            return;
        }
        self.contrast = contrast;
        self.palette = contrast.map_or_else(
            || Palette::new(self.appearance),
            |c| Palette::contrast(self.appearance, c),
        );
        for hwnd in self.controls() {
            unsafe {
                let _ = InvalidateRect(hwnd, None, true);
            }
        }
    }

    pub(crate) fn focus_targets(&self) -> Vec<HWND> {
        self.menu_buttons
            .iter()
            .copied()
            .chain(self.tabs)
            .chain([
                self.list,
                self.query,
                self.replace_toggle,
                self.replacement,
                self.replace_current,
                self.replace_all,
                self.search_previous,
                self.search_next,
                self.search_close,
            ])
            .filter(|hwnd| unsafe {
                windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(*hwnd).as_bool()
                    && windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(*hwnd).as_bool()
            })
            .collect()
    }
    pub(crate) fn new(parent: HWND, state: &ShellState) -> Result<Self, ShellError> {
        let strings = state.strings();
        let canvas = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2014, false)?;
        let background = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2010, false)?;
        let menu_text = [
            strings.file(),
            strings.edit(),
            strings.view(),
            strings.help(),
        ]
        .map(menu_label);
        let menu_background = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2019, false)?;
        let menu_buttons = [
            child(
                parent,
                w!("BUTTON"),
                &menu_text[0],
                BS_OWNERDRAW as u32,
                ID_MENU_FILE,
                true,
            )?,
            child(
                parent,
                w!("BUTTON"),
                &menu_text[1],
                BS_OWNERDRAW as u32,
                ID_MENU_EDIT,
                true,
            )?,
            child(
                parent,
                w!("BUTTON"),
                &menu_text[2],
                BS_OWNERDRAW as u32,
                ID_MENU_VIEW,
                true,
            )?,
            child(
                parent,
                w!("BUTTON"),
                &menu_text[3],
                BS_OWNERDRAW as u32,
                ID_MENU_HELP,
                true,
            )?,
        ];
        let tabs = [
            child(
                parent,
                w!("BUTTON"),
                strings.files(),
                BS_OWNERDRAW as u32,
                ID_FILES,
                true,
            )?,
            child(
                parent,
                w!("BUTTON"),
                strings.outline(),
                BS_OWNERDRAW as u32,
                ID_OUTLINE,
                true,
            )?,
        ];
        for tab in tabs.into_iter().chain(menu_buttons) {
            if !unsafe { SetWindowSubclass(tab, Some(tab_subclass), 1, 0) }.as_bool() {
                return Err(error(windows::core::Error::from_win32()));
            }
        }
        let caption = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2011, false)?;
        let query_frame = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2015, false)?;
        let query = child(
            parent,
            w!("EDIT"),
            "",
            ES_AUTOHSCROLL as u32,
            ID_QUERY,
            true,
        )?;
        if !unsafe { SetWindowSubclass(query, Some(query_subclass), 1, 0) }.as_bool() {
            return Err(error(windows::core::Error::from_win32()));
        }
        let replacement_frame = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2210, false)?;
        let replacement = child(
            parent,
            w!("EDIT"),
            "",
            ES_AUTOHSCROLL as u32,
            ID_REPLACEMENT,
            true,
        )?;
        if !unsafe { SetWindowSubclass(replacement, Some(query_subclass), 1, 0) }.as_bool() {
            return Err(error(windows::core::Error::from_win32()));
        }
        let replace_toggle = child(
            parent,
            w!("BUTTON"),
            strings.replace(),
            BS_OWNERDRAW as u32,
            ID_REPLACE_TOGGLE,
            true,
        )?;
        let replace_current = child(
            parent,
            w!("BUTTON"),
            strings.replace(),
            BS_OWNERDRAW as u32,
            ID_REPLACE_CURRENT,
            true,
        )?;
        let replace_all = child(
            parent,
            w!("BUTTON"),
            strings.replace_all(),
            BS_OWNERDRAW as u32,
            ID_REPLACE_ALL,
            true,
        )?;
        for button in [replace_toggle, replace_current, replace_all] {
            if !unsafe { SetWindowSubclass(button, Some(tab_subclass), 1, 0) }.as_bool() {
                return Err(error(windows::core::Error::from_win32()));
            }
        }
        let replacement_cue = wide(strings.replace_with());
        unsafe {
            SendMessageW(
                replacement,
                EM_SETCUEBANNER,
                WPARAM(1),
                LPARAM(replacement_cue.as_ptr() as isize),
            );
        }
        let list = child(
            parent,
            w!("LISTBOX"),
            "",
            (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32
                | WS_VSCROLL.0,
            ID_ROWS,
            true,
        )?;
        let empty = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2012, false)?;
        let status = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2013, false)?;
        if !unsafe { SetWindowSubclass(status, Some(overlay_subclass), 1, 0) }.as_bool() {
            return Err(error(windows::core::Error::from_win32()));
        }
        let document_scrollbar = child(
            parent,
            w!("SCROLLBAR"),
            "",
            SBS_VERT as u32,
            ID_DOCUMENT_SCROLLBAR,
            false,
        )?;
        unsafe {
            let _ = ShowWindow(document_scrollbar, SW_HIDE);
        }
        let search_background = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2016, false)?;
        let search_caption = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2017, false)?;
        let search_close = child(
            parent,
            w!("BUTTON"),
            search_done(state.locale()),
            BS_OWNERDRAW as u32,
            ID_SEARCH_CLOSE,
            true,
        )?;
        let search_previous = child(
            parent,
            w!("BUTTON"),
            search_step(state.locale(), false),
            BS_OWNERDRAW as u32,
            ID_SEARCH_PREVIOUS,
            true,
        )?;
        let search_next = child(
            parent,
            w!("BUTTON"),
            search_step(state.locale(), true),
            BS_OWNERDRAW as u32,
            ID_SEARCH_NEXT,
            true,
        )?;
        for button in [search_previous, search_next, search_close] {
            if !unsafe { SetWindowSubclass(button, Some(tab_subclass), 1, 0) }.as_bool() {
                return Err(error(windows::core::Error::from_win32()));
            }
        }
        unsafe {
            SetWindowPos(
                search_background,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(error)?;
        }
        let cue = wide(strings.search());
        unsafe {
            // This background is a sibling, so it must sit beneath the controls.
            SetWindowPos(
                background,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(error)?;
            SetWindowPos(
                canvas,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(error)?;
            SetWindowPos(
                menu_background,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(error)?;
            SetWindowPos(
                canvas,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(error)?;
            SetWindowPos(
                query,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(error)?;
            SendMessageW(
                query,
                EM_SETCUEBANNER,
                WPARAM(1),
                LPARAM(cue.as_ptr() as isize),
            );
        }
        let accessibility: IAccPropServices =
            unsafe { CoCreateInstance(&CAccPropServices, None, CLSCTX_INPROC_SERVER) }
                .map_err(error)?;
        let label = wide(strings.search());
        unsafe {
            accessibility.SetHwndPropStr(
                query,
                (-4i32) as u32,
                0,
                Name_Property_GUID,
                PCWSTR(label.as_ptr()),
            )
        }
        .map_err(error)?;
        let replacement_label = wide(strings.replace_with());
        unsafe {
            accessibility.SetHwndPropStr(
                replacement,
                (-4i32) as u32,
                0,
                Name_Property_GUID,
                PCWSTR(replacement_label.as_ptr()),
            )
        }
        .map_err(error)?;
        Ok(Self {
            accessibility,
            canvas,
            background,
            status,
            document_scrollbar,
            query,
            replacement,
            replacement_frame,
            replace_toggle,
            replace_current,
            replace_all,
            list,
            menu_buttons,
            menu_background,
            search_previous,
            search_next,
            search_background,
            search_caption,
            search_close,
            tabs,
            caption,
            empty,
            query_frame,
            font: None,
            secondary_font: None,
            tab_font: None,
            locale: state.locale(),
            dpi: 0,
            palette: Palette::new(state.appearance()),
            contrast: None,
            appearance: state.appearance(),
            mode: state.sidebar(),
            rows: Vec::new(),
            search_cache: None,
            search_caption_text: String::new(),
            cache: None,
            empty_text: String::new(),
            caption_text: String::new(),
            tab_text: [strings.files().into(), strings.outline().into()],
            menu_text,
        })
    }

    pub(crate) fn invalidate_content(&mut self) {
        self.cache = None;
        self.search_cache = None;
    }
    pub(crate) fn invalidate_font(&mut self) {
        self.dpi = 0;
    }
    pub(crate) fn query_text(&self) -> String {
        window_text(self.query)
    }
    pub(crate) fn replacement_text(&self) -> String {
        window_text(self.replacement)
    }
    pub(crate) fn replacement_is_composing(&self) -> bool {
        !unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetPropW(
                self.replacement,
                w!("YuSearchComposition"),
            )
        }
        .is_invalid()
    }
    pub(crate) fn query_is_composing(&self) -> bool {
        !unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetPropW(self.query, w!("YuSearchComposition"))
        }
        .is_invalid()
    }
    pub(crate) fn selected_action(&self) -> Option<PanelAction> {
        let index = unsafe { SendMessageW(self.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)) }.0;
        usize::try_from(index)
            .ok()
            .and_then(|index| self.rows.get(index))
            .map(|row| row.action.clone())
    }

    pub(crate) fn refresh(&mut self, state: &mut ShellState) -> Result<(), ShellError> {
        self.refresh_search(state)?;
        let mode = state.sidebar();
        if mode == SidebarMode::Hidden {
            self.mode = mode;
            return Ok(());
        }
        let path = state.document().session().path().to_owned();
        let revision = state.document().session().revision();
        let key = (path.clone(), revision, mode);
        let file_directory = state.file_directory();
        let workspace_root = state.workspace_root();
        // File lists do not depend on source edits; refresh on navigation or path changes.
        if self.cache.as_ref().is_some_and(|old| {
            old == &key || (mode == SidebarMode::Files && old.0 == path && old.2 == mode)
        }) {
            return Ok(());
        }
        self.mode = mode;
        let locale = state.locale();
        let strings = state.strings();
        let document_name = state.document().display_name(strings).to_owned();
        let editor = state
            .document_mut()
            .session_mut()
            .document_mut()
            .editor_mut();
        let (rows, caption, empty_text) = match mode {
            SidebarMode::Files => {
                let rows = if state.document().is_untitled() && !state.has_workspace() {
                    Vec::new()
                } else {
                    directory_rows(&file_directory, &workspace_root)?
                };
                let folder = file_directory
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or(document_name);
                (rows, folder, panel_hint(locale, 0).into())
            }
            SidebarMode::Outline => {
                let tree = OutlineTree::build(editor).map_err(error)?;
                let rows = tree
                    .rows()
                    .iter()
                    .map(|row| Row {
                        label: row.label().to_owned(),
                        identity: row.identity().to_owned(),
                        indent: row.item().level().saturating_sub(1),
                        action: PanelAction::Select(TextRange::empty(
                            row.item().label_range().start(),
                        )),
                    })
                    .collect();
                (rows, document_name, panel_hint(locale, 1).into())
            }
            SidebarMode::Hidden => unreachable!(),
        };
        let old_index = unsafe { SendMessageW(self.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)) }.0;
        let old_identity = usize::try_from(old_index)
            .ok()
            .and_then(|index| self.rows.get(index))
            .map(|row| row.identity.clone());
        let top = unsafe { SendMessageW(self.list, LB_GETTOPINDEX, WPARAM(0), LPARAM(0)) };
        self.rows = rows;
        self.caption_text = caption;
        self.empty_text = empty_text;
        for (hwnd, label) in [
            (
                self.list,
                match mode {
                    SidebarMode::Files => strings.files(),
                    SidebarMode::Outline => strings.outline(),
                    SidebarMode::Hidden => unreachable!(),
                },
            ),
            (self.caption, self.caption_text.as_str()),
            (self.empty, self.empty_text.as_str()),
        ] {
            let label = wide(label);
            unsafe {
                self.accessibility.SetHwndPropStr(
                    hwnd,
                    (-4i32) as u32,
                    0,
                    Name_Property_GUID,
                    PCWSTR(label.as_ptr()),
                )
            }
            .map_err(error)?;
        }
        unsafe {
            SendMessageW(self.list, WM_SETREDRAW, WPARAM(0), LPARAM(0));
            SendMessageW(self.list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
            for row in &self.rows {
                let text = wide(&row.label);
                SendMessageW(
                    self.list,
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(text.as_ptr() as isize),
                );
            }
            let selected = old_identity
                .and_then(|identity| self.rows.iter().position(|row| row.identity == identity));
            let selected = selected.or_else(|| {
                (mode == SidebarMode::Files)
                    .then(|| {
                        self.rows
                            .iter()
                            .position(|row| row.action == PanelAction::File(path.clone()))
                    })
                    .flatten()
            });
            if let Some(index) = selected {
                SendMessageW(self.list, LB_SETCURSEL, WPARAM(index), LPARAM(0));
            }
            SendMessageW(
                self.list,
                LB_SETTOPINDEX,
                WPARAM(top.0.max(0) as usize),
                LPARAM(0),
            );
            SendMessageW(self.list, WM_SETREDRAW, WPARAM(1), LPARAM(0));
        }
        self.cache = Some(key);
        self.layout(
            state.metrics(),
            state.sidebar(),
            state.search_visible(),
            state.replace_visible(),
            state.appearance(),
        )?;
        self.repaint();
        Ok(())
    }

    fn refresh_search(&mut self, state: &mut ShellState) -> Result<(), ShellError> {
        let visible = state.search_visible();
        let query = if visible {
            self.query_text()
        } else {
            String::new()
        };
        let key = (
            state.document().session().path().to_owned(),
            state.document().session().revision(),
            query.clone(),
            visible,
        );
        let selection = state.document().session().selection().ordered_range();
        let editor = state
            .document_mut()
            .session_mut()
            .document_mut()
            .editor_mut();
        let changed = self.search_cache.as_ref() != Some(&key);
        if changed {
            if visible {
                editor.set_search_query(&query);
            } else {
                editor.clear_search();
            }
        }
        let caption = editor
            .search()
            .filter(|search| !search.query().is_empty())
            .map_or_else(String::new, |search| {
                format!(
                    "{}/{}",
                    search.current(selection).map_or(0, |index| index + 1),
                    search.matches().len()
                )
            });
        let can_replace = editor.search().is_some_and(|search| !search.is_empty())
            && editor.composition().is_none();
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus};
            if !can_replace && [self.replace_current, self.replace_all].contains(&GetFocus()) {
                let _ = SetFocus(self.replacement);
            }
            let _ = EnableWindow(self.replace_current, can_replace);
            let _ = EnableWindow(self.replace_all, can_replace);
        }
        if self.search_caption_text != caption {
            self.search_caption_text = caption;
            let label = wide(&format!(
                "{} {}",
                state.strings().search(),
                self.search_caption_text
            ));
            unsafe {
                self.accessibility
                    .SetHwndPropStr(
                        self.search_caption,
                        (-4i32) as u32,
                        0,
                        Name_Property_GUID,
                        PCWSTR(label.as_ptr()),
                    )
                    .map_err(error)?;
                let _ = InvalidateRect(self.search_caption, None, false);
            }
        }
        if changed {
            self.search_cache = Some(key);
            self.layout(
                state.metrics(),
                state.sidebar(),
                visible,
                state.replace_visible(),
                state.appearance(),
            )?;
        }
        Ok(())
    }

    pub(crate) fn layout(
        &mut self,
        metrics: WindowMetrics,
        mode: SidebarMode,
        search_visible: bool,
        replace_visible: bool,
        appearance: Appearance,
    ) -> Result<(), ShellError> {
        self.mode = mode;
        if self.dpi != metrics.dpi() {
            let font = Font::for_dpi(metrics.dpi(), 13.0, false, self.locale)?;
            let secondary_font = Font::for_dpi(metrics.dpi(), 11.0, false, self.locale)?;
            let tab_font = Font::for_dpi(metrics.dpi(), 13.0, true, self.locale)?;
            for hwnd in self.controls() {
                let handle = if [self.status, self.caption, self.empty, self.search_caption]
                    .contains(&hwnd)
                {
                    secondary_font.0
                } else if self.tabs.contains(&hwnd) {
                    tab_font.0
                } else {
                    font.0
                };
                unsafe {
                    SendMessageW(hwnd, WM_SETFONT, WPARAM(handle.0 as usize), LPARAM(1));
                }
            }
            // All controls have released the old font before it is deleted.
            self.font = Some(font);
            self.secondary_font = Some(secondary_font);
            self.tab_font = Some(tab_font);
            self.dpi = metrics.dpi();
            unsafe {
                SendMessageW(
                    self.list,
                    LB_SETITEMHEIGHT,
                    WPARAM(0),
                    LPARAM(metrics.px(28.0) as isize),
                );
            }
        }
        if self.appearance != appearance {
            self.palette = self.contrast.map_or_else(
                || Palette::new(appearance),
                |c| Palette::contrast(appearance, c),
            );
            self.appearance = appearance;
        }
        let (width, height) = (metrics.width_px() as i32, metrics.height_px() as i32);
        let content_h = height.max(1);
        let status_h = metrics.px(24.0).min(height);
        let status_w = metrics.px(180.0).min(width);
        let header_h = menu_height(metrics);
        let search_h = search_height(metrics, search_visible, replace_visible);
        let scrollbar_w = metrics.px(12.0).min(width);
        let scrollbar_gap = metrics.px(4.0);
        let sidebar_w = sidebar_width(metrics, mode);
        let padding = metrics.px(12.0);
        let inner = (sidebar_w - padding * 2).max(1);
        unsafe {
            let _ = MoveWindow(self.canvas, 0, 0, width, content_h, true);
            let _ = MoveWindow(
                self.background,
                0,
                header_h,
                sidebar_w,
                (content_h - header_h).max(1),
                true,
            );
            let _ = MoveWindow(self.menu_background, 0, 0, width, header_h, true);
            for (index, button) in self.menu_buttons.iter().enumerate() {
                let _ = MoveWindow(
                    *button,
                    padding + metrics.px(52.0) * index as i32,
                    metrics.px(3.0),
                    metrics.px(48.0),
                    metrics.px(26.0),
                    true,
                );
            }
            let _ = MoveWindow(
                self.status,
                (width - scrollbar_w - scrollbar_gap - status_w).max(0),
                (height - status_h).max(0),
                status_w,
                status_h,
                true,
            );
            let _ = SetWindowPos(
                self.status,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let scrollbar_top = (header_h + search_h).min(height);
            let _ = MoveWindow(
                self.document_scrollbar,
                (width - scrollbar_w).max(0),
                scrollbar_top,
                scrollbar_w,
                (height - scrollbar_top).max(1),
                true,
            );
            let _ = SetWindowPos(
                self.document_scrollbar,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(
                self.background,
                if mode == SidebarMode::Hidden {
                    SW_HIDE
                } else {
                    SW_SHOW
                },
            );
            for (index, tab) in self.tabs.iter().enumerate() {
                let left = padding + inner * index as i32 / 2;
                let right = padding + inner * (index as i32 + 1) / 2;
                let _ = MoveWindow(
                    *tab,
                    left + metrics.px(3.0),
                    header_h + padding + metrics.px(3.0),
                    right - left - metrics.px(6.0),
                    metrics.px(28.0),
                    true,
                );
                let _ = ShowWindow(
                    *tab,
                    if mode == SidebarMode::Hidden {
                        SW_HIDE
                    } else {
                        SW_SHOW
                    },
                );
            }
            let _ = MoveWindow(
                self.caption,
                padding,
                header_h + metrics.px(56.0),
                inner,
                metrics.px(20.0),
                true,
            );
            let top = header_h + metrics.px(84.0);
            let panel_height = (content_h - top - padding).max(1);
            let _ = MoveWindow(self.list, padding, top, inner, panel_height, true);
            let _ = MoveWindow(
                self.empty,
                padding,
                top,
                inner,
                metrics.px(80.0).min(panel_height),
                true,
            );
            let visible = mode != SidebarMode::Hidden;
            let _ = ShowWindow(self.caption, if visible { SW_SHOW } else { SW_HIDE });
            let _ = ShowWindow(
                self.list,
                if visible && !self.rows.is_empty() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            let search_w = (width - sidebar_w).max(1);
            let toggle_w = metrics.px(72.0);
            let search_left = sidebar_w + padding + toggle_w + metrics.px(8.0);
            let gap = metrics.px(8.0);
            let button_w = metrics.px(28.0);
            let close_w = metrics.px(60.0);
            let count_w = metrics.px(56.0);
            let query_w =
                (search_w - padding * 2 - gap * 5 - toggle_w - count_w - button_w * 2 - close_w)
                    .max(1);
            let top = header_h + metrics.px(12.0);
            let control_h = metrics.px(32.0);
            let _ = MoveWindow(
                self.replace_toggle,
                sidebar_w + padding,
                top,
                toggle_w,
                control_h,
                true,
            );
            let replacement_top = top + metrics.px(40.0);
            let replace_w = metrics.px(90.0);
            let all_w = metrics.px(116.0);
            let replacement_left = sidebar_w + padding;
            let replacement_w = (search_w - padding * 2 - gap * 2 - replace_w - all_w).max(1);
            let _ = MoveWindow(
                self.replacement_frame,
                replacement_left,
                replacement_top,
                replacement_w,
                control_h,
                true,
            );
            let _ = MoveWindow(
                self.replacement,
                replacement_left + metrics.px(10.0),
                replacement_top + metrics.px(6.0),
                (replacement_w - metrics.px(20.0)).max(1),
                metrics.px(20.0),
                true,
            );
            let _ = MoveWindow(
                self.replace_current,
                replacement_left + replacement_w + gap,
                replacement_top,
                replace_w,
                control_h,
                true,
            );
            let _ = MoveWindow(
                self.replace_all,
                replacement_left + replacement_w + gap * 2 + replace_w,
                replacement_top,
                all_w,
                control_h,
                true,
            );
            for hwnd in [
                self.replacement_frame,
                self.replacement,
                self.replace_current,
                self.replace_all,
            ] {
                let _ = ShowWindow(
                    hwnd,
                    if search_visible && replace_visible {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
            let _ = MoveWindow(
                self.search_background,
                sidebar_w,
                header_h,
                search_w,
                search_h.max(1),
                true,
            );
            let _ = MoveWindow(self.query_frame, search_left, top, query_w, control_h, true);
            let _ = MoveWindow(
                self.query,
                search_left + metrics.px(10.0),
                top + metrics.px(6.0),
                (query_w - metrics.px(20.0)).max(1),
                metrics.px(20.0),
                true,
            );
            let mut left = search_left + query_w + gap;
            for (hwnd, control_w) in [
                (self.search_caption, count_w),
                (self.search_previous, button_w),
                (self.search_next, button_w),
                (self.search_close, close_w),
            ] {
                let _ = MoveWindow(hwnd, left, top, control_w, control_h, true);
                left += control_w + gap;
            }
            for hwnd in [
                self.search_background,
                self.query_frame,
                self.replace_toggle,
                self.query,
                self.search_close,
                self.search_caption,
                self.search_previous,
                self.search_next,
            ] {
                let _ = ShowWindow(hwnd, if search_visible { SW_SHOW } else { SW_HIDE });
            }
            let _ = ShowWindow(
                self.empty,
                if visible && self.rows.is_empty() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        self.repaint();
        Ok(())
    }

    fn controls(&self) -> [HWND; 25] {
        [
            self.canvas,
            self.background,
            self.status,
            self.menu_background,
            self.menu_buttons[0],
            self.menu_buttons[1],
            self.menu_buttons[2],
            self.menu_buttons[3],
            self.tabs[0],
            self.tabs[1],
            self.caption,
            self.query,
            self.list,
            self.replacement,
            self.replacement_frame,
            self.replace_toggle,
            self.replace_current,
            self.replace_all,
            self.empty,
            self.query_frame,
            self.search_background,
            self.search_caption,
            self.search_previous,
            self.search_next,
            self.search_close,
        ]
    }
    pub(crate) fn clear_accessibility_annotations(&self) {
        for hwnd in [
            self.query,
            self.replacement,
            self.list,
            self.caption,
            self.empty,
            self.search_previous,
            self.search_next,
            self.search_caption,
        ] {
            unsafe {
                let _ = self.accessibility.ClearHwndProps(
                    hwnd,
                    (-4i32) as u32,
                    0,
                    &[Name_Property_GUID],
                );
            }
        }
    }
    fn repaint(&self) {
        for hwnd in self.controls() {
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }
    pub(crate) fn control_color(&self, dc: HDC, hwnd: HWND) -> Option<LRESULT> {
        if hwnd != self.query && hwnd != self.replacement && hwnd != self.list {
            return None;
        }
        let (color, brush) = if hwnd == self.query || hwnd == self.replacement {
            (self.palette.input_color, self.palette.input.0)
        } else {
            (self.palette.background_color, self.palette.background.0)
        };
        unsafe {
            SetTextColor(dc, self.palette.text);
            SetBkColor(dc, color);
        }
        Some(LRESULT(brush.0 as isize))
    }

    pub(crate) fn draw(&self, item: &DRAWITEMSTRUCT) -> bool {
        let hwnd = item.hwndItem;
        if !self.controls().contains(&hwnd) {
            return false;
        }
        let mut rect = item.rcItem;
        let px = |value: i32| value * self.dpi.max(96) as i32 / 96;
        unsafe {
            let saved = SaveDC(item.hDC);
            FillRect(
                item.hDC,
                &rect,
                if hwnd == self.canvas {
                    self.palette.canvas.0
                } else if [
                    self.search_background,
                    self.search_caption,
                    self.query_frame,
                    self.search_previous,
                    self.replacement_frame,
                    self.replace_toggle,
                    self.replace_current,
                    self.replace_all,
                    self.search_next,
                    self.search_close,
                ]
                .contains(&hwnd)
                {
                    self.palette.input.0
                } else {
                    self.palette.background.0
                },
            );
            let font =
                if [self.status, self.caption, self.empty, self.search_caption].contains(&hwnd) {
                    &self.secondary_font
                } else if self.tabs.contains(&hwnd) {
                    &self.tab_font
                } else {
                    &self.font
                };
            let previous = font.as_ref().map(|font| SelectObject(item.hDC, font.0));
            SetBkMode(item.hDC, TRANSPARENT);
            let mut text_color = self.palette.text;
            let mut flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
            let text = if hwnd == self.menu_background {
                let line = RECT {
                    top: rect.bottom - 1,
                    ..rect
                };
                FillRect(item.hDC, &line, self.palette.border.0);
                String::new()
            } else if let Some(index) = self.menu_buttons.iter().position(|button| *button == hwnd)
            {
                if pointer_inside(hwnd) || item.itemState.0 & ODS_SELECTED.0 != 0 {
                    round_fill(item.hDC, &rect, px(6), self.palette.hover.0);
                }
                flags |= DT_CENTER;
                text_color = self.palette.muted;
                self.menu_text[index].clone()
            } else if hwnd == self.search_background {
                FillRect(item.hDC, &rect, self.palette.input.0);
                let line = RECT {
                    top: rect.bottom - 1,
                    ..rect
                };
                FillRect(item.hDC, &line, self.palette.border.0);
                String::new()
            } else if [
                self.search_close,
                self.search_previous,
                self.search_next,
                self.replace_toggle,
                self.replace_current,
                self.replace_all,
            ]
            .contains(&hwnd)
            {
                let brush = if pointer_inside(hwnd) || item.itemState.0 & ODS_SELECTED.0 != 0 {
                    self.palette.hover.0
                } else {
                    self.palette.track.0
                };
                round_fill(item.hDC, &rect, px(6), brush);
                flags |= DT_CENTER;
                if !windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd).as_bool() {
                    text_color = self.palette.muted;
                }
                if [self.replace_toggle, self.replace_current, self.replace_all].contains(&hwnd) {
                    window_text(hwnd)
                } else if hwnd == self.search_close {
                    search_done(self.locale).into()
                } else if hwnd == self.search_next {
                    "↓".into()
                } else {
                    "↑".into()
                }
            } else if hwnd == self.canvas {
                String::new()
            } else if hwnd == self.background {
                let line = RECT {
                    left: rect.right - 1,
                    ..rect
                };
                FillRect(item.hDC, &line, self.palette.border.0);
                let track = RECT {
                    left: px(12),
                    top: px(12),
                    right: rect.right - px(12),
                    bottom: px(46),
                };
                round_fill(item.hDC, &track, px(8), self.palette.track.0);
                String::new()
            } else if hwnd == self.status {
                // Lightweight document details overlay: no reserved footer,
                // divider, or persistent "Ready" state. The small canvas-colored
                // patch only clears stale text when the character count changes.
                FillRect(item.hDC, &rect, self.palette.canvas.0);
                rect.left += px(8);
                rect.right -= px(12);
                text_color = self.palette.muted;
                flags |= DT_RIGHT;
                window_text(hwnd)
            } else if let Some(index) = self.tabs.iter().position(|tab| *tab == hwnd) {
                FillRect(item.hDC, &rect, self.palette.track.0);
                let selected_mode = [SidebarMode::Files, SidebarMode::Outline][index];
                if self.mode == selected_mode || item.itemState.0 & ODS_SELECTED.0 != 0 {
                    round_fill(item.hDC, &rect, px(6), self.palette.nav_selected.0);
                    text_color = self.palette.accent;
                } else if pointer_inside(hwnd) {
                    round_fill(item.hDC, &rect, px(6), self.palette.hover.0);
                }
                flags |= DT_CENTER;
                self.tab_text[index].clone()
            } else if hwnd == self.search_caption {
                FillRect(item.hDC, &rect, self.palette.input.0);
                text_color = self.palette.muted;
                flags |= DT_CENTER;
                self.search_caption_text.clone()
            } else if hwnd == self.caption {
                text_color = self.palette.muted;
                self.caption_text.clone()
            } else if hwnd == self.empty {
                text_color = self.palette.muted;
                flags = DT_WORDBREAK | DT_NOPREFIX;
                self.empty_text.clone()
            } else if hwnd == self.query_frame || hwnd == self.replacement_frame {
                round_fill(item.hDC, &rect, px(6), self.palette.input.0);
                let region = CreateRoundRectRgn(
                    rect.left,
                    rect.top,
                    rect.right,
                    rect.bottom,
                    px(12),
                    px(12),
                );
                let _ = FrameRgn(item.hDC, region, self.palette.border.0, px(1), px(1));
                let _ = DeleteObject(region);
                String::new()
            } else if hwnd == self.list {
                if item.itemState.0 & ODS_SELECTED.0 != 0 {
                    text_color = self.palette.selected_text;
                    let fill = RECT {
                        left: rect.left + px(2),
                        top: rect.top + px(2),
                        right: rect.right - px(2),
                        bottom: rect.bottom - px(2),
                    };
                    round_fill(item.hDC, &fill, px(5), self.palette.selected.0);
                }
                let row = self.rows.get(item.itemID as usize);
                rect.left += px(8 + row.map_or(0, |row| i32::from(row.indent) * 12));
                rect.right -= px(8);
                row.map_or_else(String::new, |row| row.label.clone())
            } else {
                String::new()
            };
            SetTextColor(item.hDC, text_color);
            let mut units: Vec<u16> = text.encode_utf16().collect();
            if !units.is_empty() {
                DrawTextW(item.hDC, &mut units, &mut rect, flags);
            }
            if item.itemState.0 & ODS_FOCUS.0 != 0 {
                let _ = DrawFocusRect(item.hDC, &item.rcItem);
            }
            if let Some(previous) = previous {
                SelectObject(item.hDC, previous);
            }
            if saved != 0 {
                let _ = RestoreDC(item.hDC, saved);
            }
        }
        true
    }
}

fn menu_label(text: &str) -> String {
    text.split('(')
        .next()
        .unwrap_or(text)
        .replace('&', "")
        .trim()
        .to_owned()
}

pub(crate) fn menu_height(metrics: WindowMetrics) -> i32 {
    metrics.px(32.0)
}

fn search_step(locale: Locale, forward: bool) -> &'static str {
    match (locale, forward) {
        (Locale::English, false) => "Previous match",
        (Locale::English, true) => "Next match",
        (Locale::SimplifiedChinese, false) => "上一个匹配",
        (Locale::SimplifiedChinese, true) => "下一个匹配",
        (Locale::TraditionalChinese, false) => "上一個符合項目",
        (Locale::TraditionalChinese, true) => "下一個符合項目",
        (Locale::Japanese, false) => "前の一致",
        (Locale::Japanese, true) => "次の一致",
        (Locale::Korean, false) => "이전 일치",
        (Locale::Korean, true) => "다음 일치",
    }
}

fn search_done(locale: Locale) -> &'static str {
    match locale {
        Locale::English => "Done",
        Locale::SimplifiedChinese | Locale::TraditionalChinese => "完成",
        Locale::Japanese => "完了",
        Locale::Korean => "완료",
    }
}

pub(crate) fn search_height(metrics: WindowMetrics, visible: bool, replacement: bool) -> i32 {
    if visible {
        metrics
            .px(if replacement { 96.0 } else { 56.0 })
            .min((metrics.height_px() as i32).max(1) / 3)
    } else {
        0
    }
}

pub(crate) fn pointer_inside(hwnd: HWND) -> bool {
    let mut point = POINT::default();
    let mut rect = RECT::default();
    let valid = unsafe {
        GetCursorPos(&mut point).is_ok()
            && ScreenToClient(hwnd, &mut point).as_bool()
            && GetClientRect(hwnd, &mut rect).is_ok()
    };
    valid && point.x >= 0 && point.y >= 0 && point.x < rect.right && point.y < rect.bottom
}

unsafe extern "system" fn overlay_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_NCHITTEST {
        // Match the macOS overlay: status text is visual only and never steals
        // pointer input from the editor beneath it.
        return LRESULT(-1);
    }
    if message == WM_NCDESTROY {
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(overlay_subclass), 1);
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub(crate) unsafe extern "system" fn tab_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_MOUSEMOVE || message == WM_MOUSELEAVE {
        unsafe {
            if message == WM_MOUSEMOVE {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut tracking);
            }
            let _ = InvalidateRect(hwnd, None, false);
        }
    } else if message == WM_NCDESTROY {
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(tab_subclass), 1);
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn query_subclass(
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
                w!("YuSearchComposition"),
                HANDLE(std::ptr::dangling_mut()),
            );
        } else if message == WM_IME_ENDCOMPOSITION || message == WM_NCDESTROY {
            let _ = RemovePropW(hwnd, w!("YuSearchComposition"));
            if message == WM_NCDESTROY {
                let _ = RemoveWindowSubclass(hwnd, Some(query_subclass), 1);
            }
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

pub(crate) fn sidebar_width(metrics: WindowMetrics, mode: SidebarMode) -> i32 {
    if mode == SidebarMode::Hidden {
        0
    } else {
        metrics
            .px(240.0)
            .min((metrics.width_px() as f32 * 0.35).round() as i32)
    }
}

pub(crate) fn ui_font_family(locale: Locale) -> &'static str {
    match locale {
        Locale::SimplifiedChinese | Locale::TraditionalChinese => "Microsoft YaHei UI",
        Locale::Japanese => "Yu Gothic UI",
        Locale::Korean => "Malgun Gothic",
        Locale::English => "Segoe UI",
    }
}

pub(crate) unsafe fn round_fill(dc: HDC, rect: &RECT, radius: i32, brush: HBRUSH) {
    unsafe {
        let region = CreateRoundRectRgn(
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        let _ = FillRgn(dc, region, brush);
        let _ = DeleteObject(region);
    }
}

fn child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: u32,
    id: u16,
    tabstop: bool,
) -> Result<HWND, ShellError> {
    let text = wide(text);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD
                | WS_VISIBLE
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

fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut units = vec![0; length + 1];
    let length = unsafe { GetWindowTextW(hwnd, &mut units) }.max(0) as usize;
    String::from_utf16_lossy(&units[..length])
}

fn directory_rows(
    directory: &std::path::Path,
    root: &std::path::Path,
) -> Result<Vec<Row>, ShellError> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(error)?.take(10_000) {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() || (kind.is_file() && yu_storage::WorkspaceIndex::supports(&entry.path()))
        {
            paths.push((!kind.is_dir(), entry.path()));
        }
    }
    paths.sort_by_key(|(file, path)| {
        (
            *file,
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase()),
        )
    });
    let mut rows = Vec::new();
    if directory != root
        && let Some(parent) = directory.parent()
        && parent.starts_with(root)
    {
        rows.push(Row {
            label: "..".into(),
            identity: parent.to_string_lossy().into_owned(),
            indent: 0,
            action: PanelAction::Directory(parent.to_owned()),
        });
    }
    rows.extend(paths.into_iter().map(|(file, path)| Row {
        label: format!(
            "{}{}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            if file { "" } else { "/" }
        ),
        identity: path.to_string_lossy().into_owned(),
        indent: 0,
        action: if file {
            PanelAction::File(path)
        } else {
            PanelAction::Directory(path)
        },
    }));
    Ok(rows)
}

fn panel_hint(locale: Locale, kind: usize) -> &'static str {
    match locale {
        Locale::English => [
            "Open a Markdown file to browse its folder.",
            "This document has no headings.",
            "Search this document…",
            "No matches.",
        ][kind],
        Locale::SimplifiedChinese => [
            "打开 Markdown 文件后，可浏览同目录文件。",
            "当前文档没有标题。",
            "输入文字搜索当前文档…",
            "没有匹配结果。",
        ][kind],
        Locale::TraditionalChinese => [
            "開啟 Markdown 檔案後，可瀏覽同目錄檔案。",
            "目前文件沒有標題。",
            "輸入文字搜尋目前文件…",
            "沒有符合的結果。",
        ][kind],
        Locale::Japanese => [
            "Markdown ファイルを開くと同じフォルダーを表示します。",
            "見出しがありません。",
            "文書内を検索…",
            "一致する結果がありません。",
        ][kind],
        Locale::Korean => [
            "Markdown 파일을 열면 같은 폴더를 표시합니다.",
            "문서에 제목이 없습니다.",
            "문서 내 검색…",
            "검색 결과가 없습니다.",
        ][kind],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{GetObjectW, LOGFONTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, LB_GETITEMHEIGHT, SetWindowTextW, WM_GETFONT, WS_POPUP,
    };
    use yu_editor::EditorCommand;

    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    fn window() -> Window {
        Window(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Chrome regression"),
                WS_POPUP,
                0,
                0,
                1200,
                800,
                None,
                None,
                None,
                None,
            )
            .expect("hidden parent")
        })
    }
    fn font_height(hwnd: HWND) -> i32 {
        let handle = unsafe { SendMessageW(hwnd, WM_GETFONT, WPARAM(0), LPARAM(0)) };
        let mut font = LOGFONTW::default();
        assert_eq!(
            unsafe {
                GetObjectW(
                    HFONT(handle.0 as *mut _),
                    size_of::<LOGFONTW>() as i32,
                    Some(std::ptr::from_mut(&mut font).cast()),
                )
            },
            size_of::<LOGFONTW>() as i32
        );
        font.lfHeight.abs()
    }

    #[test]
    fn native_controls_replace_fonts_and_row_heights_across_dpi() {
        let _com = crate::native::ComApartment::initialize().expect("COM for annotations");
        let window = window();
        let state = ShellState::new(Locale::English);
        let mut chrome = Chrome::new(window.0, &state).expect("controls");
        chrome
            .layout(
                WindowMetrics::new(1200, 800, 96),
                SidebarMode::Files,
                false,
                false,
                Appearance::Light,
            )
            .expect("96 DPI");
        let secondary_height = font_height(chrome.status);
        let normal_height = font_height(chrome.query);
        assert!(secondary_height >= 11);
        assert!(normal_height > secondary_height);
        chrome
            .layout(
                WindowMetrics::new(2400, 1600, 192),
                SidebarMode::Files,
                true,
                true,
                Appearance::Dark,
            )
            .expect("192 DPI");
        for hwnd in chrome.controls() {
            assert!(font_height(hwnd) >= secondary_height * 2 - 1);
        }
        assert_eq!(
            unsafe { SendMessageW(chrome.list, LB_GETITEMHEIGHT, WPARAM(0), LPARAM(0)) }.0,
            56
        );
        // Going back must release the large font and use the smaller one.
        chrome
            .layout(
                WindowMetrics::new(1200, 800, 96),
                SidebarMode::Outline,
                false,
                false,
                Appearance::Light,
            )
            .expect("restore DPI");
        assert_eq!(font_height(chrome.query), normal_height);
        assert_eq!(font_height(chrome.status), secondary_height);
        assert_eq!(
            unsafe { SendMessageW(chrome.list, LB_GETITEMHEIGHT, WPARAM(0), LPARAM(0)) }.0,
            28
        );
    }

    #[test]
    fn native_panels_use_shared_labels_and_unicode_ranges_after_edits() {
        let _com = crate::native::ComApartment::initialize().expect("COM for annotations");
        let window = window();
        let mut state = ShellState::new(Locale::SimplifiedChinese);
        let source = "# **中文** 😀\n\n## Child `code`\n\n中文😀 中文😀\n";
        state
            .document_mut()
            .session_mut()
            .execute(EditorCommand::insert_text(source))
            .expect("source");
        state.set_metrics(WindowMetrics::new(1200, 800, 96));
        state.set_sidebar(SidebarMode::Outline);
        let mut chrome = Chrome::new(window.0, &state).expect("controls");
        chrome.refresh(&mut state).expect("outline");
        assert_eq!(chrome.rows.len(), 2);
        assert_eq!(chrome.rows[0].label, "中文 😀");
        assert_eq!(chrome.rows[1].label, "Child code");
        let revision = state.document().session().revision();
        state.set_search_visible(true);
        let query = wide("中文😀");
        unsafe {
            SetWindowTextW(chrome.query, PCWSTR(query.as_ptr())).expect("query");
        }
        chrome.refresh(&mut state).expect("search");
        assert_eq!(
            state
                .document()
                .session()
                .document()
                .editor()
                .search()
                .expect("search")
                .matches()
                .len(),
            2
        );
        assert_eq!(
            chrome.rows[0].label, "中文 😀",
            "search must preserve the outline"
        );
        let start = source.find("中文😀").expect("match") as u64;
        assert_eq!(
            state
                .document()
                .session()
                .document()
                .editor()
                .search()
                .expect("search")
                .matches()[0],
            TextRange::new(
                yu_core::ByteOffset::new(start),
                yu_core::ByteOffset::new(start + "中文😀".len() as u64)
            )
            .expect("range")
        );
        assert_eq!(
            state.document().session().revision(),
            revision,
            "panel refresh must not edit source"
        );
        state
            .document_mut()
            .session_mut()
            .execute(EditorCommand::Undo)
            .expect("undo source");
        chrome
            .refresh(&mut state)
            .expect("refresh search after undo");
        assert!(
            state
                .document()
                .session()
                .document()
                .editor()
                .search()
                .expect("search")
                .is_empty(),
            "old search ranges must not survive source edits"
        );
        state.set_sidebar(SidebarMode::Outline);
        chrome.refresh(&mut state).expect("refresh outline");
        assert!(chrome.rows.is_empty());
    }
}
