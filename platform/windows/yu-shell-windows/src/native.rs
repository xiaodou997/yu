#![cfg(target_os = "windows")]

use std::ffi::{OsStr, OsString};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    BOOL, ERROR_SUCCESS, GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
    RECT, WPARAM,
};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, ClientToScreen, EndPaint, GetSysColorBrush, PAINTSTRUCT,
    ScreenToClient,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MK_SHIFT};
use windows::Win32::UI::Accessibility::{UiaRect, UiaReturnRawElementProvider};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE,
    VK_DOWN, VK_END, VK_F6, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileOpenDialog,
    FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::TextServices::TS_TEXTCHANGE;
use windows::Win32::UI::WindowsAndMessaging::{
    ACCEL, AppendMenuW, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    CreateAcceleratorTableW, CreateMenu, CreatePopupMenu, CreateWindowExW, DLGC_WANTALLKEYS,
    DLGC_WANTARROWS, DLGC_WANTCHARS, DLGC_WANTTAB, DefWindowProcW, DestroyAcceleratorTable,
    DestroyWindow, DispatchMessageW, EN_CHANGE, FCONTROL, FSHIFT, FVIRTKEY, GWLP_USERDATA,
    GetClientRect, GetMessageW, GetParent, GetWindowLongPtrW, HACCEL, HMENU, HWND_TOP, IDC_ARROW,
    IsDialogMessageW, KillTimer, LBN_DBLCLK, LBN_SELCHANGE, LoadCursorW, MB_ICONERROR,
    MB_ICONWARNING, MB_OK, MB_YESNO, MB_YESNOCANCEL, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG,
    MessageBoxW, MoveWindow, PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetMenu, SetTimer, SetWindowLongPtrW,
    SetWindowPos, SetWindowTextW, ShowWindow, TranslateAcceleratorW, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX,
    WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_GETDLGCODE, WM_GETOBJECT, WM_KEYDOWN, WM_KEYUP,
    WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MEASUREITEM, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_MOVE, WM_NCCREATE, WM_PAINT, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE,
    WM_SYSCOLORCHANGE, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_THEMECHANGED, WM_TIMER, WNDCLASSEXW,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{Error as WindowsError, PCWSTR, Result as WindowsResult, w};
use yu_core::{ByteOffset, CaretAffinity, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    Bias, EditorCommand, EditorKey, EditorSelection, KeyEvent, KeyModifiers, KeyRouteResult,
    LayoutConfig, LayoutPoint, LayoutSnapshot, ViewportConfig, ViewportSpan,
};
use yu_font::{FontRequest, GlyphAtlasConfig};
use yu_font_windows::DirectWriteShaper;
use yu_render::SurfaceConfig;
use yu_render_windows::{D3DRenderError, D3DRenderer};
use yu_scene::Rect;
use yu_storage::{ClosePrompt, CloseRequest, CloseTransition};
use yu_workspace::{Appearance, ViewportFrameBuilder, ViewportRenderConfig};

use crate::accessibility::{
    AccessibilityHost, Action as AccessibilityAction, Snapshot as AccessibilitySnapshot,
    WM_APP_UIA_ACTION,
};
use crate::chrome::{
    Chrome, ID_FILES, ID_OUTLINE, ID_QUERY, ID_ROWS, ID_SEARCH, PanelAction, sidebar_width,
};
use crate::resources::ResourceHost;
use crate::text_input::{
    AcpProjection, AcpRange, AcpSelection, canonical_acp_range_to_source, local_selection_utf16,
    replace_local_utf16, selection_from_acp,
};
use crate::tsf::{TsfHost, WM_APP_TSF_LOCK};
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
const ID_FOCUS_NEXT: u16 = 1202;
const ID_FOCUS_PREVIOUS: u16 = 1203;
const WM_APP_RENDER: u32 = WM_APP + 1;
const BODY_FONT_SIZE: f32 = 16.0;
const DRAG_SCROLL_TIMER_ID: usize = 1;
const RESOURCE_TIMER_ID: usize = 2;
const DRAG_SCROLL_TIMER_MS: u32 = 30;
const TRIPLE_CLICK_WINDOW: Duration = Duration::from_millis(600);
const TRIPLE_CLICK_RADIUS_PX: i32 = 8;

const CANCELLED_HRESULT: i32 = 0x8007_04c7_u32 as i32;
const CF_UNICODETEXT_FORMAT: u32 = 13;

pub(super) struct ComApartment;

impl ComApartment {
    pub(super) fn initialize() -> WindowsResult<Self> {
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

struct ClipboardGuard;

impl ClipboardGuard {
    fn open(owner: HWND) -> Result<Self, ShellError> {
        unsafe { OpenClipboard(owner) }.map_err(platform_error)?;
        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn write_unicode_clipboard(owner: HWND, text: &str) -> Result<(), ShellError> {
    let _guard = ClipboardGuard::open(owner)?;
    unsafe { EmptyClipboard() }.map_err(platform_error)?;
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| ShellError::Platform("clipboard text is too large".to_owned()))?;
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }.map_err(platform_error)?;
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        unsafe {
            let _ = GlobalFree(memory);
        }
        return Err(platform_error(WindowsError::from_win32()));
    }
    unsafe {
        ptr::copy_nonoverlapping(units.as_ptr(), pointer.cast::<u16>(), units.len());
        let _ = GlobalUnlock(memory);
    }
    match unsafe { SetClipboardData(CF_UNICODETEXT_FORMAT, HANDLE(memory.0)) } {
        Ok(_) => Ok(()),
        Err(error) => {
            unsafe {
                let _ = GlobalFree(memory);
            }
            Err(platform_error(error))
        }
    }
}

fn read_unicode_clipboard(owner: HWND) -> Result<String, ShellError> {
    let _guard = ClipboardGuard::open(owner)?;
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT_FORMAT) }.map_err(platform_error)?;
    let memory = HGLOBAL(handle.0);
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        return Err(platform_error(WindowsError::from_win32()));
    }
    let units = unsafe { GlobalSize(memory) } / size_of::<u16>();
    let source = unsafe { std::slice::from_raw_parts(pointer.cast::<u16>(), units) };
    let end = source
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(source.len());
    let text =
        String::from_utf16(&source[..end]).map_err(|error| ShellError::Platform(error.to_string()));
    unsafe {
        let _ = GlobalUnlock(memory);
    }
    text
}

struct RenderHost {
    contrast: Option<yu_scene::ContrastPalette>,
    surface_hwnd: HWND,
    builder: ViewportFrameBuilder<DirectWriteShaper>,
    renderer: D3DRenderer,
    layout: Option<Arc<LayoutSnapshot>>,
    resources: Option<ResourceHost>,
    document_identity: Option<u64>,
}

impl RenderHost {
    fn new(surface_hwnd: HWND, appearance: Appearance) -> Result<Self, ShellError> {
        let surface = native_surface_config(surface_hwnd)?;
        let config = viewport_render_config(surface, appearance, 0.0)?;
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
            contrast: None,
            builder,
            renderer,
            layout: None,
            resources: None,
            document_identity: None,
        })
    }

    fn sync_surface(
        &mut self,
        surface_hwnd: HWND,
        appearance: Appearance,
    ) -> Result<(), ShellError> {
        let surface = native_surface_config(surface_hwnd)?;
        let old_viewport = self.builder.config().viewport();
        let content_height = self
            .layout
            .as_ref()
            .map(|layout| layout.content_height())
            .unwrap_or(old_viewport.height());
        let max_scroll = (content_height - surface.logical_height() as f32).max(0.0);
        let scroll_y = old_viewport.scroll_y().min(max_scroll);
        if (surface.scale() - self.renderer.surface().scale()).abs() > f64::EPSILON {
            self.renderer
                .resize(surface)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            let font = FontRequest::new("Segoe UI", BODY_FONT_SIZE)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            let shaper = DirectWriteShaper::new(font)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            self.builder = ViewportFrameBuilder::with_shaper(
                shaper,
                viewport_render_config(surface, appearance, scroll_y)?,
                GlyphAtlasConfig::default(),
            )
            .map_err(|error| ShellError::Platform(error.to_string()))?;
            self.layout = None;
            self.resources = None;
            return Ok(());
        }
        self.renderer
            .resize(surface)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        self.builder
            .update_config(viewport_render_config(surface, appearance, scroll_y)?)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        Ok(())
    }

    fn scroll_by(&mut self, delta: f32) -> Result<bool, ShellError> {
        if !delta.is_finite() || delta == 0.0 {
            return Ok(false);
        }
        let config = self.builder.config();
        let viewport = config.viewport();
        let content_height = self
            .layout
            .as_ref()
            .map(|layout| layout.content_height())
            .unwrap_or(viewport.height());
        let max_scroll = (content_height - viewport.height()).max(0.0);
        let target = (viewport.scroll_y() + delta).clamp(0.0, max_scroll);
        if (target - viewport.scroll_y()).abs() <= f32::EPSILON {
            return Ok(false);
        }
        let surface = self.renderer.surface();
        self.builder
            .update_config(viewport_render_config(
                surface,
                config.appearance(),
                target,
            )?)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        Ok(true)
    }

    fn render(&mut self, state: &mut ShellState) -> Result<(), ShellError> {
        let session = state.document_mut().session_mut();
        let viewport_config = editor_viewport_config(
            self.builder.config().scene_viewport().width(),
            self.builder.config().appearance(),
        );
        if session.viewport_config() != viewport_config {
            session.set_viewport_config(viewport_config)?;
        }
        let revision = state.document().session().revision();
        let identity = state.document().identity();
        let path = state.document().session().path().to_path_buf();
        let appearance = self.builder.config().appearance();
        let contrast = self.contrast;
        if self.document_identity.is_some_and(|old| old != identity) {
            self.renderer.reset_document();
            self.builder
                .reset_document()
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            self.layout = None;
            self.builder
                .update_config(viewport_render_config(
                    self.renderer.surface(),
                    appearance,
                    0.0,
                )?)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
        }
        self.document_identity = Some(identity);
        self.builder
            .update_config(self.builder.config().with_contrast_palette(contrast))
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        if self.resources.as_ref().is_none_or(|resources| {
            resources.identity != identity
                || resources.path != path
                || resources.appearance != appearance
                || resources.contrast != contrast
        }) {
            self.resources = Some(ResourceHost::new(
                identity,
                path,
                appearance,
                self.builder.config().raster_scale(),
            )?);
            if let Some(resources) = self.resources.as_mut() {
                resources.contrast = contrast;
            }
        }
        let resources = self
            .resources
            .as_mut()
            .ok_or_else(|| ShellError::Platform("Missing resource host".into()))?;
        let (images, intrinsics) = resources.prepare(
            state
                .document_mut()
                .session_mut()
                .document_mut()
                .editor_mut(),
            &self.builder,
        )?;
        self.builder
            .set_embedded_publications(resources.embedded_publications());
        let snapshot = state
            .document()
            .session()
            .document()
            .editor()
            .capture_render_snapshot();
        let mut layout = snapshot.into_layout_context();
        let publication = self
            .builder
            .publish_with_images_and_intrinsics(&mut layout, &images, &intrinsics)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        self.layout = Some(publication.layout_snapshot());
        if !state
            .document_mut()
            .session_mut()
            .document_mut()
            .editor_mut()
            .adopt_layout_snapshot(publication.layout_snapshot())
        {
            return Err(ShellError::Platform(
                "rendered layout does not match the current editor state".into(),
            ));
        }

        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!("yu-windows-resources stats={:?}", resources.diagnostics());
        }
        let presented = resources
            .upload(&mut self.renderer, &images)
            .and_then(|()| {
                self.renderer.render_viewport_frame(
                    revision,
                    publication.frame(),
                    self.builder.atlas(),
                )
            });
        match presented {
            Ok(()) => Ok(()),
            Err(D3DRenderError::DeviceLost) => {
                self.renderer
                    .recreate(self.surface_hwnd)
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                resources
                    .upload(&mut self.renderer, &images)
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                self.renderer
                    .render_viewport_frame(revision, publication.frame(), self.builder.atlas())
                    .map_err(|error| ShellError::Platform(error.to_string()))
            }
            Err(error) => Err(ShellError::Platform(error.to_string())),
        }
    }
}

pub(crate) struct AppWindow {
    editor_focused: bool,
    contrast: Option<yu_scene::ContrastPalette>,
    hwnd: HWND,
    sidebar: HWND,
    surface: HWND,
    status: HWND,
    chrome: Option<Chrome>,
    state: ShellState,
    render: Option<RenderHost>,
    tsf: Option<TsfHost>,
    accessibility: Option<AccessibilityHost>,
    drag_anchor: Option<ByteOffset>,
    drag_point: Option<(i32, i32)>,
    last_double_click: Option<(Instant, i32, i32)>,
    semantic_click: bool,
    pending_high_surrogate: Option<u16>,
}

impl AppWindow {
    fn new(state: ShellState) -> Self {
        Self {
            hwnd: HWND::default(),
            editor_focused: false,
            contrast: None,
            sidebar: HWND::default(),
            surface: HWND::default(),
            status: HWND::default(),
            chrome: None,
            state,
            render: None,
            tsf: None,
            accessibility: None,
            drag_anchor: None,
            drag_point: None,
            last_double_click: None,
            semantic_click: false,
            pending_high_surrogate: None,
        }
    }

    fn initialize(&mut self, hwnd: HWND) -> Result<(), ShellError> {
        self.hwnd = hwnd;
        self.apply_system_theme();
        self.install_menu()
            .map_err(|error| startup_error("install menu", error))?;
        self.create_children()
            .map_err(|error| startup_error("create child windows", error))?;
        self.accessibility = Some(AccessibilityHost::new(self.surface));
        self.refresh_chrome();
        self.update_layout();
        self.render = Some(
            RenderHost::new(self.surface, self.state.appearance())
                .map_err(|error| startup_error("create renderer", error))?,
        );
        self.render_current()
            .map_err(|error| startup_error("render first frame", error))?;
        let app = self as *mut Self;
        self.tsf = Some(
            TsfHost::new(app, self.surface)
                .map_err(|error| startup_error("initialize TSF", error))?,
        );
        unsafe {
            let _ = SetFocus(self.surface);
        }
        Ok(())
    }

    fn render_current(&mut self) -> Result<(), ShellError> {
        if let Some(chrome) = self.chrome.as_mut() {
            chrome.set_contrast_palette(self.contrast);
            chrome.refresh(&mut self.state)?;
        }
        let Some(mut render) = self.render.take() else {
            return Ok(());
        };
        render.sync_surface(self.surface, self.state.appearance())?;
        render.contrast = self.contrast;
        let result = render.render(&mut self.state);
        unsafe {
            if render
                .resources
                .as_ref()
                .is_some_and(ResourceHost::has_work)
            {
                SetTimer(self.hwnd, RESOURCE_TIMER_ID, 50, None);
            } else {
                let _ = KillTimer(self.hwnd, RESOURCE_TIMER_ID);
            }
        }
        self.render = Some(render);
        result?;
        self.publish_accessibility()
    }

    fn publish_accessibility(&mut self) -> Result<(), ShellError> {
        let Some(host) = self.accessibility.as_ref() else {
            return Ok(());
        };
        let Some(render) = self.render.as_ref() else {
            return Ok(());
        };
        let Some(layout) = render.layout.as_ref() else {
            return Ok(());
        };
        let session = self.state.document().session();
        let source = session.snapshot();
        let identity = self.state.document().identity();
        let previous = host
            .shared
            .snapshot
            .read()
            .ok()
            .and_then(|snapshot| snapshot.clone());
        let (semantic, labels) = if let Some(old) = previous
            .filter(|old| old.identity == identity && old.source.revision() == source.revision())
        {
            (old.semantic.clone(), old.labels.clone())
        } else {
            let document = session.document().editor();
            let semantic = Arc::new(
                yu_editor::AccessibilitySemanticSnapshot::from_document(document)
                    .map_err(|error| ShellError::Platform(error.to_string()))?,
            );
            let labels = semantic
                .nodes()
                .iter()
                .filter(|node| node.index() != 0)
                .map(|node| {
                    semantic
                        .label_text(document, node.index())
                        .map(|text| (node.index(), text))
                })
                .collect::<Result<std::collections::HashMap<_, _>, _>>()
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            (semantic, Arc::new(labels))
        };
        let text = yu_editor::AccessibilityTextSnapshot::from_selection(
            source.clone(),
            session.selection(),
        )
        .map_err(|error| ShellError::Platform(error.to_string()))?;
        let mut origin = POINT::default();
        if !unsafe { ClientToScreen(self.surface, &mut origin) }.as_bool() {
            return Err(platform_error(WindowsError::from_win32()));
        }
        let surface = render.renderer.surface();
        let snapshot = AccessibilitySnapshot {
            identity,
            source,
            text,
            semantic,
            labels,
            layout: layout.clone(),
            bounds: UiaRect {
                left: origin.x as f64,
                top: origin.y as f64,
                width: surface.pixel_width() as f64,
                height: surface.pixel_height() as f64,
            },
            scale: surface.scale(),
            scroll_y: render.builder.config().viewport().scroll_y(),
            focused: self.editor_focused,
            visible: unsafe {
                windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.surface)
            }
            .as_bool()
                && !unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(self.hwnd) }
                    .as_bool(),
            caret: session.selection().focus(),
            name: self
                .state
                .document()
                .display_name(self.state.strings())
                .to_owned(),
        };
        host.publish(snapshot);
        Ok(())
    }

    fn accessibility_action(&mut self, action: AccessibilityAction) -> windows::core::Result<()> {
        if let AccessibilityAction::Focus = action {
            unsafe {
                let _ = SetFocus(self.surface);
            }
            return Ok(());
        }
        let (identity, revision) = match &action {
            AccessibilityAction::Select {
                identity, revision, ..
            }
            | AccessibilityAction::Scroll {
                identity, revision, ..
            }
            | AccessibilityAction::Toggle {
                identity, revision, ..
            } => (*identity, *revision),
            AccessibilityAction::Focus => unreachable!(),
        };
        if identity != self.state.document().identity()
            || revision != self.state.document().session().revision()
        {
            return Err(WindowsError::from_hresult(windows::core::HRESULT(
                windows::Win32::UI::Accessibility::UIA_E_ELEMENTNOTAVAILABLE as i32,
            )));
        }
        if self.state.document().session().composition().is_some() {
            return Err(WindowsError::from_hresult(windows::core::HRESULT(
                windows::Win32::UI::Accessibility::UIA_E_INVALIDOPERATION as i32,
            )));
        }
        let result = (|| -> Result<(), ShellError> {
            match action {
                AccessibilityAction::Select { source, .. } => {
                    let snapshot = self.state.document().session().snapshot();
                    let selection = EditorSelection::range(
                        &snapshot,
                        source.start(),
                        source.end(),
                        CaretAffinity::Downstream,
                    )
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                    let before = self.input_projection()?;
                    let before_selection = self.state.document().session().selection();
                    self.state
                        .document_mut()
                        .session_mut()
                        .set_selection(selection)?;
                    self.reveal_panel_selection()?;
                    self.notify_tsf_after_command(before.end_acp(), before_selection, false)?;
                }
                AccessibilityAction::Scroll { source, top, .. } => {
                    let old_selection = self.state.document().session().selection();
                    let snapshot = self.state.document().session().snapshot();
                    let position = if top { source.start() } else { source.end() };
                    let selection = EditorSelection::range(
                        &snapshot,
                        position,
                        position,
                        CaretAffinity::Downstream,
                    )
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                    self.state
                        .document_mut()
                        .session_mut()
                        .set_selection(selection)?;
                    let target = if let Some(render) = self.render.as_ref() {
                        let config = render.builder.config();
                        self.state
                            .document_mut()
                            .session_mut()
                            .caret_scroll_request_with_shaper(
                                config.viewport(),
                                0.0,
                                render.builder.shaper(),
                            )
                    } else {
                        self.state
                            .document_mut()
                            .session_mut()
                            .set_selection(old_selection)?;
                        return Ok(());
                    };
                    self.state
                        .document_mut()
                        .session_mut()
                        .set_selection(old_selection)?;
                    let request = target?;
                    if let Some(render) = self.render.as_mut() {
                        let config = render.builder.config();
                        let caret = request.caret();
                        let target = if top {
                            caret.y()
                        } else {
                            caret.y() + caret.height() - config.viewport().height()
                        };
                        render
                            .builder
                            .update_config(viewport_render_config(
                                render.renderer.surface(),
                                config.appearance(),
                                target.max(0.0),
                            )?)
                            .map_err(|error| ShellError::Platform(error.to_string()))?;
                    }
                    self.render_current()?;
                    if let Some(tsf) = self.tsf.as_ref() {
                        tsf.notify_layout_change();
                    }
                }
                AccessibilityAction::Toggle { block, .. } => {
                    let before = self.input_projection()?;
                    let before_selection = self.state.document().session().selection();
                    self.state
                        .document_mut()
                        .session_mut()
                        .execute(EditorCommand::ToggleTask { block })?;
                    self.refresh_chrome();
                    self.render_current()?;
                    self.notify_tsf_after_command(before.end_acp(), before_selection, true)?;
                }
                AccessibilityAction::Focus => {}
            }
            Ok(())
        })();
        result.map_err(|_| WindowsError::from_hresult(windows::Win32::Foundation::E_FAIL))
    }

    pub(crate) fn input_projection(&self) -> Result<AcpProjection, ShellError> {
        let session = self.state.document().session();
        let snapshot = session.snapshot();
        AcpProjection::new(&snapshot, session.selection(), session.composition())
            .map_err(|error| ShellError::Platform(error.to_string()))
    }

    pub(crate) fn input_set_selection(
        &mut self,
        selection: AcpSelection,
    ) -> Result<(), ShellError> {
        let projection = self.input_projection()?;
        if projection.composition().is_some() {
            let local = projection
                .composition_local_range(selection.range)
                .map_err(|error| ShellError::Platform(error.to_string()))?
                .ok_or_else(|| {
                    ShellError::Platform(
                        "TSF selection escaped the active composition range".to_owned(),
                    )
                })?;
            let text = self
                .state
                .document()
                .session()
                .composition()
                .map(|overlay| overlay.text().to_owned())
                .ok_or_else(|| ShellError::Platform("composition disappeared".to_owned()))?;
            let utf16 = local_selection_utf16(&text, local)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            self.state
                .document_mut()
                .session_mut()
                .update_composition(text, utf16)?;
        } else {
            let snapshot = self.state.document().session().snapshot();
            let selection = selection_from_acp(&snapshot, selection)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            self.state
                .document_mut()
                .session_mut()
                .set_selection(selection)?;
        }
        self.render_current()
    }

    pub(crate) fn input_replace(
        &mut self,
        range: AcpRange,
        text: &str,
        composing: bool,
    ) -> Result<TS_TEXTCHANGE, ShellError> {
        let inserted = i32::try_from(text.encode_utf16().count())
            .map_err(|_| ShellError::Platform("TSF inserted text exceeds ACP range".to_owned()))?;
        let change = TS_TEXTCHANGE {
            acpStart: range.start(),
            acpOldEnd: range.end(),
            acpNewEnd: range
                .start()
                .checked_add(inserted)
                .ok_or_else(|| ShellError::Platform("TSF ACP overflow".to_owned()))?,
        };

        let projection = self.input_projection()?;
        if composing {
            if projection.composition().is_some() {
                let local = projection
                    .composition_local_range(range)
                    .map_err(|error| ShellError::Platform(error.to_string()))?
                    .ok_or_else(|| {
                        ShellError::Platform(
                            "TSF edit escaped the active composition range".to_owned(),
                        )
                    })?;
                let old = self
                    .state
                    .document()
                    .session()
                    .composition()
                    .map(|overlay| overlay.text().to_owned())
                    .ok_or_else(|| ShellError::Platform("composition disappeared".to_owned()))?;
                let (updated, selection) = replace_local_utf16(&old, local, text)
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                self.state
                    .document_mut()
                    .session_mut()
                    .update_composition(updated, selection)?;
            } else {
                let snapshot = self.state.document().session().snapshot();
                let source = canonical_acp_range_to_source(&snapshot, range)
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                let caret = u64::try_from(inserted)
                    .map_err(|_| ShellError::Platform("negative ACP insertion".to_owned()))?;
                let selection = Utf16Range::new(Utf16Offset::new(caret), Utf16Offset::new(caret))
                    .ok_or_else(|| {
                    ShellError::Platform("invalid preedit selection".to_owned())
                })?;
                self.state.document_mut().session_mut().begin_composition(
                    source,
                    text.to_owned(),
                    selection,
                )?;
            }
        } else {
            if self.state.document().session().composition().is_some() {
                return Err(ShellError::Platform(
                    "permanent TSF edit arrived while composition is active".to_owned(),
                ));
            }
            let snapshot = self.state.document().session().snapshot();
            let source = canonical_acp_range_to_source(&snapshot, range)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            let selection = EditorSelection::range(
                &snapshot,
                source.start(),
                source.end(),
                CaretAffinity::Downstream,
            )
            .map_err(|error| ShellError::Platform(error.to_string()))?;
            self.state
                .document_mut()
                .session_mut()
                .set_selection(selection)?;
            self.state
                .document_mut()
                .session_mut()
                .execute(EditorCommand::InsertText(Arc::from(text)))?;
            self.refresh_chrome();
        }
        self.render_current()?;
        Ok(change)
    }

    pub(crate) fn input_end_composition(&mut self) -> Result<(), ShellError> {
        let Some(text) = self
            .state
            .document()
            .session()
            .composition()
            .map(|overlay| overlay.text().to_owned())
        else {
            return Ok(());
        };
        self.state
            .document_mut()
            .session_mut()
            .commit_composition(text)?;
        self.refresh_chrome();
        self.render_current()
    }

    fn input_layout(&self) -> Result<&LayoutSnapshot, ShellError> {
        self.render
            .as_ref()
            .and_then(|render| render.layout.as_deref())
            .ok_or_else(|| ShellError::Platform("Windows input layout is unavailable".to_owned()))
    }

    fn surface_document_point(&self, mut point: POINT) -> Result<LayoutPoint, ShellError> {
        if !unsafe { ScreenToClient(self.surface, &mut point) }.as_bool() {
            return Err(platform_error(WindowsError::from_win32()));
        }
        let surface = native_surface_config(self.surface)?;
        let viewport = self
            .render
            .as_ref()
            .map(|render| render.builder.config().viewport())
            .ok_or_else(|| ShellError::Platform("render host is unavailable".to_owned()))?;
        Ok(LayoutPoint::new(
            point.x as f32 / surface.scale() as f32,
            point.y as f32 / surface.scale() as f32 + viewport.scroll_y(),
        ))
    }

    pub(crate) fn input_acp_from_screen(
        &mut self,
        point: POINT,
        nearest: bool,
    ) -> Result<i32, ShellError> {
        if !nearest {
            let view = self.input_screen_ext()?;
            if point.x < view.left
                || point.x >= view.right
                || point.y < view.top
                || point.y >= view.bottom
            {
                return Err(ShellError::Platform(
                    "TSF point is outside the active editor view".to_owned(),
                ));
            }
        }
        let point = self.surface_document_point(point)?;
        let hit = self
            .input_layout()?
            .hit_test(point)
            .map_err(|error| ShellError::Platform(error.to_string()))?
            .map(|(block, hit)| {
                (
                    hit.source(),
                    hit.visual(),
                    block.layout().visual().composition_visual(),
                )
            });
        if let Some((_, visual, Some(composition_visual))) = hit {
            let projection = self.input_projection()?;
            if let Some(composition) = projection.composition()
                && visual >= composition_visual.start()
                && visual <= composition_visual.end()
            {
                let local_bytes = visual
                    .get()
                    .saturating_sub(composition_visual.start().get());
                let preedit = self
                    .state
                    .document()
                    .session()
                    .composition()
                    .map(|overlay| overlay.text().to_owned())
                    .ok_or_else(|| ShellError::Platform("composition disappeared".to_owned()))?;
                let preedit_snapshot = yu_text::TextBuffer::new(preedit).snapshot();
                let local_utf16 = preedit_snapshot
                    .utf16_offset(ByteOffset::new(local_bytes))
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                return composition
                    .range
                    .start()
                    .checked_add(
                        i32::try_from(local_utf16.get())
                            .map_err(|_| ShellError::Platform("ACP overflow".to_owned()))?,
                    )
                    .ok_or_else(|| ShellError::Platform("ACP overflow".to_owned()));
            }
        }
        let source = hit
            .map(|(source, _, _)| source)
            .ok_or_else(|| ShellError::Platform("TSF point has no measured layout".to_owned()))?;
        let snapshot = self.state.document().session().snapshot();
        let utf16 = snapshot
            .utf16_offset(source)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        i32::try_from(utf16.get()).map_err(|_| ShellError::Platform("ACP overflow".to_owned()))
    }

    fn logical_rect_to_screen(
        &self,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    ) -> Result<RECT, ShellError> {
        let surface = native_surface_config(self.surface)?;
        let viewport = self
            .render
            .as_ref()
            .map(|render| render.builder.config().viewport())
            .ok_or_else(|| ShellError::Platform("render host is unavailable".to_owned()))?;
        let scale = surface.scale() as f32;
        let mut a = POINT {
            x: (left * scale).floor() as i32,
            y: ((top - viewport.scroll_y()) * scale).floor() as i32,
        };
        let mut b = POINT {
            x: (right * scale).ceil() as i32,
            y: ((bottom - viewport.scroll_y()) * scale).ceil() as i32,
        };
        unsafe {
            if !ClientToScreen(self.surface, &mut a).as_bool()
                || !ClientToScreen(self.surface, &mut b).as_bool()
            {
                return Err(platform_error(WindowsError::from_win32()));
            }
        }
        Ok(RECT {
            left: a.x,
            top: a.y,
            right: b.x.max(a.x + 1),
            bottom: b.y.max(a.y + 1),
        })
    }

    pub(crate) fn input_text_ext(&mut self, range: AcpRange) -> Result<RECT, ShellError> {
        let projection = self.input_projection()?;
        if let Some(composition) = projection.composition()
            && range.start() >= composition.range.start()
            && range.end() <= composition.range.end()
        {
            let block = self
                .input_layout()?
                .block_for_source(composition.canonical.start())
                .ok_or_else(|| {
                    ShellError::Platform("composition block is not measured".to_owned())
                })?;
            let visual = block
                .layout()
                .visual()
                .composition_selection_visual()
                .ok_or_else(|| {
                    ShellError::Platform("composition geometry is unavailable".to_owned())
                })?;
            let caret = block
                .layout()
                .caret_for_visual(visual.end(), Bias::After)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            let point = block.document_point(caret.point());
            return self.logical_rect_to_screen(
                point.x(),
                point.y(),
                point.x() + 1.0,
                point.y() + block.caret_height(caret),
            );
        }

        let start = projection
            .projected_to_canonical(range.start())
            .map_err(|error| ShellError::Platform(error.to_string()))?
            .ok_or_else(|| ShellError::Platform("ACP has no canonical source".to_owned()))?;
        let end = projection
            .projected_to_canonical(range.end())
            .map_err(|error| ShellError::Platform(error.to_string()))?
            .ok_or_else(|| ShellError::Platform("ACP has no canonical source".to_owned()))?;
        let snapshot = self.state.document().session().snapshot();
        let canonical =
            AcpRange::new(start, end).map_err(|error| ShellError::Platform(error.to_string()))?;
        let source = canonical_acp_range_to_source(&snapshot, canonical)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        let (left, top, right, bottom) = source_range_bounds(self.input_layout()?, source)?;
        self.logical_rect_to_screen(left, top, right, bottom)
    }

    pub(crate) fn input_screen_ext(&mut self) -> Result<RECT, ShellError> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(self.surface, &mut rect) }.map_err(platform_error)?;
        let mut top_left = POINT {
            x: rect.left,
            y: rect.top,
        };
        let mut bottom_right = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        unsafe {
            if !ClientToScreen(self.surface, &mut top_left).as_bool()
                || !ClientToScreen(self.surface, &mut bottom_right).as_bool()
            {
                return Err(platform_error(WindowsError::from_win32()));
            }
        }
        Ok(RECT {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        })
    }

    fn notify_tsf_after_command(
        &mut self,
        before_end: i32,
        before_selection: EditorSelection,
        changed: bool,
    ) -> Result<(), ShellError> {
        let after = self.input_projection()?;
        let selection_changed = self.state.document().session().selection() != before_selection;
        if let Some(tsf) = self.tsf.as_ref() {
            if changed {
                tsf.notify_external_change(before_end, after.end_acp(), selection_changed);
            } else if selection_changed {
                tsf.notify_selection_change();
            }
        }
        Ok(())
    }

    fn execute_input_command(&mut self, command: EditorCommand) -> Result<bool, ShellError> {
        let before = self.input_projection()?;
        let before_selection = self.state.document().session().selection();
        let result = self.state.document_mut().session_mut().execute(command)?;
        self.refresh_chrome();
        self.render_current()?;
        self.notify_tsf_after_command(before.end_acp(), before_selection, result.changed())?;
        Ok(true)
    }

    fn execute_vertical_input(&mut self, up: bool, extend: bool) -> Result<bool, ShellError> {
        let before = self.input_projection()?;
        let before_selection = self.state.document().session().selection();
        let (config, shaper) = {
            let render = self
                .render
                .as_ref()
                .ok_or_else(|| ShellError::Platform("render host is unavailable".to_owned()))?;
            let config = render
                .layout
                .as_ref()
                .map(|layout| layout.config())
                .ok_or_else(|| ShellError::Platform("input layout is unavailable".to_owned()))?;
            (config, render.builder.shaper().clone())
        };
        let result = self
            .state
            .document_mut()
            .session_mut()
            .move_vertical_with_shaper(up, extend, config, &shaper)?;
        self.render_current()?;
        self.notify_tsf_after_command(before.end_acp(), before_selection, result.changed())?;
        Ok(true)
    }

    fn key_modifiers_for(&self, key: usize) -> KeyModifiers {
        let pressed = |vk| unsafe { GetKeyState(vk) } < 0;
        let mut modifiers = KeyModifiers::NONE;
        if pressed(VK_SHIFT.0 as i32) {
            modifiers = modifiers | KeyModifiers::SHIFT;
        }
        if pressed(VK_MENU.0 as i32) {
            modifiers = modifiers | KeyModifiers::OPTION;
        }
        if pressed(VK_CONTROL.0 as i32) {
            modifiers = modifiers
                | if matches!(
                    key,
                    value if value == VK_HOME.0 as usize
                        || value == VK_END.0 as usize
                        || (b'A' as usize..=b'Z' as usize).contains(&value)
                ) {
                    KeyModifiers::COMMAND
                } else {
                    KeyModifiers::CONTROL
                };
        }
        modifiers
    }

    fn copy_selection_to_clipboard(&mut self, cut: bool) -> Result<bool, ShellError> {
        let session = self.state.document().session();
        let snapshot = session.snapshot();
        let mut pieces = Vec::new();
        let mut all_nonempty = true;
        for selection in session.selections().as_slice() {
            let range = selection.ordered_range();
            if range.is_empty() {
                all_nonempty = false;
                continue;
            }
            let start = usize::try_from(range.start().get())
                .map_err(|_| ShellError::Platform("selection offset overflow".to_owned()))?;
            let end = usize::try_from(range.end().get())
                .map_err(|_| ShellError::Platform("selection offset overflow".to_owned()))?;
            pieces.push(
                snapshot
                    .as_str()
                    .get(start..end)
                    .ok_or_else(|| {
                        ShellError::Platform("selection is not UTF-8 aligned".to_owned())
                    })?
                    .to_owned(),
            );
        }
        if pieces.is_empty() {
            return Ok(true);
        }
        write_unicode_clipboard(self.surface, &pieces.join("\n"))?;
        if cut && all_nonempty {
            self.execute_input_command(EditorCommand::DeleteBackward)?;
        }
        Ok(true)
    }

    fn paste_from_clipboard(&mut self) -> Result<bool, ShellError> {
        let text = read_unicode_clipboard(self.surface)?;
        self.execute_input_command(EditorCommand::PasteClipboardText {
            text: Arc::from(text),
            tabular: false,
        })
    }

    fn handle_key_down(&mut self, wparam: WPARAM) -> Result<bool, ShellError> {
        let key = wparam.0;
        let modifiers = self.key_modifiers_for(key);

        if modifiers == KeyModifiers::COMMAND {
            match key {
                value if value == b'C' as usize => {
                    return self.copy_selection_to_clipboard(false);
                }
                value if value == b'X' as usize => {
                    return self.copy_selection_to_clipboard(true);
                }
                value if value == b'V' as usize => {
                    return self.paste_from_clipboard();
                }
                _ => {}
            }
        }
        if key == b'A' as usize && modifiers == KeyModifiers::COMMAND {
            let snapshot = self.state.document().session().snapshot();
            let selection = EditorSelection::range(
                &snapshot,
                ByteOffset::ZERO,
                snapshot.len_bytes(),
                CaretAffinity::Downstream,
            )
            .map_err(|error| ShellError::Platform(error.to_string()))?;
            let before = self.state.document().session().selection();
            self.state
                .document_mut()
                .session_mut()
                .set_selection(selection)?;
            self.render_current()?;
            if before != selection
                && let Some(tsf) = self.tsf.as_ref()
            {
                tsf.notify_selection_change();
            }
            return Ok(true);
        }
        if key == b'Y' as usize && modifiers == KeyModifiers::COMMAND {
            return self.execute_input_command(EditorCommand::Redo);
        }
        if key == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 as usize
            && self.state.document().session().composition().is_some()
        {
            let _ = self.state.document_mut().session_mut().cancel_composition();
            self.render_current()?;
            if let Some(tsf) = self.tsf.as_ref() {
                tsf.notify_layout_change();
            }
            return Ok(true);
        }

        if key == VK_UP.0 as usize || key == VK_DOWN.0 as usize {
            let extend = modifiers.contains(KeyModifiers::SHIFT);
            return self.execute_vertical_input(key == VK_UP.0 as usize, extend);
        }
        if key == VK_PRIOR.0 as usize || key == VK_NEXT.0 as usize {
            let extend = modifiers.contains(KeyModifiers::SHIFT);
            let lines = self
                .render
                .as_ref()
                .and_then(|render| render.layout.as_ref())
                .map(|layout| {
                    let viewport = self
                        .render
                        .as_ref()
                        .expect("render is present")
                        .builder
                        .config()
                        .viewport();
                    (viewport.height() / layout.config().line_height())
                        .floor()
                        .max(1.0) as usize
                })
                .unwrap_or(1);
            for _ in 0..lines {
                self.execute_vertical_input(key == VK_PRIOR.0 as usize, extend)?;
            }
            return Ok(true);
        }

        let editor_key = match key {
            value if value == VK_BACK.0 as usize => Some(EditorKey::Backspace),
            value if value == VK_DELETE.0 as usize => Some(EditorKey::Delete),
            value if value == VK_LEFT.0 as usize => Some(EditorKey::Left),
            value if value == VK_RIGHT.0 as usize => Some(EditorKey::Right),
            value if value == VK_HOME.0 as usize => Some(EditorKey::Home),
            value if value == VK_END.0 as usize => Some(EditorKey::End),
            value if value == VK_RETURN.0 as usize => Some(EditorKey::Enter),
            value if value == VK_TAB.0 as usize => Some(EditorKey::Tab),
            value if (b'A' as usize..=b'Z' as usize).contains(&value) => {
                char::from_u32(value as u32).map(EditorKey::Character)
            }
            _ => None,
        };
        let Some(editor_key) = editor_key else {
            return Ok(false);
        };

        let before = self.input_projection()?;
        let before_selection = self.state.document().session().selection();
        match self
            .state
            .document_mut()
            .session_mut()
            .route_key(KeyEvent::new(editor_key, modifiers))?
        {
            KeyRouteResult::Executed(result) => {
                self.refresh_chrome();
                self.render_current()?;
                self.notify_tsf_after_command(
                    before.end_acp(),
                    before_selection,
                    result.changed(),
                )?;
                Ok(true)
            }
            KeyRouteResult::Unhandled if editor_key == EditorKey::Tab => {
                self.execute_input_command(EditorCommand::insert_text("\t"))
            }
            KeyRouteResult::Unhandled => Ok(false),
        }
    }

    fn handle_char(&mut self, unit: u16) -> Result<bool, ShellError> {
        if matches!(unit, 0x08 | 0x09 | 0x0A | 0x0D) || unit < 0x20 {
            return Ok(true);
        }
        if (0xD800..=0xDBFF).contains(&unit) {
            self.pending_high_surrogate = Some(unit);
            return Ok(true);
        }
        let character = if (0xDC00..=0xDFFF).contains(&unit) {
            let Some(high) = self.pending_high_surrogate.take() else {
                return Ok(true);
            };
            char::decode_utf16([high, unit]).next().and_then(Result::ok)
        } else {
            self.pending_high_surrogate = None;
            char::from_u32(u32::from(unit))
        };
        let Some(character) = character else {
            return Ok(true);
        };
        self.execute_input_command(EditorCommand::insert_text(character.to_string()))
    }

    fn adjust_scroll(&mut self, delta: f32) -> Result<bool, ShellError> {
        self.render
            .as_mut()
            .ok_or_else(|| ShellError::Platform("render host is unavailable".to_owned()))?
            .scroll_by(delta)
    }

    fn scroll_and_render(&mut self, delta: f32) -> Result<bool, ShellError> {
        if !self.adjust_scroll(delta)? {
            return Ok(true);
        }
        self.render_current()?;
        if let Some(tsf) = self.tsf.as_ref() {
            tsf.notify_layout_change();
        }
        Ok(true)
    }

    fn handle_mouse_wheel(&mut self, wparam: WPARAM) -> Result<bool, ShellError> {
        let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as f32;
        if delta == 0.0 {
            return Ok(true);
        }
        let line_height = self
            .render
            .as_ref()
            .and_then(|render| render.layout.as_ref())
            .map(|layout| layout.config().line_height())
            .unwrap_or(BODY_FONT_SIZE * 1.5);
        let logical_delta = -(delta / 120.0) * line_height * 3.0;
        self.scroll_and_render(logical_delta)
    }

    fn drag_scroll_delta(&self, y: i32) -> Result<f32, ShellError> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(self.surface, &mut rect) }.map_err(platform_error)?;
        let outside = if y < rect.top {
            y - rect.top
        } else if y > rect.bottom {
            y - rect.bottom
        } else {
            0
        };
        if outside == 0 {
            return Ok(0.0);
        }
        let surface = native_surface_config(self.surface)?;
        let line_height = self
            .render
            .as_ref()
            .and_then(|render| render.layout.as_ref())
            .map(|layout| layout.config().line_height())
            .unwrap_or(BODY_FONT_SIZE * 1.5);
        let overshoot = outside as f32 / surface.scale() as f32;
        let magnitude = overshoot.abs().max(line_height).min(line_height * 6.0);
        Ok(magnitude.copysign(overshoot))
    }

    fn update_drag_scroll_timer(&self, y: i32) {
        let mut rect = RECT::default();
        let outside = unsafe { GetClientRect(self.surface, &mut rect) }.is_ok()
            && (y < rect.top || y > rect.bottom);
        unsafe {
            if outside {
                let _ = SetTimer(
                    self.surface,
                    DRAG_SCROLL_TIMER_ID,
                    DRAG_SCROLL_TIMER_MS,
                    None,
                );
            } else {
                let _ = KillTimer(self.surface, DRAG_SCROLL_TIMER_ID);
            }
        }
    }

    fn stop_drag_scroll_timer(&self) {
        unsafe {
            let _ = KillTimer(self.surface, DRAG_SCROLL_TIMER_ID);
        }
    }

    fn drag_scroll_tick(&mut self) -> Result<(), ShellError> {
        let Some((x, y)) = self.drag_point else {
            self.stop_drag_scroll_timer();
            return Ok(());
        };
        let delta = self.drag_scroll_delta(y)?;
        if delta == 0.0 || !self.adjust_scroll(delta)? {
            return Ok(());
        }
        // Publish the newly scrolled viewport before hit-testing the pointer;
        // otherwise a long drag can select against the previous frame's
        // measured blocks and lag one scroll step behind.
        self.render_current()?;
        self.set_mouse_selection(x, y, false, true)
    }

    fn record_double_click(&mut self, x: i32, y: i32) {
        self.last_double_click = Some((Instant::now(), x, y));
    }

    fn consume_triple_click(&mut self, x: i32, y: i32) -> bool {
        let Some((when, double_x, double_y)) = self.last_double_click else {
            return false;
        };
        let elapsed = when.elapsed();
        if elapsed > TRIPLE_CLICK_WINDOW {
            self.last_double_click = None;
            return false;
        }
        let close = x.abs_diff(double_x) <= TRIPLE_CLICK_RADIUS_PX as u32
            && y.abs_diff(double_y) <= TRIPLE_CLICK_RADIUS_PX as u32;
        if close {
            self.last_double_click = None;
        }
        close
    }

    fn set_semantic_mouse_selection(
        &mut self,
        x: i32,
        y: i32,
        line: bool,
    ) -> Result<(), ShellError> {
        let source = self.mouse_source(x, y)?;
        let selection = {
            let editor = self.state.document().session().document().editor();
            if line {
                editor.line_selection_at(source)
            } else {
                editor.word_selection_at(source)
            }
            .map_err(|error| ShellError::Platform(error.to_string()))?
        };
        self.state
            .document_mut()
            .session_mut()
            .set_selection(selection)?;
        self.drag_anchor = Some(selection.anchor());
        self.drag_point = Some((x, y));
        self.render_current()?;
        if let Some(tsf) = self.tsf.as_ref() {
            tsf.notify_selection_change();
        }
        Ok(())
    }

    fn mouse_source(&self, x: i32, y: i32) -> Result<ByteOffset, ShellError> {
        let surface = native_surface_config(self.surface)?;
        let viewport = self
            .render
            .as_ref()
            .map(|render| render.builder.config().viewport())
            .ok_or_else(|| ShellError::Platform("render host is unavailable".to_owned()))?;
        let point = LayoutPoint::new(
            x as f32 / surface.scale() as f32,
            y as f32 / surface.scale() as f32 + viewport.scroll_y(),
        );
        Ok(self
            .input_layout()?
            .hit_test(point)
            .map_err(|error| ShellError::Platform(error.to_string()))?
            .map(|(_, hit)| hit.source())
            .unwrap_or(ByteOffset::ZERO))
    }

    fn set_mouse_selection(
        &mut self,
        x: i32,
        y: i32,
        extend: bool,
        dragging: bool,
    ) -> Result<(), ShellError> {
        let focus = self.mouse_source(x, y)?;
        let snapshot = self.state.document().session().snapshot();
        let anchor = if dragging {
            self.drag_anchor.unwrap_or(focus)
        } else if extend {
            self.state.document().session().selection().anchor()
        } else {
            focus
        };
        let selection = EditorSelection::range(&snapshot, anchor, focus, CaretAffinity::Downstream)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        self.state
            .document_mut()
            .session_mut()
            .set_selection(selection)?;
        if !dragging {
            self.drag_anchor = Some(anchor);
        }
        self.drag_point = Some((x, y));
        self.render_current()?;
        if let Some(tsf) = self.tsf.as_ref() {
            tsf.notify_selection_change();
        }
        Ok(())
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
        let chrome = Chrome::new(self.hwnd, &self.state)?;
        self.sidebar = chrome.background;
        self.status = chrome.status;
        self.chrome = Some(chrome);
        unsafe {
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
            // Child creation can place the surface below existing siblings.
            // Keep the GPU view above the full-client background canvas.
            SetWindowPos(
                self.surface,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
            .map_err(platform_error)?;
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

        let metrics = self.state.metrics();
        let spec = self.state.appearance().theme_id().spec();
        let status_h = metrics.px(24.0).min(height);
        let content_h = (height - status_h).max(1);
        let sidebar_w = sidebar_width(metrics, self.state.sidebar());
        let available_w = (width - sidebar_w).max(1);
        let gutter = metrics.px(spec.gutter).min((available_w - 1) / 4);
        let surface_w = (available_w - gutter * 2)
            .min(metrics.px(spec.column_width))
            .max(1);
        let surface_x = sidebar_w + (available_w - surface_w) / 2;
        let surface_y = metrics.px(spec.top).min((content_h - 1) / 4);
        let bottom = metrics.px(16.0).min((content_h - 1) / 4);
        let surface_h = (content_h - surface_y - bottom).max(1);

        unsafe {
            let _ = MoveWindow(
                self.surface,
                surface_x,
                surface_y,
                surface_w,
                surface_h,
                true,
            );
        }
        if let Some(chrome) = self.chrome.as_mut()
            && let Err(error) = chrome.layout(
                self.state.metrics(),
                self.state.sidebar(),
                self.state.appearance(),
            )
        {
            show_error(self.hwnd, &self.state, &error);
        }
        self.refresh_chrome();
    }

    fn apply_system_theme(&mut self) {
        self.contrast = crate::contrast::system_contrast();
        let dark = self.contrast.map_or_else(system_prefers_dark, |c| {
            u32::from(c.background.red())
                + u32::from(c.background.green())
                + u32::from(c.background.blue())
                < 384
        });
        self.state.set_appearance(if dark {
            Appearance::YuDark
        } else {
            Appearance::YuLight
        });
        unsafe {
            let dark_value = BOOL::from(dark);
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                ptr::from_ref(&dark_value).cast(),
                size_of::<BOOL>() as u32,
            );
            let backdrop = if self.contrast.is_some() {
                DWMSBT_NONE
            } else {
                DWMSBT_MAINWINDOW
            };
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
            ID_FOCUS_NEXT | ID_FOCUS_PREVIOUS => {
                let mut targets = vec![self.surface];
                if let Some(chrome) = self.chrome.as_ref() {
                    targets.extend(chrome.focus_targets());
                }
                let focus = unsafe { GetFocus() };
                let index = targets
                    .iter()
                    .position(|target| *target == focus)
                    .unwrap_or(0);
                let next = if command == ID_FOCUS_NEXT {
                    (index + 1) % targets.len()
                } else {
                    (index + targets.len() - 1) % targets.len()
                };
                unsafe {
                    let _ = SetFocus(targets[next]);
                }
            }
            ID_FILE_NEW => {
                if self.confirm_replace_current()? {
                    self.state.new_document();
                    if let Some(chrome) = self.chrome.as_mut() {
                        chrome.invalidate_content();
                    }
                    self.refresh_chrome();
                }
            }
            ID_FILE_OPEN => {
                if let Some(path) = open_file_dialog(self.hwnd)? {
                    let candidate = DocumentSlot::open(path)?;
                    if self.confirm_replace_current()? {
                        self.state.replace_document(candidate);
                        if let Some(chrome) = self.chrome.as_mut() {
                            chrome.invalidate_content();
                        }
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
                unsafe {
                    let _ = SetFocus(self.surface);
                }
            }
            ID_SEARCH => {
                self.handle_chrome_command(ID_SEARCH, 0)?;
            }
            _ => {}
        }
        if command != ID_FILE_EXIT {
            self.render_current()?;
        }
        Ok(())
    }

    fn handle_chrome_command(&mut self, command: u16, notification: u16) -> Result<(), ShellError> {
        match command {
            ID_FILES | ID_OUTLINE | ID_SEARCH => {
                self.state.set_sidebar(match command {
                    ID_FILES => SidebarMode::Files,
                    ID_OUTLINE => SidebarMode::Outline,
                    _ => SidebarMode::Search,
                });
                self.update_layout();
                self.render_current()?;
                if let Some(tsf) = self.tsf.as_ref() {
                    tsf.notify_layout_change();
                }
                if command == ID_SEARCH
                    && let Some(chrome) = self.chrome.as_ref()
                {
                    unsafe {
                        let _ = SetFocus(chrome.query);
                    }
                }
            }
            ID_QUERY if notification == EN_CHANGE as u16 => {
                self.render_current()?;
            }
            ID_ROWS
                if (notification == LBN_SELCHANGE as u16 || notification == LBN_DBLCLK as u16)
                    && (self.state.sidebar() != SidebarMode::Files
                        || notification == LBN_DBLCLK as u16) =>
            {
                self.activate_panel_row(notification == LBN_DBLCLK as u16)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn activate_panel_row(&mut self, focus_editor: bool) -> Result<(), ShellError> {
        // TSF owns active preedit. Navigation must not replace its canonical range.
        if self.state.document().session().composition().is_some() {
            return Ok(());
        }
        let action = self.chrome.as_ref().and_then(Chrome::selected_action);
        let before = self.input_projection()?;
        let before_selection = self.state.document().session().selection();
        let mut document_changed = false;
        match action {
            Some(PanelAction::File(path)) => {
                if path != self.state.document().session().path() {
                    let candidate = DocumentSlot::open(path)?;
                    if !self.confirm_replace_current()? {
                        return Ok(());
                    }
                    self.state.replace_document(candidate);
                    document_changed = true;
                    if let Some(chrome) = self.chrome.as_mut() {
                        chrome.invalidate_content();
                    }
                    if let Some(render) = self.render.as_mut() {
                        let config = render.builder.config();
                        render
                            .builder
                            .update_config(viewport_render_config(
                                render.renderer.surface(),
                                config.appearance(),
                                0.0,
                            )?)
                            .map_err(|error| ShellError::Platform(error.to_string()))?;
                    }
                }
            }
            Some(PanelAction::Select(range)) => {
                let snapshot = self.state.document().session().snapshot();
                let selection = EditorSelection::range(
                    &snapshot,
                    range.start(),
                    range.end(),
                    CaretAffinity::Downstream,
                )
                .map_err(|error| ShellError::Platform(error.to_string()))?;
                self.state
                    .document_mut()
                    .session_mut()
                    .set_selection(selection)?;
                self.reveal_panel_selection()?;
            }
            None => return Ok(()),
        }
        self.refresh_chrome();
        self.render_current()?;
        self.notify_tsf_after_command(before.end_acp(), before_selection, document_changed)?;
        if let Some(tsf) = self.tsf.as_ref() {
            tsf.notify_layout_change();
        }
        if focus_editor {
            unsafe {
                let _ = SetFocus(self.surface);
            }
        }
        Ok(())
    }

    fn reveal_panel_selection(&mut self) -> Result<(), ShellError> {
        // Measuring newly visible blocks can change the target's document y.
        // Adopt each frame before resolving the next request against that geometry.
        for _ in 0..4 {
            let Some(render) = self.render.as_mut() else {
                return Ok(());
            };
            let config = render.builder.config();
            let request = self
                .state
                .document_mut()
                .session_mut()
                .caret_scroll_request_with_shaper(
                    config.viewport(),
                    24.0,
                    render.builder.shaper(),
                )?;
            render
                .builder
                .update_config(viewport_render_config(
                    render.renderer.surface(),
                    config.appearance(),
                    request.target_scroll_y(),
                )?)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            self.render_current()?;
            let render = self
                .render
                .as_ref()
                .expect("render host survived publication");
            let range = TextRange::empty(self.state.document().session().selection().focus());
            if let Some(layout) = render.layout.as_ref()
                && let Ok((_, top, _, bottom)) = source_range_bounds(layout, range)
            {
                let viewport = render.builder.config().viewport();
                if top >= viewport.scroll_y() && bottom <= viewport.scroll_y() + viewport.height() {
                    return Ok(());
                }
            }
        }
        Err(ShellError::Platform(
            "navigation target could not be revealed in the current viewport".into(),
        ))
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
    let _com =
        ComApartment::initialize().map_err(|error| startup_error("initialize COM", error))?;
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    register_classes().map_err(|error| startup_error("register window classes", error))?;

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
            return Err(startup_error("create main window", error));
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
    static REGISTERED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    REGISTERED
        .get_or_init(|| register_classes_once().map_err(|error| error.to_string()))
        .as_ref()
        .copied()
        .map_err(|message| ShellError::Platform(message.clone()))
}

fn register_classes_once() -> Result<(), ShellError> {
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
        style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
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
            fVirt: FVIRTKEY,
            key: VK_F6.0,
            cmd: ID_FOCUS_NEXT,
        },
        ACCEL {
            fVirt: FVIRTKEY | FSHIFT,
            key: VK_F6.0,
            cmd: ID_FOCUS_PREVIOUS,
        },
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
        ACCEL {
            fVirt: FCONTROL | FVIRTKEY,
            key: b'F' as u16,
            cmd: ID_SEARCH,
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
        let app_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppWindow;
        let tsf_ate = if app_ptr.is_null() {
            false
        } else {
            let app = unsafe { &mut *app_ptr };
            match message.message {
                WM_KEYDOWN
                    if app.chrome.as_ref().is_some_and(|chrome| {
                        message.hwnd == chrome.query || message.hwnd == chrome.list
                    }) && message.wParam.0 == VK_RETURN.0 as usize
                        && !app.chrome.as_ref().is_some_and(|chrome| {
                            message.hwnd == chrome.query && chrome.query_is_composing()
                        }) =>
                {
                    if let Some(chrome) = app.chrome.as_ref()
                        && message.hwnd == chrome.query
                    {
                        chrome.select_first();
                    }
                    if let Err(error) = app.activate_panel_row(true) {
                        show_error(hwnd, &app.state, &error);
                    }
                    true
                }
                WM_KEYDOWN
                    if message.hwnd != app.surface
                        && message.wParam.0
                            == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0
                                as usize
                        && !app.chrome.as_ref().is_some_and(|chrome| {
                            message.hwnd == chrome.query && chrome.query_is_composing()
                        }) =>
                {
                    unsafe {
                        let _ = SetFocus(app.surface);
                    }
                    true
                }
                WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP
                    if message.hwnd != app.surface =>
                {
                    false
                }
                WM_KEYDOWN | WM_SYSKEYDOWN => app
                    .tsf
                    .as_ref()
                    .is_some_and(|tsf| tsf.filter_key_down(message.wParam, message.lParam)),
                WM_KEYUP | WM_SYSKEYUP => app
                    .tsf
                    .as_ref()
                    .is_some_and(|tsf| tsf.filter_key_up(message.wParam, message.lParam)),
                _ => false,
            }
        };
        if tsf_ate {
            continue;
        }
        let local_undo = !app_ptr.is_null()
            && message.hwnd != unsafe { (*app_ptr).surface }
            && matches!(message.wParam.0, 0x5a | 0x59)
            && unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        if !local_undo && unsafe { TranslateAcceleratorW(hwnd, accelerator, &message) } != 0 {
            continue;
        }
        if unsafe { IsDialogMessageW(hwnd, &message) }.as_bool() {
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
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
                let result = if lparam.0 != 0 && (ID_FILES..=ID_ROWS).contains(&command) {
                    app.handle_chrome_command(command, (wparam.0 >> 16) as u16)
                } else {
                    app.handle_command(command)
                };
                if let Err(error) = result {
                    show_error(hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_SIZE => {
                app.update_layout();
                if let Err(error) = app.render_current() {
                    show_error(hwnd, &app.state, &error);
                } else if let Some(tsf) = app.tsf.as_ref() {
                    tsf.notify_layout_change();
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
                } else if let Some(tsf) = app.tsf.as_ref() {
                    tsf.notify_layout_change();
                }
                return LRESULT(0);
            }
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                app.apply_system_theme();
                if let Some(chrome) = app.chrome.as_mut() {
                    chrome.invalidate_font();
                }
                app.update_layout();
                app.refresh_chrome();
                if let Err(error) = app.render_current() {
                    show_error(hwnd, &app.state, &error);
                } else if let Some(tsf) = app.tsf.as_ref() {
                    tsf.notify_layout_change();
                }
                return LRESULT(0);
            }
            WM_MOVE => {
                let _ = app.publish_accessibility();
            }
            windows::Win32::UI::WindowsAndMessaging::WM_WINDOWPOSCHANGED => {
                // WM_SHOWWINDOW precedes the visibility style change. Publish
                // after DefWindowProc has processed the final window position.
                let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
                let _ = app.publish_accessibility();
                return result;
            }
            WM_APP_RENDER => {
                if let Err(error) = app.render_current() {
                    show_error(hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == RESOURCE_TIMER_ID => {
                let revision = app.state.document().session().revision();
                let changed = app
                    .render
                    .as_mut()
                    .and_then(|render| render.resources.as_mut())
                    .is_some_and(|resources| resources.advance(revision));
                if changed && let Err(error) = app.render_current() {
                    unsafe {
                        let _ = KillTimer(hwnd, RESOURCE_TIMER_ID);
                    }
                    show_error(hwnd, &app.state, &error);
                } else if changed && let Some(tsf) = app.tsf.as_ref() {
                    tsf.notify_layout_change();
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
                if let Some(chrome) = app.chrome.as_ref() {
                    chrome.clear_accessibility_annotations();
                }
                if let Some(host) = app.accessibility.as_ref() {
                    host.close();
                }
                unsafe {
                    PostQuitMessage(0);
                }
                return LRESULT(0);
            }
            WM_MEASUREITEM => {
                let item = unsafe { &mut *(lparam.0 as *mut MEASUREITEMSTRUCT) };
                if item.CtlID == u32::from(ID_ROWS) {
                    item.itemHeight = app.state.metrics().px(32.0) as u32;
                    return LRESULT(1);
                }
            }
            WM_DRAWITEM => {
                let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
                if app.chrome.as_ref().is_some_and(|chrome| chrome.draw(item)) {
                    return LRESULT(1);
                }
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                if let Some(result) = app.chrome.as_ref().and_then(|chrome| {
                    chrome.control_color(
                        windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _),
                        HWND(lparam.0 as *mut _),
                    )
                }) {
                    return result;
                }
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
    let parent = unsafe { GetParent(hwnd) }.ok();
    let app_ptr = parent
        .map(|parent| unsafe { GetWindowLongPtrW(parent, GWLP_USERDATA) } as *mut AppWindow)
        .unwrap_or(ptr::null_mut());
    if !app_ptr.is_null() {
        let app = unsafe { &mut *app_ptr };
        match message {
            WM_GETOBJECT => {
                if let Some(host) = app.accessibility.as_ref() {
                    return unsafe {
                        UiaReturnRawElementProvider(hwnd, wparam, lparam, &host.provider)
                    };
                }
            }
            WM_APP_UIA_ACTION => {
                if let Some(shared) = app.accessibility.as_ref().map(|host| host.shared.clone())
                    && let Some(action) = shared.action(wparam.0)
                {
                    let result = app.accessibility_action(action);
                    shared.complete(wparam.0, result);
                }
                return LRESULT(0);
            }
            WM_GETDLGCODE => {
                return LRESULT(
                    (DLGC_WANTALLKEYS | DLGC_WANTARROWS | DLGC_WANTCHARS | DLGC_WANTTAB) as isize,
                );
            }
            WM_SETFOCUS => {
                app.editor_focused = true;
                if let Some(tsf) = app.tsf.as_ref() {
                    tsf.focus();
                }
                let _ = app.publish_accessibility();
                return LRESULT(0);
            }
            WM_KEYDOWN => match app.handle_key_down(wparam) {
                Ok(true) => return LRESULT(0),
                Ok(false) => {}
                Err(error) => {
                    show_error(app.hwnd, &app.state, &error);
                    return LRESULT(0);
                }
            },
            WM_CHAR => match app.handle_char((wparam.0 & 0xffff) as u16) {
                Ok(true) => return LRESULT(0),
                Ok(false) => {}
                Err(error) => {
                    show_error(app.hwnd, &app.state, &error);
                    return LRESULT(0);
                }
            },
            WM_LBUTTONDBLCLK => {
                unsafe {
                    let _ = SetFocus(hwnd);
                    let _ = SetCapture(hwnd);
                }
                let (x, y) = mouse_coordinates(lparam);
                app.record_double_click(x, y);
                app.semantic_click = true;
                if let Err(error) = app.set_semantic_mouse_selection(x, y, false) {
                    show_error(app.hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    let _ = SetFocus(hwnd);
                    let _ = SetCapture(hwnd);
                }
                let (x, y) = mouse_coordinates(lparam);
                let triple = app.consume_triple_click(x, y);
                app.semantic_click = triple;
                let result = if triple {
                    app.set_semantic_mouse_selection(x, y, true)
                } else {
                    app.set_mouse_selection(x, y, wparam.0 & MK_SHIFT.0 as usize != 0, false)
                };
                if let Err(error) = result {
                    show_error(app.hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_MOUSEMOVE if wparam.0 & MK_LBUTTON.0 as usize != 0 => {
                let (x, y) = mouse_coordinates(lparam);
                if app.semantic_click
                    && app.drag_point.is_some_and(|(start_x, start_y)| {
                        x.abs_diff(start_x) > 2 || y.abs_diff(start_y) > 2
                    })
                {
                    app.semantic_click = false;
                }
                app.drag_point = Some((x, y));
                app.update_drag_scroll_timer(y);
                if let Err(error) = app.set_mouse_selection(x, y, false, true) {
                    show_error(app.hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_MOUSEWHEEL => {
                if let Err(error) = app.handle_mouse_wheel(wparam) {
                    show_error(app.hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == DRAG_SCROLL_TIMER_ID => {
                if let Err(error) = app.drag_scroll_tick() {
                    show_error(app.hwnd, &app.state, &error);
                }
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let (x, y) = mouse_coordinates(lparam);
                if !app.semantic_click
                    && let Err(error) = app.set_mouse_selection(x, y, false, true)
                {
                    show_error(app.hwnd, &app.state, &error);
                }
                app.drag_anchor = None;
                app.drag_point = None;
                app.semantic_click = false;
                app.stop_drag_scroll_timer();
                unsafe {
                    let _ = ReleaseCapture();
                }
                return LRESULT(0);
            }
            WM_KILLFOCUS => {
                app.editor_focused = false;
                app.drag_anchor = None;
                app.drag_point = None;
                app.semantic_click = false;
                app.stop_drag_scroll_timer();
                let _ = app.publish_accessibility();
            }
            WM_APP_TSF_LOCK => {
                if let Some(tsf) = app.tsf.as_ref() {
                    tsf.grant_pending_lock();
                }
                return LRESULT(0);
            }
            _ => {}
        }
    }
    if message == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        unsafe {
            BeginPaint(hwnd, &mut paint);
            let _ = EndPaint(hwnd, &paint);
            if let Some(parent) = parent {
                let _ = PostMessageW(parent, WM_APP_RENDER, WPARAM(0), LPARAM(0));
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn mouse_coordinates(lparam: LPARAM) -> (i32, i32) {
    let raw = lparam.0 as u32;
    let x = (raw & 0xffff) as u16 as i16 as i32;
    let y = (raw >> 16) as u16 as i16 as i32;
    (x, y)
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

fn include_input_bounds(
    bounds: &mut Option<(f32, f32, f32, f32)>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Result<(), ShellError> {
    if !x.is_finite()
        || !y.is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || width < 0.0
        || height <= 0.0
    {
        return Err(ShellError::Platform("invalid input geometry".to_owned()));
    }
    let right = x + width.max(1.0);
    let bottom = y + height;
    *bounds = Some(match *bounds {
        Some((left, top, old_right, old_bottom)) => (
            left.min(x),
            top.min(y),
            old_right.max(right),
            old_bottom.max(bottom),
        ),
        None => (x, y, right, bottom),
    });
    Ok(())
}

fn source_range_bounds(
    layout: &LayoutSnapshot,
    source: TextRange,
) -> Result<(f32, f32, f32, f32), ShellError> {
    let mut bounds = None;
    if source.start() < source.end() {
        for block in layout.blocks() {
            let block_source = block.metadata().source();
            if block_source.start() >= source.end() || block_source.end() <= source.start() {
                continue;
            }
            for cluster in block.layout().layout().clusters() {
                let visual = cluster.visual();
                let start = block
                    .layout()
                    .visual()
                    .visual_to_source(visual.start(), Bias::After)
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                let end = block
                    .layout()
                    .visual()
                    .visual_to_source(visual.end(), Bias::Before)
                    .map_err(|error| ShellError::Platform(error.to_string()))?;
                let span = TextRange::new(start, end.max(start))
                    .ok_or_else(|| ShellError::Platform("invalid cluster source".to_owned()))?;
                if span.start() < source.end() && span.end() > source.start() {
                    let line = block
                        .layout()
                        .layout()
                        .lines()
                        .get(cluster.line())
                        .ok_or_else(|| {
                            ShellError::Platform("cluster line is missing".to_owned())
                        })?;
                    include_input_bounds(
                        &mut bounds,
                        cluster.x(),
                        block.content_y() + line.y(),
                        cluster.width(),
                        line.height(),
                    )?;
                }
            }
        }
    }
    for (position, bias) in [(source.start(), Bias::After), (source.end(), Bias::Before)] {
        if let Some(block) = layout.block_for_source(position) {
            let caret = block
                .layout()
                .caret_for_source(position, bias)
                .map_err(|error| ShellError::Platform(error.to_string()))?;
            let point = block.document_point(caret.point());
            include_input_bounds(
                &mut bounds,
                point.x(),
                point.y(),
                1.0,
                block.caret_height(caret),
            )?;
        }
    }
    bounds.ok_or_else(|| ShellError::Platform("source range is outside measured layout".to_owned()))
}

fn viewport_render_config(
    surface: SurfaceConfig,
    appearance: Appearance,
    scroll_y: f32,
) -> Result<ViewportRenderConfig, ShellError> {
    let width = surface.logical_width() as f32;
    let height = surface.logical_height() as f32;
    // Scene primitives use document coordinates. The shared draw-command
    // builder subtracts this viewport origin when producing surface pixels.
    let scene = Rect::new(0.0, scroll_y.max(0.0), width, height)
        .map_err(|error| ShellError::Platform(error.to_string()))?;
    Ok(ViewportRenderConfig::new(
        ViewportSpan::new(scroll_y.max(0.0), height),
        BODY_FONT_SIZE,
        scene,
        appearance.text(),
    )
    .with_background(appearance.background())
    .with_raster_scale(surface.scale() as f32)
    .with_editor_decorations(appearance.editor_decorations())
    .with_appearance(appearance))
}

fn editor_viewport_config(logical_width: f32, appearance: Appearance) -> ViewportConfig {
    let line_height = BODY_FONT_SIZE * appearance.theme_id().spec().body_line_ratio;
    ViewportConfig::new(
        LayoutConfig::new(logical_width, line_height)
            .with_default_advance(BODY_FONT_SIZE * 0.5)
            .with_theme(appearance.theme_id()),
        line_height,
        line_height * 2.0,
    )
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

fn startup_error(stage: &str, error: impl std::fmt::Display) -> ShellError {
    ShellError::Platform(format!("Windows startup ({stage}): {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::WS_POPUP;
    use yu_scene::{EditorDecorationPrimitiveRole, Primitive};

    fn settle_resources(render: &mut RenderHost, state: &mut ShellState) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            render.render(state).expect("native resource Present");
            if !render
                .resources
                .as_ref()
                .is_some_and(ResourceHost::has_work)
            {
                break;
            }
            assert!(Instant::now() < deadline, "resource deadline exceeded");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "requires the companion helper; run-self-checks.ps1 builds and runs it"]
    fn native_group5_resources_present_recover_and_preserve_source() {
        let _com = ComApartment::initialize().expect("COM");
        let window = Window(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Yu resources regression"),
                WS_POPUP,
                0,
                0,
                1000,
                1600,
                None,
                None,
                None,
                None,
            )
            .expect("native surface")
        });
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Fixtures/group5-resources.md");
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/debug/yu-document-renderer.exe");
        assert!(helper.is_file(), "build yu-document-renderer first");
        let mut state = ShellState::from_path(Locale::English, &fixture).expect("fixture");
        let source = state.document().session().snapshot().as_str().to_owned();
        let revision = state.document().session().revision();
        let mut render = RenderHost::new(window.0, Appearance::Light).expect("D3D");
        render.resources = Some(
            ResourceHost::with_helper(
                state.document().identity(),
                fixture.clone(),
                Appearance::Light,
                render.builder.config().raster_scale(),
                helper.clone(),
            )
            .expect("resource worker"),
        );
        settle_resources(&mut render, &mut state);
        let frame = render.builder.last_publication().expect("frame").frame();
        assert_eq!(
            frame
                .plan()
                .commands()
                .iter()
                .filter(|command| matches!(command, yu_render::RenderCommand::Image { .. }))
                .count(),
            2
        );
        let embedded = frame.plan().embedded_resources();
        assert_eq!(
            embedded
                .iter()
                .filter(|resource| resource.kind() == 0)
                .count(),
            2,
            "inline and display formula"
        );
        assert_eq!(
            embedded
                .iter()
                .filter(|resource| resource.kind() == 1)
                .count(),
            1,
            "Mermaid"
        );
        assert_eq!(state.document().session().revision(), revision);
        assert_eq!(state.document().session().snapshot().as_str(), source);
        assert!(!state.document().session().is_dirty());

        let svg_key = yu_assets::ImageKey::new("assets/group5-transparency.svg")
            .expect("image key")
            .fingerprint();
        let svg_bounds = frame
            .plan()
            .commands()
            .iter()
            .find_map(|command| match command {
                yu_render::RenderCommand::Image {
                    resource, bounds, ..
                } if *resource == svg_key => Some(*bounds),
                _ => None,
            })
            .expect("SVG command");
        render.renderer.request_frame_capture();
        render.render(&mut state).expect("captured Present");
        let capture = render.renderer.take_captured_frame().expect("GPU readback");
        let scale = render.renderer.surface().scale() as f32;
        let left = (svg_bounds.x() * scale) as u32;
        let top = (svg_bounds.y() * scale) as u32;
        let mut colored = 0;
        for y in top..(top + (svg_bounds.height() * scale) as u32).min(capture.height()) {
            for x in left..(left + (svg_bounds.width() * scale) as u32).min(capture.width()) {
                let offset = (y * capture.width() + x) as usize * 4;
                let pixel = &capture.pixels()[offset..offset + 4];
                colored += usize::from(u16::from(pixel[0]) > u16::from(pixel[1]) + 40);
            }
        }
        assert!(
            colored > 200,
            "SVG texture did not reach the D3D target: {colored}"
        );
        let surface = render.renderer.surface();
        render.renderer.recreate(window.0).expect("recreate device");
        settle_resources(&mut render, &mut state);
        render.renderer.request_frame_capture();
        render
            .render(&mut state)
            .expect("captured recovered Present");
        let recovered = render
            .renderer
            .take_captured_frame()
            .expect("recovered GPU frame");
        assert!(
            recovered.pixels() == capture.pixels(),
            "resource recovery changed actual GPU pixels"
        );

        for cycle in 0..20 {
            state
                .document_mut()
                .session_mut()
                .execute(EditorCommand::MoveDocumentBoundary {
                    end: true,
                    extend: false,
                })
                .expect("end");
            let before = state.document().session().snapshot().as_str().to_owned();
            let end = ByteOffset::new(before.len() as u64);
            let preedit_selection = Utf16Range::empty(Utf16Offset::new(5));
            state
                .document_mut()
                .session_mut()
                .begin_composition(TextRange::empty(end), "ceshi", preedit_selection)
                .expect("composition");
            render.render(&mut state).expect("preedit Present");
            assert_eq!(state.document().session().snapshot().as_str(), before);
            assert!(state.document_mut().session_mut().cancel_composition());
            assert_eq!(state.document().session().snapshot().as_str(), before);
            state
                .document_mut()
                .session_mut()
                .begin_composition(TextRange::empty(end), "ceshi", preedit_selection)
                .expect("composition");
            state
                .document_mut()
                .session_mut()
                .commit_composition(format!(" 中文😀{cycle}"))
                .expect("commit");
            render
                .render(&mut state)
                .expect("render during revision changes");
            state
                .document_mut()
                .session_mut()
                .execute(EditorCommand::Undo)
                .expect("undo");
            state
                .document_mut()
                .session_mut()
                .execute(EditorCommand::Redo)
                .expect("redo");
        }
        settle_resources(&mut render, &mut state);
        assert!(
            state
                .document()
                .session()
                .snapshot()
                .as_str()
                .contains("中文😀19")
        );

        if let Some(seconds) = std::env::var("YU_GROUP5_SOAK_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| (1..=600).contains(seconds))
        {
            let started = Instant::now();
            let mut frames = 0;
            let stable_source = state.document().session().snapshot().as_str().to_owned();
            while started.elapsed().as_secs() < seconds {
                render
                    .scroll_by(if frames % 2 == 0 { 64.0 } else { -64.0 })
                    .expect("soak scroll");
                settle_resources(&mut render, &mut state);
                assert_eq!(
                    state.document().session().snapshot().as_str(),
                    stable_source
                );
                frames += 1;
                std::thread::sleep(Duration::from_millis(200));
            }
            render.scroll_by(-f32::MAX).expect("restore viewport");
            settle_resources(&mut render, &mut state);
            println!(
                "yu-group5-soak seconds={:.2} frames={frames} commit=20 cancel=20",
                started.elapsed().as_secs_f64()
            );
        }

        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../artifacts/windows-group5/20261001")
            .join(format!(
                "roundtrip-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_millis()
            ));
        std::fs::create_dir_all(&output).expect("roundtrip directory");
        let saved = output.join("保存 重开 😀.md");
        state
            .document_mut()
            .save_as(&saved, false)
            .expect("Unicode Save As");
        let saved_source = state.document().session().snapshot().as_str().to_owned();
        assert!(!state.document().session().is_dirty());
        assert_eq!(
            std::fs::read_to_string(&saved).expect("saved UTF-8"),
            saved_source
        );
        let old_identity = state.document().identity();
        state.open_document(&saved).expect("reopen");
        assert_ne!(state.document().identity(), old_identity);
        assert_eq!(state.document().session().snapshot().as_str(), saved_source);
        assert!(saved_source.contains("中文😀19") && saved_source.contains("\\frac{1}{2}"));
        render.resources = Some(
            ResourceHost::with_helper(
                state.document().identity(),
                saved.clone(),
                Appearance::Light,
                render.builder.config().raster_scale(),
                helper.clone(),
            )
            .expect("reopened resources"),
        );
        settle_resources(&mut render, &mut state);
        println!(
            "yu-group5-roundtrip path={} source_bytes={} cycles=20 commit=20 cancel=20",
            saved.display(),
            saved_source.len()
        );
        state.set_appearance(Appearance::Dark);
        render
            .builder
            .update_config(
                viewport_render_config(surface, Appearance::Dark, 0.0).expect("dark config"),
            )
            .expect("config");
        render.resources = Some(
            ResourceHost::with_helper(
                state.document().identity(),
                saved,
                Appearance::Dark,
                render.builder.config().raster_scale(),
                helper,
            )
            .expect("dark resources"),
        );
        settle_resources(&mut render, &mut state);
        assert_eq!(
            render
                .builder
                .last_publication()
                .expect("dark frame")
                .frame()
                .plan()
                .embedded_resources()
                .len(),
            3
        );

        let bad = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Fixtures/group5-errors.md");
        state.open_document(&bad).expect("error fixture");
        let original = state.document().session().snapshot().as_str().to_owned();
        render.resources = Some(
            ResourceHost::with_helper(
                state.document().identity(),
                bad,
                Appearance::Dark,
                render.builder.config().raster_scale(),
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../target/debug/yu-document-renderer.exe"),
            )
            .expect("error resources"),
        );
        settle_resources(&mut render, &mut state);
        let diagnostics = render.resources.as_ref().expect("resources").diagnostics();
        assert_eq!(
            diagnostics.3, 3,
            "missing image, invalid math, invalid Mermaid"
        );
        for _ in 0..5 {
            render.render(&mut state).expect("stable error source");
        }
        assert!(!render.resources.as_ref().expect("resources").has_work());
        assert_eq!(state.document().session().snapshot().as_str(), original);
        assert!(!state.document().session().is_dirty());
    }

    struct Window(HWND);

    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    fn has_decoration(
        render: &mut RenderHost,
        state: &ShellState,
        role: EditorDecorationPrimitiveRole,
    ) -> bool {
        let mut context = state
            .document()
            .session()
            .document()
            .editor()
            .capture_render_snapshot()
            .into_layout_context();
        let publication = render.builder.publish(&mut context).expect("editor frame");
        publication.frame().scene().scene().primitives().iter().any(|primitive| {
            matches!(primitive, Primitive::EditorDecoration(decoration) if decoration.role() == role)
        })
    }

    #[test]
    fn native_render_uses_surface_width_and_readable_lines_after_resize() {
        let _com = ComApartment::initialize().expect("COM");
        let window = Window(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Yu layout regression"),
                WS_POPUP,
                0,
                0,
                640,
                480,
                None,
                None,
                None,
                None,
            )
            .expect("hidden surface")
        });
        let mut state = ShellState::new(Locale::English);
        state
            .document_mut()
            .session_mut()
            .execute(EditorCommand::insert_text("a".repeat(200)))
            .expect("source");
        let revision = state.document().session().revision();
        let mut render = RenderHost::new(window.0, Appearance::Light).expect("renderer");
        render.render(&mut state).expect("first Present");
        assert!(has_decoration(
            &mut render,
            &state,
            EditorDecorationPrimitiveRole::Caret
        ));
        let layout = render.layout.as_ref().expect("geometry");
        assert_eq!(
            layout.config().max_width(),
            render.renderer.surface().logical_width() as f32
        );
        let narrow_lines: usize = layout
            .blocks()
            .iter()
            .map(|block| {
                assert!(
                    block
                        .layout()
                        .lines()
                        .iter()
                        .all(|line| line.bounds().height() >= BODY_FONT_SIZE)
                );
                block.layout().lines().len()
            })
            .sum();
        assert!(narrow_lines > 1);
        unsafe {
            MoveWindow(window.0, 0, 0, 1280, 480, false).expect("resize");
        }
        render
            .sync_surface(window.0, Appearance::Light)
            .expect("sync surface");
        render.render(&mut state).expect("resized Present");
        let layout = render.layout.as_ref().expect("resized geometry");
        let wide_lines: usize = layout
            .blocks()
            .iter()
            .map(|block| block.layout().lines().len())
            .sum();
        assert!(
            wide_lines < narrow_lines,
            "wrap width must track the resized surface"
        );
        assert_eq!(
            layout.config().max_width(),
            render.renderer.surface().logical_width() as f32
        );
        assert_eq!(state.document().session().revision(), revision);
        let snapshot = state.document().session().snapshot();
        let selection = EditorSelection::range(
            &snapshot,
            ByteOffset::ZERO,
            snapshot.len_bytes(),
            CaretAffinity::Downstream,
        )
        .expect("selection");
        state
            .document_mut()
            .session_mut()
            .set_selection(selection)
            .expect("select source");
        render.render(&mut state).expect("selected Present");
        assert!(has_decoration(
            &mut render,
            &state,
            EditorDecorationPrimitiveRole::Selection
        ));
        state.set_appearance(Appearance::Dark);
        render
            .sync_surface(window.0, Appearance::Dark)
            .expect("dark theme");
        render
            .render(&mut state)
            .expect("dark Present and layout adoption");
        assert_eq!(
            state
                .document()
                .session()
                .viewport_config()
                .layout()
                .theme(),
            Appearance::Dark.theme_id()
        );
        assert_eq!(state.document().session().revision(), revision);
    }

    #[test]
    fn native_group6_text_provider_actions_contrast_and_lifetime() {
        use windows::Win32::System::Ole::{SafeArrayDestroy, SafeArrayGetUBound};
        use windows::Win32::UI::Accessibility::*;
        use windows::core::{BSTR, Interface};
        use yu_scene::{ContrastPalette, Rgba8};
        let _com = ComApartment::initialize().expect("COM");
        register_classes().expect("process-wide classes");
        let mut state = ShellState::new(Locale::English);
        let source =
            "# Heading\n\nA😀B 👩‍💻 e\u{301} 中文\n\n- [ ] Task\n\n[Link](https://example.com)\n";
        state
            .document_mut()
            .session_mut()
            .execute(EditorCommand::insert_text(source))
            .expect("fixture");
        let mut app = Box::new(AppWindow::new(state));
        let app_ptr = ptr::from_mut(app.as_mut());
        let window = Window(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    MAIN_CLASS,
                    w!("Yu group6 regression"),
                    WS_POPUP | WS_CLIPCHILDREN,
                    60,
                    60,
                    1200,
                    900,
                    None,
                    None,
                    None,
                    Some(app_ptr.cast()),
                )
            }
            .expect("window"),
        );
        app.initialize(window.0).expect("native editor");
        unsafe {
            let _ = ShowWindow(
                window.0,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
            );
        }
        app.publish_accessibility().expect("visible geometry");
        let provider = app.accessibility.as_ref().expect("host").provider.clone();
        let text: ITextProvider2 = provider.cast().expect("TextPattern2");
        let full = unsafe { text.DocumentRange() }.expect("document range");
        let heading_style = unsafe {
            full.FindAttribute(
                UIA_StyleIdAttributeId,
                &windows::core::VARIANT::from(StyleId_Heading1.0),
                false,
            )
        }
        .expect("heading in mixed attributes");
        assert!(
            unsafe { heading_style.GetText(-1) }
                .expect("heading source")
                .to_string()
                .starts_with("# Heading")
        );
        assert_eq!(
            unsafe { full.GetText(-1) }.expect("text").to_string(),
            source
        );
        assert_eq!(
            unsafe { text.SupportedTextSelection() }.expect("selection support"),
            SupportedTextSelection_Single
        );
        let emoji = unsafe { full.FindText(&BSTR::from("😀"), false, false) }.expect("find emoji");
        assert_eq!(
            unsafe { emoji.GetText(1) }
                .expect("bounded text")
                .to_string(),
            "",
            "do not truncate a surrogate"
        );
        assert_eq!(
            unsafe { emoji.GetText(2) }
                .expect("bounded text")
                .to_string(),
            "😀"
        );
        let copy = unsafe { emoji.Clone() }.expect("clone");
        assert!(unsafe { copy.Compare(&emoji) }.expect("compare").as_bool());
        unsafe { emoji.Select() }.expect("native selection marshalled to HWND");
        let selected = app.state.document().session().selection().ordered_range();
        assert_eq!(
            &app.state.document().session().snapshot().as_str()
                [selected.start().get() as usize..selected.end().get() as usize],
            "😀"
        );
        let rectangles = unsafe { emoji.GetBoundingRectangles() }.expect("geometry");
        assert!(unsafe { SafeArrayGetUBound(rectangles, 1) }.expect("geometry count") >= 3);
        unsafe { SafeArrayDestroy(rectangles) }.expect("free geometry");
        let visible = unsafe { text.GetVisibleRanges() }.expect("visible ranges");
        assert!(unsafe { SafeArrayGetUBound(visible, 1) }.expect("range count") >= 0);
        unsafe { SafeArrayDestroy(visible) }.expect("free ranges");
        let zwj = unsafe { full.FindText(&BSTR::from("👩‍💻"), false, false) }.expect("ZWJ");
        unsafe { zwj.ExpandToEnclosingUnit(TextUnit_Character) }.expect("grapheme");
        assert_eq!(
            unsafe { zwj.GetText(-1) }
                .expect("grapheme text")
                .to_string(),
            "👩‍💻"
        );
        assert_eq!(
            unsafe { zwj.Move(TextUnit_Character, 1) }.expect("character movement"),
            1
        );
        assert_eq!(
            unsafe { zwj.GetText(-1) }.expect("next unit").to_string(),
            " "
        );
        let fragment: IRawElementProviderFragment = provider.cast().expect("fragment");
        let heading =
            unsafe { fragment.Navigate(NavigateDirection_FirstChild) }.expect("semantic heading");
        let heading: IRawElementProviderSimple = heading.cast().expect("semantic simple");
        assert_eq!(
            BSTR::try_from(
                &unsafe { heading.GetPropertyValue(UIA_NamePropertyId) }.expect("heading name")
            )
            .expect("string")
            .to_string(),
            "Heading"
        );
        assert_eq!(
            i32::try_from(
                &unsafe { heading.GetPropertyValue(UIA_HeadingLevelPropertyId) }
                    .expect("heading level")
            )
            .expect("level"),
            HeadingLevel1.0
        );
        let revision = app.state.document().session().revision();
        let colors = ContrastPalette {
            background: Rgba8::black(),
            foreground: Rgba8::new(255, 255, 0, 255),
            selection: Rgba8::new(0, 0, 255, 255),
            selected_text: Rgba8::white(),
        };
        let word = unsafe { full.FindText(&BSTR::from("中文"), false, false) }.expect("word");
        unsafe { word.Select() }.expect("select contrast text");
        app.contrast = Some(colors);
        app.render
            .as_mut()
            .expect("render")
            .renderer
            .request_frame_capture();
        app.render_current().expect("contrast render");
        let pixels = app
            .render
            .as_mut()
            .expect("render")
            .renderer
            .take_captured_frame()
            .expect("actual GPU capture");
        let count = |test: fn(&[u8; 4]) -> bool| {
            pixels
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| test(pixel))
                .count()
        };
        assert!(
            count(|p| p[0] < 10 && p[1] < 10 && p[2] < 10) > 1000,
            "opaque black background"
        );
        assert!(
            count(|p| p[0] > 180 && p[1] > 180 && p[2] < 30) > 100,
            "yellow body text"
        );
        assert!(
            count(|p| p[0] < 30 && p[1] < 30 && p[2] > 180) > 100,
            "opaque blue selection"
        );
        assert!(
            count(|p| p[0] > 180 && p[1] > 180 && p[2] > 180) > 20,
            "selected white text survives opaque selection"
        );
        assert_eq!(app.state.document().session().revision(), revision);
        assert_eq!(app.state.document().session().snapshot().as_str(), source);
        app.contrast = None;
        app.render_current().expect("restore normal colors");
        assert!(
            app.render
                .as_ref()
                .expect("render")
                .builder
                .config()
                .contrast_palette()
                .is_none()
        );
        app.execute_input_command(EditorCommand::insert_text("X"))
            .expect("edit");
        assert_eq!(
            unsafe { full.GetText(-1) }
                .expect_err("old range invalidated")
                .code()
                .0,
            UIA_E_ELEMENTNOTAVAILABLE as i32
        );
        assert_eq!(
            unsafe { heading.GetPropertyValue(UIA_NamePropertyId) }
                .expect_err("old node invalidated")
                .code()
                .0,
            UIA_E_ELEMENTNOTAVAILABLE as i32
        );
        let new_range = unsafe { text.DocumentRange() }.expect("new range");
        assert!(unsafe { new_range.GetText(-1) }.is_ok());
        unsafe {
            let _ = ShowWindow(window.0, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
        }
        app.publish_accessibility().expect("hidden geometry");
        assert!(
            bool::try_from(
                &unsafe { provider.GetPropertyValue(UIA_IsOffscreenPropertyId) }
                    .expect("hidden state")
            )
            .expect("bool")
        );
        let invisible = unsafe { text.GetVisibleRanges() }.expect("hidden visible ranges");
        assert_eq!(
            unsafe { SafeArrayGetUBound(invisible, 1) }.expect("empty range count"),
            -1
        );
        unsafe { SafeArrayDestroy(invisible) }.expect("free hidden ranges");
        drop(window);
        assert_eq!(
            unsafe { text.DocumentRange() }
                .expect_err("closed HWND unavailable")
                .code()
                .0,
            UIA_E_ELEMENTNOTAVAILABLE as i32
        );
    }

    #[test]
    fn native_sidebar_navigation_reveals_unicode_search_without_editing_source() {
        use windows::Win32::UI::WindowsAndMessaging::{
            GW_CHILD, GetWindow, LB_GETCOUNT, SendMessageW,
        };
        let _com = ComApartment::initialize().expect("COM");
        register_classes().expect("classes");
        let mut state = ShellState::new(Locale::English);
        let source = format!(
            "# First\n\n{}\n\n## **目标**\n\n中文😀 result\n",
            "filler paragraph\n\n".repeat(60)
        );
        state
            .document_mut()
            .session_mut()
            .execute(EditorCommand::insert_text(source.as_str()))
            .expect("source");
        let mut app = Box::new(AppWindow::new(state));
        let app_ptr = std::ptr::from_mut(app.as_mut());
        let window = Window(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                MAIN_CLASS,
                w!("Native sidebar regression"),
                WS_POPUP | WS_CLIPCHILDREN,
                0,
                0,
                1200,
                800,
                None,
                None,
                None,
                Some(app_ptr.cast()),
            )
            .expect("hidden main window")
        });
        app.initialize(window.0)
            .expect("native chrome / render / TSF");
        assert_eq!(
            unsafe { GetWindow(app.hwnd, GW_CHILD) }.expect("top child"),
            app.surface,
            "background canvas must not obscure the GPU surface"
        );
        let revision = app.state.document().session().revision();
        app.handle_chrome_command(ID_OUTLINE, 0)
            .expect("outline tab");
        let list = app.chrome.as_ref().expect("chrome").list;
        assert_eq!(
            unsafe { SendMessageW(list, LB_GETCOUNT, WPARAM(0), LPARAM(0)) }.0,
            2
        );
        app.handle_chrome_command(ID_SEARCH, 0).expect("search tab");
        let query = app.chrome.as_ref().expect("chrome").query;
        set_window_text(query, "中文😀");
        assert_eq!(
            unsafe { SendMessageW(list, LB_GETCOUNT, WPARAM(0), LPARAM(0)) }.0,
            1
        );
        app.chrome.as_ref().expect("chrome").select_first();
        app.activate_panel_row(false)
            .expect("jump to search result");
        let range = app.state.document().session().selection().ordered_range();
        assert_eq!(
            range.start().get(),
            source.find("中文😀").expect("hit") as u64
        );
        assert_eq!(
            range.end().get() - range.start().get(),
            "中文😀".len() as u64
        );
        assert!(
            app.render
                .as_ref()
                .expect("render")
                .builder
                .config()
                .viewport()
                .scroll_y()
                > 0.0,
            "offscreen result must be revealed"
        );
        let render = app.render.as_ref().expect("render");
        let frame = render
            .builder
            .last_publication()
            .expect("published frame")
            .frame();
        let viewport = frame.plan().viewport();
        assert!(
            viewport.y() > 0.0,
            "scene viewport must translate document coordinates"
        );
        assert!(
            frame.scene().scene().primitives().iter().any(|primitive| {
                matches!(primitive, Primitive::EditorDecoration(decoration)
                if decoration.role() == EditorDecorationPrimitiveRole::Selection
                    && decoration.bounds().y() < viewport.y() + viewport.height()
                    && decoration.bounds().y() + decoration.bounds().height() > viewport.y())
            }),
            "selected search result must paint inside the visible scene viewport"
        );
        let mut client_origin = POINT::default();
        assert!(unsafe { ClientToScreen(app.hwnd, &mut client_origin) }.as_bool());
        let view = app.input_screen_ext().expect("inset view");
        assert!(
            view.left > client_origin.x + sidebar_width(app.state.metrics(), app.state.sidebar()),
            "editor must have a real HWND gutter, including TSF coordinates"
        );
        assert!(view.top > client_origin.y);
        let selection = app.input_projection().expect("projection").selection();
        let caret_acp = selection.range.end();
        let caret = app
            .input_text_ext(AcpRange::new(caret_acp, caret_acp).expect("caret range"))
            .expect("caret screen geometry");
        assert!(caret.left >= view.left && caret.left < view.right);
        assert!(caret.top >= view.top && caret.top < view.bottom);
        let hit = app
            .input_acp_from_screen(
                POINT {
                    x: caret.left,
                    y: (caret.top + caret.bottom) / 2,
                },
                false,
            )
            .expect("caret point hit after inset");
        assert!(selection.range.contains(hit));
        assert_eq!(app.state.document().session().revision(), revision);
        assert_eq!(app.state.document().session().snapshot().as_str(), source);
        drop(window);
    }
}
