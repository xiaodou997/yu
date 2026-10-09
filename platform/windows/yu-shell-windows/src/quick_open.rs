//! Native file switcher over the shared metadata-only, cancellable index.
use crate::chrome::Font;
use crate::locale::WorkspaceText as Text;
use crate::{Locale, ShellError, Strings, WindowMetrics};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Controls::EM_SETLIMITTEXT;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetActiveWindow, SetFocus, VK_DOWN, VK_ESCAPE,
    VK_RETURN, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};
use yu_storage::{WorkspaceFile, WorkspaceIndex, WorkspaceScan};

const QUERY: usize = 4101;
const RESULTS: usize = 4102;
const REFRESH: usize = 4103;
const COMMITTED: u32 = WM_APP + 37;
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn error(value: impl std::fmt::Display) -> ShellError {
    ShellError::Platform(value.to_string())
}
fn text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut buffer = vec![0; length + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..copied])
}
fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}
fn composing(hwnd: HWND) -> bool {
    !unsafe { GetPropW(hwnd, w!("YuQuickComposition")) }.is_invalid()
}

struct NativeWindow(HWND);
impl Drop for NativeWindow {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.0).as_bool() {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}
struct OwnerGuard(HWND, HWND);
impl Drop for OwnerGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = EnableWindow(self.0, true);
            let _ = SetActiveWindow(self.0);
            let _ = SetFocus(self.1);
        }
    }
}

struct Picker {
    root: PathBuf,
    strings: Strings,
    locale: Locale,
    font: Font,
    controls: [HWND; 7], // root, query, results, status, refresh, cancel, open
    scan: Option<WorkspaceScan>,
    index: Option<WorkspaceIndex>,
    rows: Vec<WorkspaceFile>,
    done: bool,
    result: Option<PathBuf>,
}
impl Picker {
    fn refresh(&mut self) {
        self.index = None;
        self.rows.clear();
        unsafe {
            SendMessageW(self.controls[2], LB_RESETCONTENT, WPARAM(0), LPARAM(0));
        }
        self.enable_open(false);
        match WorkspaceScan::start(self.root.clone()) {
            Ok(scan) => {
                self.scan = Some(scan);
                set_text(self.controls[3], self.strings.workspace(Text::Scanning));
            }
            Err(_) => {
                self.scan = None;
                set_text(self.controls[3], self.strings.workspace(Text::Unavailable));
            }
        }
    }
    fn enable_open(&self, enabled: bool) {
        unsafe {
            if !enabled && GetFocus() == self.controls[6] {
                let _ = SetFocus(self.controls[1]);
            }
            let _ = EnableWindow(self.controls[6], enabled);
        }
    }
    fn poll(&mut self) {
        let Some(result) = self.scan.as_ref().and_then(WorkspaceScan::poll) else {
            return;
        };
        self.scan = None;
        match result {
            Ok(index) => {
                self.index = Some(index);
                self.query();
            }
            Err(_) => {
                self.enable_open(false);
                set_text(self.controls[3], self.strings.workspace(Text::Unavailable));
            }
        }
    }
    fn query(&mut self) {
        if composing(self.controls[1]) {
            return;
        }
        let Some(index) = self.index.as_ref() else {
            return;
        };
        let query = text(self.controls[1]);
        let old_row =
            unsafe { SendMessageW(self.controls[2], LB_GETCURSEL, WPARAM(0), LPARAM(0)) }.0;
        let old_path = usize::try_from(old_row)
            .ok()
            .and_then(|row| self.rows.get(row))
            .map(|file| file.path.clone());
        self.rows = index.search(&query, 100).into_iter().cloned().collect();
        let status = if self.rows.is_empty() {
            self.strings.workspace(Text::NoMatches).to_owned()
        } else {
            format!(
                "{} / {} — {}",
                self.rows.len(),
                index.files.len(),
                self.strings.workspace(Text::Files)
            )
        };
        let status = if index.truncated || index.skipped > 0 {
            format!("{status} — {}", self.strings.workspace(Text::Partial))
        } else {
            status
        };
        unsafe {
            SendMessageW(self.controls[2], WM_SETREDRAW, WPARAM(0), LPARAM(0));
            SendMessageW(self.controls[2], LB_RESETCONTENT, WPARAM(0), LPARAM(0));
            for row in &self.rows {
                let label = wide(&row.relative);
                SendMessageW(
                    self.controls[2],
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(label.as_ptr() as isize),
                );
            }
            if !self.rows.is_empty() {
                let row = self
                    .rows
                    .iter()
                    .position(|file| Some(&file.path) == old_path.as_ref())
                    .unwrap_or(0);
                SendMessageW(self.controls[2], LB_SETCURSEL, WPARAM(row), LPARAM(0));
            }
            SendMessageW(self.controls[2], WM_SETREDRAW, WPARAM(1), LPARAM(0));
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(self.controls[2], None, true);
        }
        self.enable_open(!self.rows.is_empty());
        set_text(self.controls[3], &status);
    }
    fn choose(&mut self) {
        if !unsafe { IsWindowEnabled(self.controls[6]) }.as_bool() {
            return;
        }
        if composing(self.controls[1]) {
            return;
        }
        let row = unsafe { SendMessageW(self.controls[2], LB_GETCURSEL, WPARAM(0), LPARAM(0)) }.0;
        let Some(file) = usize::try_from(row).ok().and_then(|row| self.rows.get(row)) else {
            return;
        };
        let Some(index) = self.index.as_ref() else {
            return;
        };
        match index.resolve(&file.path) {
            Ok(path) => {
                self.result = Some(path);
                self.done = true;
            }
            Err(_) => {
                set_text(
                    self.controls[3],
                    self.strings.workspace(Text::FileUnavailable),
                );
                self.enable_open(false);
            }
        }
    }
    fn layout(&mut self, hwnd: HWND) {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let m = WindowMetrics::new(0, 0, dpi);
        let mut rect = RECT::default();
        if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
            return;
        }
        if let Ok(font) = Font::for_dpi(dpi, 13.0, false, self.locale) {
            for control in self.controls {
                unsafe {
                    SendMessageW(control, WM_SETFONT, WPARAM(font.0.0 as usize), LPARAM(1));
                }
            }
            self.font = font;
        }
        let width = (rect.right - m.px(32.0)).max(1);
        let bottom = rect.bottom;
        let button_width = m.px(104.0);
        let positions = [
            (m.px(16.0), m.px(10.0), width, m.px(24.0)),
            (m.px(16.0), m.px(42.0), width, m.px(28.0)),
            (m.px(16.0), m.px(82.0), width, (bottom - m.px(178.0)).max(1)),
            (m.px(16.0), bottom - m.px(86.0), width, m.px(36.0)),
            (m.px(16.0), bottom - m.px(42.0), m.px(180.0), m.px(28.0)),
            (
                rect.right - m.px(24.0) - button_width * 2,
                bottom - m.px(42.0),
                button_width,
                m.px(28.0),
            ),
            (
                rect.right - m.px(16.0) - button_width,
                bottom - m.px(42.0),
                button_width,
                m.px(28.0),
            ),
        ];
        for (control, (x, y, w, h)) in self.controls.into_iter().zip(positions) {
            unsafe {
                let _ = MoveWindow(control, x, y, w, h, true);
            }
        }
    }
}

unsafe extern "system" fn query_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if message == WM_IME_STARTCOMPOSITION {
            let _ = SetPropW(
                hwnd,
                w!("YuQuickComposition"),
                HANDLE(std::ptr::dangling_mut()),
            );
        }
        if message == WM_IME_ENDCOMPOSITION {
            let _ = RemovePropW(hwnd, w!("YuQuickComposition"));
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(parent, COMMITTED, WPARAM(0), LPARAM(0));
            }
        }
        if message == WM_NCDESTROY {
            let _ = RemovePropW(hwnd, w!("YuQuickComposition"));
            let _ = RemoveWindowSubclass(hwnd, Some(query_proc), 1);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}
unsafe extern "system" fn picker_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let state = unsafe { &mut *(data as *mut Picker) };
    match message {
        WM_TIMER => {
            state.poll();
            return LRESULT(0);
        }
        COMMITTED => {
            state.query();
            return LRESULT(0);
        }
        WM_COMMAND => {
            let command = w.0 & 0xffff;
            let event = w.0 >> 16;
            match command {
                QUERY if event == EN_CHANGE as usize => state.query(),
                RESULTS if event == LBN_SELCHANGE as usize => {
                    state.enable_open(!state.rows.is_empty())
                }
                RESULTS if event == LBN_DBLCLK as usize => state.choose(),
                1 => state.choose(),
                2 if !composing(state.controls[1]) => state.done = true,
                REFRESH => state.refresh(),
                _ => {}
            }
            return LRESULT(0);
        }
        WM_SIZE => state.layout(hwnd),
        WM_DPICHANGED => {
            let rect = unsafe { &*(l.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            state.layout(hwnd);
            return LRESULT(0);
        }
        WM_CLOSE => {
            state.done = true;
            return LRESULT(0);
        }
        WM_NCDESTROY => {
            state.done = true;
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(picker_proc), 1);
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, w, l) }
}

pub(crate) fn show(
    owner: HWND,
    root: &Path,
    locale: Locale,
) -> Result<Option<PathBuf>, ShellError> {
    let strings = locale.strings();
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
    let m = WindowMetrics::new(0, 0, dpi);
    let font = Font::for_dpi(dpi, 13.0, false, locale)?;
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let ex = WS_EX_DLGMODALFRAME;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: m.px(640.0),
        bottom: m.px(450.0),
    };
    unsafe { AdjustWindowRectExForDpi(&mut rect, style, false, ex, dpi) }.map_err(error)?;
    let mut owner_rect = RECT::default();
    unsafe { GetWindowRect(owner, &mut owner_rect) }.map_err(error)?;
    let title = wide(strings.workspace(Text::QuickOpen));
    let window = NativeWindow(
        unsafe {
            CreateWindowExW(
                ex,
                w!("#32770"),
                PCWSTR(title.as_ptr()),
                style,
                owner_rect.left
                    + ((owner_rect.right - owner_rect.left) - (rect.right - rect.left)) / 2,
                owner_rect.top
                    + ((owner_rect.bottom - owner_rect.top) - (rect.bottom - rect.top)) / 2,
                rect.right - rect.left,
                rect.bottom - rect.top,
                owner,
                None,
                None,
                None,
            )
        }
        .map_err(error)?,
    );
    let child =
        |class: PCWSTR, label: &str, id: usize, bits: u32, tab: bool| -> Result<HWND, ShellError> {
            let label = wide(label);
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    PCWSTR(label.as_ptr()),
                    WS_CHILD
                        | WS_VISIBLE
                        | WINDOW_STYLE(bits)
                        | if tab { WS_TABSTOP } else { WINDOW_STYLE(0) },
                    0,
                    0,
                    1,
                    1,
                    window.0,
                    HMENU(id as *mut _),
                    None,
                    None,
                )
            }
            .map_err(error)
        };
    let controls = [
        child(
            w!("STATIC"),
            &root.to_string_lossy(),
            0,
            0x8000, // SS_PATHELLIPSIS: preserve the end of a long folder path.
            false,
        )?,
        child(
            w!("EDIT"),
            "",
            QUERY,
            WS_BORDER.0 | ES_AUTOHSCROLL as u32,
            true,
        )?,
        child(
            w!("LISTBOX"),
            "",
            RESULTS,
            WS_BORDER.0 | WS_VSCROLL.0 | LBS_NOTIFY as u32 | LBS_NOINTEGRALHEIGHT as u32,
            true,
        )?,
        child(w!("STATIC"), strings.workspace(Text::Scanning), 0, 0, false)?,
        child(
            w!("BUTTON"),
            strings.workspace(Text::Refresh),
            REFRESH,
            BS_PUSHBUTTON as u32,
            true,
        )?,
        child(
            w!("BUTTON"),
            strings.cancel(),
            2,
            BS_PUSHBUTTON as u32,
            true,
        )?,
        child(
            w!("BUTTON"),
            strings.workspace(Text::Open),
            1,
            BS_DEFPUSHBUTTON as u32,
            true,
        )?,
    ];
    let mut state = Box::new(Picker {
        root: root.to_owned(),
        strings,
        locale,
        font,
        controls,
        scan: None,
        index: None,
        rows: Vec::new(),
        done: false,
        result: None,
    });
    if !unsafe { SetWindowSubclass(controls[1], Some(query_proc), 1, 0) }.as_bool() {
        return Err(error(windows::core::Error::from_win32()));
    }
    if !unsafe {
        SetWindowSubclass(
            window.0,
            Some(picker_proc),
            1,
            std::ptr::from_mut(state.as_mut()) as usize,
        )
    }
    .as_bool()
    {
        return Err(error(windows::core::Error::from_win32()));
    }
    let old_focus = unsafe { GetFocus() };
    let owner_guard = OwnerGuard(owner, old_focus);
    state.layout(window.0);
    state.refresh();
    unsafe {
        SendMessageW(controls[1], EM_SETLIMITTEXT, WPARAM(512), LPARAM(0));
        let cue = wide(strings.workspace(Text::Filter));
        SendMessageW(
            controls[1],
            0x1501,
            WPARAM(1),
            LPARAM(cue.as_ptr() as isize),
        );
        let _ = EnableWindow(owner, false);
        let _ = SetTimer(window.0, 1, 80, None);
        let _ = ShowWindow(window.0, SW_SHOW);
        let _ = SetFocus(controls[1]);
    }
    while !state.done {
        let mut message = MSG::default();
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result <= 0 {
            if result == 0 {
                unsafe {
                    PostQuitMessage(message.wParam.0 as i32);
                }
            }
            break;
        }
        if message.message == WM_KEYDOWN && !composing(controls[1]) {
            if message.wParam.0 == VK_ESCAPE.0 as usize {
                state.done = true;
                continue;
            }
            if [controls[1], controls[2]].contains(&message.hwnd) {
                if message.wParam.0 == VK_RETURN.0 as usize {
                    state.choose();
                    continue;
                }
                if message.hwnd == controls[1]
                    && [VK_UP.0 as usize, VK_DOWN.0 as usize].contains(&message.wParam.0)
                    && !state.rows.is_empty()
                {
                    let current =
                        unsafe { SendMessageW(controls[2], LB_GETCURSEL, WPARAM(0), LPARAM(0)) }.0;
                    let delta = if message.wParam.0 == VK_UP.0 as usize {
                        -1
                    } else {
                        1
                    };
                    let next = (current + delta).clamp(0, state.rows.len() as isize - 1);
                    unsafe {
                        SendMessageW(controls[2], LB_SETCURSEL, WPARAM(next as usize), LPARAM(0));
                    }
                    continue;
                }
            }
        }
        // Do not let dialog navigation steal Return/Escape from native IME.
        if composing(controls[1]) || !unsafe { IsDialogMessageW(window.0, &message) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    unsafe {
        let _ = KillTimer(window.0, 1);
        let _ = DestroyWindow(window.0);
    }
    drop(owner_guard);
    Ok(state.result.take())
}
