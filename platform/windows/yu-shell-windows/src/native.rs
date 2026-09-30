#![cfg(target_os = "windows")]

use std::ffi::{OsStr, OsString};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr;

use windows::Win32::Foundation::{
    BOOL, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, EndPaint, GetSysColorBrush, PAINTSTRUCT,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileOpenDialog,
    FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ACCEL, AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    CreateAcceleratorTableW, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyAcceleratorTable, DestroyWindow, DispatchMessageW, FCONTROL, FSHIFT, FVIRTKEY,
    GWLP_USERDATA, GetClientRect, GetMessageW, GetParent, GetWindowLongPtrW, HACCEL, HMENU,
    IDC_ARROW, LoadCursorW, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO, MB_YESNOCANCEL,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, MessageBoxW, MoveWindow, PostMessageW, PostQuitMessage,
    RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetMenu, SetWindowLongPtrW,
    SetWindowPos, SetWindowTextW, ShowWindow, TranslateAcceleratorW, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DPICHANGED, WM_NCCREATE,
    WM_PAINT, WM_SETTINGCHANGE, WM_SIZE, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{Error as WindowsError, PCWSTR, Result as WindowsResult, w};
use yu_editor::{EditorCommand, ViewportSpan};
use yu_font::{FontRequest, GlyphAtlasConfig};
use yu_font_windows::DirectWriteShaper;
use yu_render::SurfaceConfig;
use yu_render_windows::{D3DRenderError, D3DRenderer};
use yu_scene::Rect;
use yu_storage::{ClosePrompt, CloseRequest, CloseTransition};
use yu_workspace::{Appearance, ViewportFrameBuilder, ViewportRenderConfig};

use crate::{DocumentSlot, Locale, SaveAction, ShellError, ShellState, SidebarMode, WindowMetrics};

const MAIN_CLASS: PCWSTR = w!("YuEditorWindow");
const SURFACE_CLASS: PCWSTR = w!("YuEditorSurface");

const ID_FILE_NEW: u16 = 1001;
const ID_FILE_OPEN: u16 = 1002;
const ID_FILE_SAVE: u16 = 1003;
const ID_FILE_SAVE_AS: u16 = 1004;
const ID_FILE_EXIT: u16 = 1005;
const ID_EDIT_UNDO: u16 = 1101;
const ID_EDIT_REDO: u16 = 1102;
const ID_VIEW_SIDEBAR: u16 = 1201;
const WM_APP_RENDER: u32 = WM_APP + 1;
const BODY_FONT_SIZE: f32 = 16.0;

const CANCELLED_HRESULT: i32 = 0x8007_04c7_u32 as i32;

struct ComApartment;

impl ComApartment {
    fn initialize() -> WindowsResult<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct RenderHost {
    surface_hwnd: HWND,
    builder: ViewportFrameBuilder<DirectWriteShaper>,
    renderer: D3DRenderer,
}

impl RenderHost {
    fn new(surface_hwnd: HWND, appearance: Appearance) -> Result<Self, ShellError> {
        let surface = native_surface_config(surface_hwnd)?;
        let config = viewport_render_config(surface, appearance)?;
        let font = FontRequest::new("Segoe UI", BODY_FONT_SIZE)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        let shaper = DirectWriteShaper::new(font)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        let builder =
            ViewportFrameBuilder::with_shaper(shaper, config, GlyphAtlasConfig::default())
                .map_err(|error| ShellError::Platform(error.to_string()))?;
        let renderer = D3DRenderer::new(surface_hwnd, surface)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        Ok(Self {
            surface_hwnd,
            builder,
            renderer,
        })
    }

    fn sync_surface(
        &mut self,
        surface_hwnd: HWND,
        appearance: Appearance,
    ) -> Result<(), ShellError> {
        let surface = native_surface_config(surface_hwnd)?;
        if (surface.scale() - self.renderer.surface().scale()).abs() > f64::EPSILON {
            *self = Self::new(surface_hwnd, appearance)?;
            return Ok(());
        }
        self.renderer
            .resize(surface)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        self.builder
            .update_config(viewport_render_config(surface, appearance)?)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        Ok(())
    }

    fn render(&mut self, state: &mut ShellState) -> Result<(), ShellError> {
        let revision = state.document().session().revision();
        let snapshot = state
            .document()
            .session()
            .document()
            .editor()
            .capture_render_snapshot();
        let mut layout = snapshot.into_layout_context();
        let publication = self
            .builder
            .publish(&mut layout)
            .map_err(|error| ShellError::Platform(error.to_string()))?;

        match self.renderer.render_viewport_frame(
            revision,
            publication.frame(),
            self.builder.atlas(),
        ) {
            Ok(()) => Ok(()),
            Err(D3DRenderError::DeviceLost) => {
                let surface = self.renderer.surface();
                self.renderer = D3DRenderer::new(self.surface_hwnd, surface)
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                self.renderer
                    .render_viewport_frame(revision, publication.frame(), self.builder.atlas())
                    .map_err(|error| ShellError::Platform(error.to_string()))
            }
            Err(error) => Err(ShellError::Platform(error.to_string())),
        }
    }
}

struct AppWindow {
    hwnd: HWND,
    sidebar: HWND,
    surface: HWND,
    status: HWND,
    state: ShellState,
    render: Option<RenderHost>,
}

impl AppWindow {
    fn new(state: ShellState) -> Self {
        Self {
            hwnd: HWND::default(),
            sidebar: HWND::default(),
            surface: HWND::default(),
            status: HWND::default(),
            state,
            render: None,
        }
    }

    fn initialize(&mut self, hwnd: HWND) -> Result<(), ShellError> {
        self.hwnd = hwnd;
        self.apply_system_theme();
        self.install_menu()?;
        self.create_children()?;
        self.refresh_chrome();
        self.update_layout();
        self.render = Some(RenderHost::new(self.surface, self.state.appearance())?);
        self.render_current()?;
        Ok(())
    }

    fn render_current(&mut self) -> Result<(), ShellError> {
        let Some(mut render) = self.render.take() else {
            return Ok(());
        };
        render.sync_surface(self.surface, self.state.appearance())?;
        let result = render.render(&mut self.state);
        self.render = Some(render);
        result
    }

    fn install_menu(&self) -> Result<(), ShellError> {
        let strings = self.state.strings();
        unsafe {
            let root = CreateMenu().map_err(platform_error)?;
            let file = CreatePopupMenu().map_err(platform_error)?;
            append_string(file, ID_FILE_NEW, strings.new_document())?;
            append_string(file, ID_FILE_OPEN, strings.open())?;
            AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null()).map_err(platform_error)?;
            append_string(file, ID_FILE_SAVE, strings.save())?;
            append_string(file, ID_FILE_SAVE_AS, strings.save_as())?;
            AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null()).map_err(platform_error)?;
            append_string(file, ID_FILE_EXIT, strings.exit())?;
            append_popup(root, file, strings.file())?;

            let edit = CreatePopupMenu().map_err(platform_error)?;
            append_string(edit, ID_EDIT_UNDO, strings.undo())?;
            append_string(edit, ID_EDIT_REDO, strings.redo())?;
            append_popup(root, edit, strings.edit())?;

            let view = CreatePopupMenu().map_err(platform_error)?;
            append_string(view, ID_VIEW_SIDEBAR, strings.toggle_sidebar())?;
            append_popup(root, view, strings.view())?;

            SetMenu(self.hwnd, root).map_err(platform_error)?;
        }
        Ok(())
    }

    fn create_children(&mut self) -> Result<(), ShellError> {
        let strings = self.state.strings();
        let sidebar_text = format!(
            "{}\r\n\r\n{}\r\n\r\n{}",
            strings.files(),
            strings.outline(),
            strings.search()
        );
        let status_text = strings.ready().to_owned();

        unsafe {
            self.sidebar = create_child_static(self.hwnd, &sidebar_text)?;
            self.surface = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                SURFACE_CLASS,
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                self.hwnd,
                None,
                None,
                None,
            )
            .map_err(platform_error)?;
            self.status = create_child_static(self.hwnd, &status_text)?;
        }
        Ok(())
    }

    fn refresh_chrome(&self) {
        set_window_text(self.hwnd, &self.state.window_title());
        let scale = (self.state.metrics().scale() * 100.0).round() as u32;
        let status = format!(
            "{} · {scale}% · UTF-8 · Markdown",
            self.state.strings().ready()
        );
        set_window_text(self.status, &status);
    }

    fn update_layout(&mut self) {
        let mut rect = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rect) }.is_err() {
            return;
        }
        let width = (rect.right - rect.left).max(1);
        let height = (rect.bottom - rect.top).max(1);
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        self.state
            .set_metrics(WindowMetrics::new(width as u32, height as u32, dpi));

        let status_h = self.state.metrics().px(24.0).clamp(20, height);
        let content_h = (height - status_h).max(1);
        let sidebar_w = if self.state.sidebar() == SidebarMode::Hidden {
            0
        } else {
            self.state.metrics().px(240.0).clamp(0, (width / 2).max(0))
        };
        let surface_w = (width - sidebar_w).max(1);

        unsafe {
            let _ = MoveWindow(self.sidebar, 0, 0, sidebar_w, content_h, true);
            let _ = MoveWindow(self.surface, sidebar_w, 0, surface_w, content_h, true);
            let _ = MoveWindow(self.status, 0, content_h, width, status_h, true);
        }
        self.refresh_chrome();
    }

    fn apply_system_theme(&mut self) {
        let dark = system_prefers_dark();
        self.state.set_appearance(if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        });
        unsafe {
            let dark_value = BOOL::from(dark);
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                ptr::from_ref(&dark_value).cast(),
                size_of::<BOOL>() as u32,
            );
            let backdrop = DWMSBT_MAINWINDOW;
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                ptr::from_ref(&backdrop).cast(),
                size_of_val(&backdrop) as u32,
            );
        }
    }

    fn handle_command(&mut self, command: u16) -> Result<(), ShellError> {
        match command {
            ID_FILE_NEW => {
                if self.confirm_replace_current()? {
                    self.state.new_document();
                    self.refresh_chrome();
                }
            }
            ID_FILE_OPEN => {
                if let Some(path) = open_file_dialog(self.hwnd)? {
                    let candidate = DocumentSlot::open(path)?;
                    if self.confirm_replace_current()? {
                        self.state.replace_document(candidate);
                        self.refresh_chrome();
                    }
                }
            }
            ID_FILE_SAVE => {
                self.save_current(false)?;
            }
            ID_FILE_SAVE_AS => {
                self.save_current(true)?;
            }
            ID_FILE_EXIT => unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    self.hwnd,
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            },
            ID_EDIT_UNDO => {
                if self
                    .state
                    .document()
                    .session()
                    .command_available(&EditorCommand::Undo)
                {
                    self.state
                        .document_mut()
                        .session_mut()
                        .execute(EditorCommand::Undo)?;
                    self.refresh_chrome();
                }
            }
            ID_EDIT_REDO => {
                if self
                    .state
                    .document()
                    .session()
                    .command_available(&EditorCommand::Redo)
                {
                    self.state
                        .document_mut()
                        .session_mut()
                        .execute(EditorCommand::Redo)?;
                    self.refresh_chrome();
                }
            }
            ID_VIEW_SIDEBAR => {
                self.state.toggle_sidebar();
                self.update_layout();
            }
            _ => {}
        }
        if command != ID_FILE_EXIT {
            self.render_current()?;
        }
        Ok(())
    }

    fn save_current(&mut self, force_save_as: bool) -> Result<(), ShellError> {
        if force_save_as {
            if let Some(path) = save_file_dialog(self.hwnd, self.state.document())? {
                self.state.document_mut().save_as(path, true)?;
            }
        } else {
            match self.state.document_mut().save()? {
                SaveAction::Saved(_) => {}
                SaveAction::NeedsDestination => {
                    if let Some(path) = save_file_dialog(self.hwnd, self.state.document())? {
                        self.state.document_mut().save_as(path, true)?;
                    }
                }
            }
        }
        self.refresh_chrome();
        Ok(())
    }

    fn confirm_replace_current(&mut self) -> Result<bool, ShellError> {
        match self.state.document_mut().close_request()? {
            CloseRequest::CloseNow => Ok(true),
            CloseRequest::AlreadyClosed => Ok(true),
            CloseRequest::Prompt(ClosePrompt::SaveChanges) => self.confirm_save_changes(),
            CloseRequest::Prompt(ClosePrompt::ExternalChange { .. }) => {
                self.confirm_external_change()
            }
        }
    }

    fn confirm_save_changes(&mut self) -> Result<bool, ShellError> {
        let strings = self.state.strings();
        let result = message_box(
            self.hwnd,
            strings.save_changes_question(),
            strings.app_name(),
            MB_YESNOCANCEL | MB_ICONWARNING,
        );
        use windows::Win32::UI::WindowsAndMessaging::{IDNO, IDYES};
        if result == IDYES {
            let transition = if self.state.document().is_untitled() {
                let Some(path) = save_file_dialog(self.hwnd, self.state.document())? else {
                    let _ = self.state.document_mut().cancel_close();
                    return Ok(false);
                };
                self.state.document_mut().save_as_close(path, true)?
            } else {
                self.state.document_mut().save_close()?
            };
            Ok(matches!(transition, CloseTransition::Closed))
        } else if result == IDNO {
            Ok(matches!(
                self.state.document_mut().discard_close()?,
                CloseTransition::Closed
            ))
        } else {
            let _ = self.state.document_mut().cancel_close();
            Ok(false)
        }
    }

    fn confirm_external_change(&mut self) -> Result<bool, ShellError> {
        let strings = self.state.strings();
        let result = message_box(
            self.hwnd,
            strings.external_change_question(),
            strings.app_name(),
            MB_YESNO | MB_ICONWARNING,
        );
        use windows::Win32::UI::WindowsAndMessaging::IDYES;
        if result == IDYES {
            Ok(matches!(
                self.state.document_mut().discard_close()?,
                CloseTransition::Closed
            ))
        } else {
            let _ = self.state.document_mut().cancel_close();
            Ok(false)
        }
    }
}

pub fn run() -> Result<(), ShellError> {
    let _com = ComApartment::initialize().map_err(platform_error)?;
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    register_classes()?;

    let locale = current_locale();
    let argument = std::env::args_os().nth(1);
    let (state, auto_close, cleanup) =
        if argument.as_deref() == Some(OsStr::new("--window-self-check")) {
            let path = std::env::temp_dir()
                .join(format!("yu-windows-shell-smoke-{}.md", std::process::id()));
            std::fs::write(&path, "# Yu Windows shell smoke\n")
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            (ShellState::from_path(locale, &path)?, true, Some(path))
        } else {
            let state = match argument {
                Some(path) => ShellState::from_path(locale, Path::new(&path))?,
                None => ShellState::new(locale),
            };
            (state, false, None)
        };
    let result = run_window(state, auto_close);
    if let Some(path) = cleanup {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn run_window(state: ShellState, auto_close: bool) -> Result<(), ShellError> {
    let title = wide(&state.window_title());
    let app = Box::new(AppWindow::new(state));
    let app_ptr = Box::into_raw(app);

    let instance = unsafe { GetModuleHandleW(None) }.map_err(platform_error)?;
    let hwnd_result = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            MAIN_CLASS,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1200,
            800,
            None,
            None,
            HINSTANCE(instance.0),
            Some(app_ptr.cast()),
        )
    };

    let hwnd = match hwnd_result {
        Ok(hwnd) => hwnd,
        Err(error) => {
            unsafe {
                drop(Box::from_raw(app_ptr));
            }
            return Err(platform_error(error));
        }
    };

    let initialization = unsafe { (&mut *app_ptr).initialize(hwnd) };
    if let Err(error) = initialization {
        unsafe {
            let _ = DestroyWindow(hwnd);
            drop(Box::from_raw(app_ptr));
        }
        return Err(error);
    }

    let accelerator = create_accelerators()?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        if auto_close {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                hwnd,
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
    message_loop(hwnd, accelerator);
    unsafe {
        let _ = DestroyAcceleratorTable(accelerator);
        drop(Box::from_raw(app_ptr));
    }
    Ok(())
}

fn register_classes() -> Result<(), ShellError> {
    let instance = unsafe { GetModuleHandleW(None) }.map_err(platform_error)?;
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.map_err(platform_error)?;
    let main = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: HINSTANCE(instance.0),
        hCursor: cursor,
        hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
        lpszClassName: MAIN_CLASS,
        ..Default::default()
    };
    let surface = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(surface_proc),
        hInstance: HINSTANCE(instance.0),
        hCursor: cursor,
        // The D3D swapchain owns every editor pixel. A class brush here would
        // briefly erase the retained backbuffer during WM_PAINT and create a
        // second visual path/flicker.
        hbrBackground: Default::default(),
        lpszClassName: SURFACE_CLASS,
        ..Default::default()
    };
    unsafe {
        if RegisterClassExW(&main) == 0 {
            return Err(platform_error(WindowsError::from_win32()));
        }
        if RegisterClassExW(&surface) == 0 {
            return Err(platform_error(WindowsError::from_win32()));
        }
    }
    Ok(())
}

fn create_accelerators() -> Result<HACCEL, ShellError> {
    let entries = [
        ACCEL {
            fVirt: FCONTROL | FVIRTKEY,
            key: b'N' as u16,
            cmd: ID_FILE_NEW,
        },
        ACCEL {
            fVirt: FCONTROL | FVIRTKEY,
            key: b'O' as u16,
            cmd: ID_FILE_OPEN,
        },
        ACCEL {
            fVirt: FCONTROL | FVIRTKEY,
            key: b'S' as u16,
            cmd: ID_FILE_SAVE,
        },
        ACCEL {
            fVirt: FCONTROL | FSHIFT | FVIRTKEY,
            key: b'S' as u16,
            cmd: ID_FILE_SAVE_AS,
        },
        ACCEL {
            fVirt: FCONTROL | FVIRTKEY,
            key: b'Z' as u16,
            cmd: ID_EDIT_UNDO,
        },
        ACCEL {
            fVirt: FCONTROL | FVIRTKEY,
            key: b'Y' as u16,
            cmd: ID_EDIT_REDO,
        },
        ACCEL {
            fVirt: FCONTROL | FSHIFT | FVIRTKEY,
            key: b'L' as u16,
            cmd: ID_VIEW_SIDEBAR,
        },
    ];
    unsafe { CreateAcceleratorTableW(&entries) }.map_err(platform_error)
}

fn message_loop(hwnd: HWND, accelerator: HACCEL) {
    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        if unsafe { TranslateAcceleratorW(hwnd, accelerator, &message) } == 0 {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        let app_ptr = create.lpCreateParams.cast::<AppWindow>();
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
            (*app_ptr).hwnd = hwnd;
        }
        return LRESULT(1);
    }

    let app_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppWindow;
    if !app_ptr.is_null() {
        let app = unsafe { &mut *app_ptr };
        match message {
            WM_COMMAND => {
                let command = (wparam.0 & 0xffff) as u16;
                if let Err(error) = app.handle_command(command) {
                    show_error(hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_SIZE => {
                app.update_layout();
                if let Err(error) = app.render_current() {
                    show_error(hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let suggested = unsafe { &*(lparam.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                app.update_layout();
                if let Err(error) = app.render_current() {
                    show_error(hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_SETTINGCHANGE => {
                app.apply_system_theme();
                app.refresh_chrome();
                if let Err(error) = app.render_current() {
                    show_error(hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_APP_RENDER => {
                if let Err(error) = app.render_current() {
                    show_error(hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_CLOSE => {
                match app.confirm_replace_current() {
                    Ok(true) => unsafe {
                        let _ = DestroyWindow(hwnd);
                    },
                    Ok(false) => {}
                    Err(error) => show_error(hwnd, &app.state, &error),
                }
                return LRESULT(0);
            }
            WM_DESTROY => {
                unsafe {
                    PostQuitMessage(0);
                }
                return LRESULT(0);
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn surface_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        unsafe {
            BeginPaint(hwnd, &mut paint);
            let _ = EndPaint(hwnd, &paint);
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(parent, WM_APP_RENDER, WPARAM(0), LPARAM(0));
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn native_surface_config(hwnd: HWND) -> Result<SurfaceConfig, ShellError> {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.map_err(platform_error)?;
    let width_px = (rect.right - rect.left).max(1) as u32;
    let height_px = (rect.bottom - rect.top).max(1) as u32;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let scale = f64::from(dpi) / 96.0;
    SurfaceConfig::new(
        f64::from(width_px) / scale,
        f64::from(height_px) / scale,
        scale,
    )
    .map_err(|error| ShellError::Platform(error.to_string()))
}

fn viewport_render_config(
    surface: SurfaceConfig,
    appearance: Appearance,
) -> Result<ViewportRenderConfig, ShellError> {
    let width = surface.logical_width() as f32;
    let height = surface.logical_height() as f32;
    let scene = Rect::new(0.0, 0.0, width, height)
        .map_err(|error| ShellError::Platform(error.to_string()))?;
    Ok(ViewportRenderConfig::new(
        ViewportSpan::new(0.0, height),
        BODY_FONT_SIZE,
        scene,
        appearance.text(),
    )
    .with_background(appearance.background())
    .with_raster_scale(surface.scale() as f32)
    .with_appearance(appearance))
}

unsafe fn create_child_static(parent: HWND, text: &str) -> Result<HWND, ShellError> {
    let text = wide(text);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            1,
            1,
            parent,
            None,
            None,
            None,
        )
    }
    .map_err(platform_error)
}

unsafe fn append_string(menu: HMENU, id: u16, text: &str) -> Result<(), ShellError> {
    let text = wide(text);
    unsafe { AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(text.as_ptr())) }
        .map_err(platform_error)
}

unsafe fn append_popup(root: HMENU, popup: HMENU, text: &str) -> Result<(), ShellError> {
    let text = wide(text);
    unsafe { AppendMenuW(root, MF_POPUP, popup.0 as usize, PCWSTR(text.as_ptr())) }
        .map_err(platform_error)
}

fn open_file_dialog(owner: HWND) -> Result<Option<PathBuf>, ShellError> {
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(platform_error)?;
    unsafe {
        let options = dialog.GetOptions().map_err(platform_error)?
            | FOS_FORCEFILESYSTEM
            | FOS_FILEMUSTEXIST
            | FOS_PATHMUSTEXIST;
        dialog.SetOptions(options).map_err(platform_error)?;
        set_markdown_filter(&dialog)?;
    }
    show_file_dialog(owner, &dialog)
}

fn save_file_dialog(owner: HWND, document: &DocumentSlot) -> Result<Option<PathBuf>, ShellError> {
    let dialog: IFileSaveDialog =
        unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(platform_error)?;
    let name = if document.is_untitled() {
        "Untitled.md".to_owned()
    } else {
        document
            .session()
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled.md")
            .to_owned()
    };
    let name_wide = wide(&name);
    unsafe {
        let options = dialog.GetOptions().map_err(platform_error)?
            | FOS_FORCEFILESYSTEM
            | FOS_PATHMUSTEXIST
            | FOS_OVERWRITEPROMPT;
        dialog.SetOptions(options).map_err(platform_error)?;
        set_markdown_filter(&dialog)?;
        dialog
            .SetDefaultExtension(w!("md"))
            .map_err(platform_error)?;
        dialog
            .SetFileName(PCWSTR(name_wide.as_ptr()))
            .map_err(platform_error)?;
    }
    show_file_dialog(owner, &dialog)
}

unsafe fn set_markdown_filter<D>(dialog: &D) -> Result<(), ShellError>
where
    D: windows::core::Interface,
{
    use windows::Win32::UI::Shell::IFileDialog;
    let dialog: IFileDialog = dialog.cast().map_err(platform_error)?;
    let name = wide("Markdown (*.md;*.markdown)");
    let pattern = wide("*.md;*.markdown");
    let all_name = wide("All files (*.*)");
    let all_pattern = wide("*.*");
    let filters = [
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(pattern.as_ptr()),
        },
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(all_name.as_ptr()),
            pszSpec: PCWSTR(all_pattern.as_ptr()),
        },
    ];
    unsafe { dialog.SetFileTypes(&filters) }.map_err(platform_error)
}

fn show_file_dialog<D>(owner: HWND, dialog: &D) -> Result<Option<PathBuf>, ShellError>
where
    D: windows::core::Interface,
{
    use windows::Win32::UI::Shell::{IFileDialog, IModalWindow};
    let modal: IModalWindow = dialog.cast().map_err(platform_error)?;
    match unsafe { modal.Show(owner) } {
        Ok(()) => {}
        Err(error) if error.code().0 == CANCELLED_HRESULT => return Ok(None),
        Err(error) => return Err(platform_error(error)),
    }
    let file_dialog: IFileDialog = dialog.cast().map_err(platform_error)?;
    let item = unsafe { file_dialog.GetResult() }.map_err(platform_error)?;
    let raw = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(platform_error)?;
    if raw.is_null() {
        return Ok(None);
    }
    let path = unsafe {
        let value = OsString::from_wide(raw.as_wide());
        CoTaskMemFree(Some(raw.as_ptr().cast()));
        PathBuf::from(value)
    };
    Ok(Some(path))
}

fn current_locale() -> Locale {
    let mut buffer = [0_u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if len <= 1 {
        return Locale::English;
    }
    let end = usize::try_from(len - 1).unwrap_or(0).min(buffer.len());
    let tag = String::from_utf16_lossy(&buffer[..end]);
    Locale::from_language_tag(&tag)
}

fn system_prefers_dark() -> bool {
    let mut value = 1_u32;
    let mut bytes = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(ptr::from_mut(&mut value).cast()),
            Some(&mut bytes),
        )
    };
    result == ERROR_SUCCESS && value == 0
}

fn show_error(hwnd: HWND, state: &ShellState, error: &ShellError) {
    let strings = state.strings();
    let message = error.to_string();
    let _ = message_box(hwnd, &message, strings.error_title(), MB_ICONERROR | MB_OK);
}

fn message_box(
    hwnd: HWND,
    text: &str,
    title: &str,
    style: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE,
) -> windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_RESULT {
    let text = wide(text);
    let title = wide(title);
    unsafe { MessageBoxW(hwnd, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), style) }
}

fn set_window_text(hwnd: HWND, text: &str) {
    if hwnd.is_invalid() {
        return;
    }
    let text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn platform_error(error: WindowsError) -> ShellError {
    ShellError::Platform(error.to_string())
}
