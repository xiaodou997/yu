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
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CreateWindowExW, ES_AUTOHSCROLL, GetClientRect, GetCursorPos,
    GetWindowTextLengthW, GetWindowTextW, HMENU, HWND_BOTTOM, HWND_TOP, LB_ADDSTRING, LB_GETCURSEL,
    LB_GETTOPINDEX, LB_RESETCONTENT, LB_SETCURSEL, LB_SETITEMHEIGHT, LB_SETTOPINDEX,
    LBS_HASSTRINGS, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LBS_OWNERDRAWFIXED, MoveWindow,
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SendMessageW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_MOUSEMOVE, WM_NCDESTROY, WM_SETFONT, WM_SETREDRAW, WS_CHILD, WS_CLIPSIBLINGS, WS_TABSTOP,
    WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{PCWSTR, w};
use yu_core::{Revision, TextRange};
use yu_editor::{OutlineTree, SearchResults};
use yu_workspace::Appearance;

pub(crate) const ID_FILES: u16 = 2001;
pub(crate) const ID_OUTLINE: u16 = 2002;
pub(crate) const ID_SEARCH: u16 = 2003;
pub(crate) const ID_QUERY: u16 = 2004;
pub(crate) const ID_ROWS: u16 = 2005;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn error(error: impl std::fmt::Display) -> ShellError {
    ShellError::Platform(error.to_string())
}

struct Font(HFONT);
impl Font {
    fn for_dpi(dpi: u32, size: f32, bold: bool, locale: Locale) -> Result<Self, ShellError> {
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
        for (target, unit) in font
            .lfFaceName
            .iter_mut()
            .zip(ui_font_family(locale).encode_utf16())
        {
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
struct Brush(HBRUSH);
impl Brush {
    fn new(color: COLORREF) -> Self {
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

pub(crate) struct Chrome {
    accessibility: IAccPropServices,
    canvas: HWND,
    pub(crate) background: HWND,
    pub(crate) status: HWND,
    pub(crate) query: HWND,
    pub(crate) list: HWND,
    tabs: [HWND; 3],
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
    cache: Option<(PathBuf, Revision, SidebarMode, String)>,
    empty_text: String,
    caption_text: String,
    tab_text: [String; 3],
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
        self.tabs
            .iter()
            .copied()
            .chain([self.query, self.list])
            .filter(|hwnd| {
                unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(*hwnd) }.as_bool()
            })
            .collect()
    }
    pub(crate) fn new(parent: HWND, state: &ShellState) -> Result<Self, ShellError> {
        let strings = state.strings();
        let canvas = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2014, false)?;
        let background = child(parent, w!("STATIC"), "", SS_OWNERDRAW.0, 2010, false)?;
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
            child(
                parent,
                w!("BUTTON"),
                strings.search(),
                BS_OWNERDRAW as u32,
                ID_SEARCH,
                true,
            )?,
        ];
        for tab in tabs {
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
        Ok(Self {
            accessibility,
            canvas,
            background,
            status,
            query,
            list,
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
            cache: None,
            empty_text: String::new(),
            caption_text: String::new(),
            tab_text: [
                strings.files().into(),
                strings.outline().into(),
                strings.search().into(),
            ],
        })
    }

    pub(crate) fn invalidate_content(&mut self) {
        self.cache = None;
    }
    pub(crate) fn invalidate_font(&mut self) {
        self.dpi = 0;
    }
    pub(crate) fn query_text(&self) -> String {
        window_text(self.query)
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
    pub(crate) fn select_first(&self) {
        if !self.rows.is_empty() {
            unsafe {
                SendMessageW(self.list, LB_SETCURSEL, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub(crate) fn refresh(&mut self, state: &mut ShellState) -> Result<(), ShellError> {
        let mode = state.sidebar();
        if mode == SidebarMode::Hidden {
            self.mode = mode;
            return Ok(());
        }
        let path = state.document().session().path().to_owned();
        let revision = state.document().session().revision();
        let query = self.query_text();
        let key = (path.clone(), revision, mode, query.clone());
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
                let rows = if state.document().is_untitled() {
                    Vec::new()
                } else {
                    directory_rows(&path)?
                };
                let folder = path
                    .parent()
                    .and_then(|parent| parent.file_name())
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
            SidebarMode::Search => {
                editor.set_search_query(&query);
                let matches = SearchResults::build(editor).map_err(error)?;
                let rows = matches
                    .rows()
                    .iter()
                    .map(|row| Row {
                        label: row.label().to_owned(),
                        identity: format!("{}", row.hit().start().get()),
                        indent: 0,
                        action: PanelAction::Select(row.hit()),
                    })
                    .collect::<Vec<_>>();
                let caption = format!("{} · {}", strings.search(), rows.len());
                (
                    rows,
                    caption,
                    panel_hint(locale, if query.is_empty() { 2 } else { 3 }).into(),
                )
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
                    _ => strings.search(),
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
        self.layout(state.metrics(), state.sidebar(), state.appearance())?;
        self.repaint();
        Ok(())
    }

    pub(crate) fn layout(
        &mut self,
        metrics: WindowMetrics,
        mode: SidebarMode,
        appearance: Appearance,
    ) -> Result<(), ShellError> {
        self.mode = mode;
        if self.dpi != metrics.dpi() {
            let font = Font::for_dpi(metrics.dpi(), 13.0, false, self.locale)?;
            let secondary_font = Font::for_dpi(metrics.dpi(), 11.0, false, self.locale)?;
            let tab_font = Font::for_dpi(metrics.dpi(), 13.0, true, self.locale)?;
            for hwnd in self.controls() {
                let handle = if hwnd == self.status || hwnd == self.caption || hwnd == self.empty {
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
        let status_h = metrics.px(24.0).min(height);
        let content_h = (height - status_h).max(1);
        let sidebar_w = sidebar_width(metrics, mode);
        let padding = metrics.px(12.0);
        let inner = (sidebar_w - padding * 2).max(1);
        unsafe {
            let _ = MoveWindow(self.canvas, 0, 0, width, content_h, true);
            let _ = MoveWindow(self.background, 0, 0, sidebar_w, content_h, true);
            let _ = MoveWindow(self.status, 0, content_h, width, status_h, true);
            let _ = ShowWindow(
                self.background,
                if mode == SidebarMode::Hidden {
                    SW_HIDE
                } else {
                    SW_SHOW
                },
            );
            for (index, tab) in self.tabs.iter().enumerate() {
                let left = padding + inner * index as i32 / 3;
                let right = padding + inner * (index as i32 + 1) / 3;
                let _ = MoveWindow(
                    *tab,
                    left + metrics.px(3.0),
                    padding + metrics.px(3.0),
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
                metrics.px(56.0),
                inner,
                metrics.px(20.0),
                true,
            );
            let _ = MoveWindow(
                self.query_frame,
                padding,
                metrics.px(84.0),
                inner,
                metrics.px(32.0),
                true,
            );
            let _ = MoveWindow(
                self.query,
                padding + metrics.px(10.0),
                metrics.px(90.0),
                (inner - metrics.px(20.0)).max(1),
                metrics.px(20.0),
                true,
            );
            let top = metrics.px(if mode == SidebarMode::Search {
                128.0
            } else {
                84.0
            });
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
                self.query_frame,
                if visible && mode == SidebarMode::Search {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            let _ = ShowWindow(
                self.query,
                if visible && mode == SidebarMode::Search {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            let _ = ShowWindow(
                self.list,
                if visible && !self.rows.is_empty() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
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

    fn controls(&self) -> [HWND; 11] {
        [
            self.canvas,
            self.background,
            self.status,
            self.tabs[0],
            self.tabs[1],
            self.tabs[2],
            self.caption,
            self.query,
            self.list,
            self.empty,
            self.query_frame,
        ]
    }
    pub(crate) fn clear_accessibility_annotations(&self) {
        for hwnd in [self.query, self.list, self.caption, self.empty] {
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
        if hwnd != self.query && hwnd != self.list {
            return None;
        }
        let (color, brush) = if hwnd == self.query {
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
                } else {
                    self.palette.background.0
                },
            );
            let font = if hwnd == self.status || hwnd == self.caption || hwnd == self.empty {
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
            let text = if hwnd == self.canvas {
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
                FillRect(item.hDC, &rect, self.palette.canvas.0);
                let line = RECT {
                    bottom: rect.top + 1,
                    ..rect
                };
                FillRect(item.hDC, &line, self.palette.border.0);
                rect.left += px(12);
                rect.right -= px(12);
                text_color = self.palette.muted;
                let status = window_text(hwnd);
                if let Some((ready, details)) = status.split_once('·') {
                    SetTextColor(item.hDC, text_color);
                    let mut details: Vec<u16> = details.trim().encode_utf16().collect();
                    DrawTextW(item.hDC, &mut details, &mut rect, flags | DT_RIGHT);
                    ready.trim().to_owned()
                } else {
                    status
                }
            } else if let Some(index) = self.tabs.iter().position(|tab| *tab == hwnd) {
                FillRect(item.hDC, &rect, self.palette.track.0);
                let selected_mode = [
                    SidebarMode::Files,
                    SidebarMode::Outline,
                    SidebarMode::Search,
                ][index];
                if self.mode == selected_mode || item.itemState.0 & ODS_SELECTED.0 != 0 {
                    round_fill(item.hDC, &rect, px(6), self.palette.nav_selected.0);
                    text_color = self.palette.accent;
                } else if pointer_inside(hwnd) {
                    round_fill(item.hDC, &rect, px(6), self.palette.hover.0);
                }
                flags |= DT_CENTER;
                self.tab_text[index].clone()
            } else if hwnd == self.caption {
                text_color = self.palette.muted;
                self.caption_text.clone()
            } else if hwnd == self.empty {
                text_color = self.palette.muted;
                flags = DT_WORDBREAK | DT_NOPREFIX;
                self.empty_text.clone()
            } else if hwnd == self.query_frame {
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

fn pointer_inside(hwnd: HWND) -> bool {
    let mut point = POINT::default();
    let mut rect = RECT::default();
    let valid = unsafe {
        GetCursorPos(&mut point).is_ok()
            && ScreenToClient(hwnd, &mut point).as_bool()
            && GetClientRect(hwnd, &mut rect).is_ok()
    };
    valid && point.x >= 0 && point.y >= 0 && point.x < rect.right && point.y < rect.bottom
}

unsafe extern "system" fn tab_subclass(
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

unsafe fn round_fill(dc: HDC, rect: &RECT, radius: i32, brush: HBRUSH) {
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

fn directory_rows(path: &std::path::Path) -> Result<Vec<Row>, ShellError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(parent).map_err(error)? {
        let entry = entry.map_err(error)?;
        if entry.file_type().map_err(error)?.is_file()
            && entry.path().extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
            })
        {
            paths.push(entry.path());
        }
    }
    paths.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    });
    Ok(paths
        .into_iter()
        .map(|path| Row {
            label: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            identity: path.to_string_lossy().into_owned(),
            indent: 0,
            action: PanelAction::File(path),
        })
        .collect())
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
                SidebarMode::Search,
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
        state.set_sidebar(SidebarMode::Search);
        let query = wide("中文😀");
        unsafe {
            SetWindowTextW(chrome.query, PCWSTR(query.as_ptr())).expect("query");
        }
        chrome.refresh(&mut state).expect("search");
        assert_eq!(chrome.rows.len(), 2);
        let start = source.find("中文😀").expect("match") as u64;
        assert_eq!(
            chrome.rows[0].action,
            PanelAction::Select(
                TextRange::new(
                    yu_core::ByteOffset::new(start),
                    yu_core::ByteOffset::new(start + "中文😀".len() as u64)
                )
                .expect("range")
            )
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
            chrome.rows.is_empty(),
            "old search ranges must not survive source edits"
        );
        state.set_sidebar(SidebarMode::Outline);
        chrome.refresh(&mut state).expect("refresh outline");
        assert!(chrome.rows.is_empty());
    }
}
