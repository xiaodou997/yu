#![cfg(target_os = "windows")]
#![allow(non_snake_case)]

use std::cmp;
use std::ptr;
use std::rc::Rc;
use std::slice;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{
    BOOL, E_FAIL, E_INVALIDARG, E_NOTIMPL, E_UNEXPECTED, HWND, LPARAM, POINT, RECT, WPARAM,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, FORMATETC, IDataObject};
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{
    ComObject, Error as WindowsError, HRESULT, IUnknown, Interface, PCWSTR, PWSTR,
    Result as WinResult, implement,
};

use crate::native::AppWindow;
use crate::text_input::{
    AcpError, AcpProjection, AcpRange, AcpSelection, query_insert_range, utf16_prefix_len,
};

pub(crate) const WM_APP_TSF_LOCK: u32 = WM_APP + 2;
const VIEW_COOKIE: u32 = 0;

fn error(code: HRESULT) -> WindowsError {
    WindowsError::from_hresult(code)
}

fn invalid_arg() -> WindowsError {
    WindowsError::from_hresult(E_INVALIDARG)
}

fn no_lock() -> WindowsError {
    error(TS_E_NOLOCK)
}

fn invalid_pos() -> WindowsError {
    error(TS_E_INVALIDPOS)
}

fn map_acp_error(error: AcpError) -> WindowsError {
    match error {
        AcpError::PositionOutOfRange(_)
        | AcpError::InvalidRange { .. }
        | AcpError::InsideSurrogatePair(_) => invalid_pos(),
        AcpError::OffsetOverflow | AcpError::Selection(_) => WindowsError::from_hresult(E_FAIL),
    }
}

#[derive(Clone)]
struct SinkConnection {
    identity: IUnknown,
    sink: ITextStoreACPSink,
    mask: u32,
}

#[derive(Default)]
struct LockState {
    current: u32,
    pending: u32,
}

struct TextStoreState {
    app: usize,
    hwnd: HWND,
    sink: Mutex<Option<SinkConnection>>,
    lock: Mutex<LockState>,
    composition_session: AtomicBool,
}

impl TextStoreState {
    fn new(app: *mut AppWindow, hwnd: HWND) -> Self {
        Self {
            app: app as usize,
            hwnd,
            sink: Mutex::new(None),
            lock: Mutex::new(LockState::default()),
            composition_session: AtomicBool::new(false),
        }
    }

    fn with_app<T>(&self, callback: impl FnOnce(&mut AppWindow) -> WinResult<T>) -> WinResult<T> {
        let app = self.app as *mut AppWindow;
        if app.is_null() {
            return Err(WindowsError::from_hresult(E_UNEXPECTED));
        }
        // AppWindow is boxed for the lifetime of TsfHost. COM callbacks execute
        // on the same STA. The mutable reference never escapes this callback.
        callback(unsafe { &mut *app })
    }

    fn projection(&self) -> WinResult<AcpProjection> {
        self.with_app(|app| {
            app.input_projection()
                .map_err(|_| WindowsError::from_hresult(E_FAIL))
        })
    }

    fn has_read_lock(&self) -> bool {
        self.lock
            .lock()
            .is_ok_and(|state| state.current & TS_LF_READ.0 != 0)
    }

    fn has_write_lock(&self) -> bool {
        self.lock
            .lock()
            .is_ok_and(|state| state.current & TS_LF_READWRITE.0 == TS_LF_READWRITE.0)
    }

    fn require_read_lock(&self) -> WinResult<()> {
        self.has_read_lock().then_some(()).ok_or_else(no_lock)
    }

    fn require_write_lock(&self) -> WinResult<()> {
        self.has_write_lock().then_some(()).ok_or_else(no_lock)
    }

    fn sink_snapshot(&self) -> Option<SinkConnection> {
        self.sink.lock().ok()?.clone()
    }

    fn post_pending_lock(&self) {
        unsafe {
            let _ = PostMessageW(self.hwnd, WM_APP_TSF_LOCK, WPARAM(0), LPARAM(0));
        }
    }

    fn grant(&self, flags: u32) -> WinResult<HRESULT> {
        let sink = self
            .sink_snapshot()
            .ok_or_else(|| WindowsError::from_hresult(E_UNEXPECTED))?;
        {
            let mut lock = self
                .lock
                .lock()
                .map_err(|_| WindowsError::from_hresult(E_FAIL))?;
            if lock.current != 0 {
                return Ok(TS_S_ASYNC);
            }
            lock.current = flags & TS_LF_READWRITE.0;
            if lock.current == 0 {
                lock.current = TS_LF_READ.0;
            }
        }

        let result = unsafe { sink.sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(flags)) };
        let pending = {
            let mut lock = self
                .lock
                .lock()
                .map_err(|_| WindowsError::from_hresult(E_FAIL))?;
            lock.current = 0;
            let pending = lock.pending;
            lock.pending = 0;
            pending
        };
        if pending != 0 {
            if let Ok(mut lock) = self.lock.lock() {
                lock.pending = pending;
            }
            self.post_pending_lock();
        }
        result.map(|_| HRESULT(0))
    }

    fn request_lock(&self, flags: u32) -> WinResult<HRESULT> {
        let occupied = {
            let mut lock = self
                .lock
                .lock()
                .map_err(|_| WindowsError::from_hresult(E_FAIL))?;
            if lock.current == 0 {
                false
            } else {
                if flags & TS_LF_SYNC != 0 {
                    return Ok(TS_E_SYNCHRONOUS);
                }
                let requested = flags & TS_LF_READWRITE.0;
                if requested & TS_LF_READWRITE.0 == TS_LF_READWRITE.0 {
                    lock.pending = TS_LF_READWRITE.0;
                } else if lock.pending == 0 {
                    lock.pending = TS_LF_READ.0;
                }
                true
            }
        };
        if occupied {
            Ok(TS_S_ASYNC)
        } else {
            self.grant(flags)
        }
    }

    fn grant_pending(&self) {
        let pending = match self.lock.lock() {
            Ok(mut lock) if lock.current == 0 && lock.pending != 0 => {
                let pending = lock.pending;
                lock.pending = 0;
                pending
            }
            _ => return,
        };
        let _ = self.grant(pending);
    }

    fn notify_text_change(&self, change: TS_TEXTCHANGE) {
        let Some(connection) = self.sink_snapshot() else {
            return;
        };
        if connection.mask & TS_AS_TEXT_CHANGE == 0 || self.has_read_lock() {
            return;
        }
        unsafe {
            let _ = connection
                .sink
                .OnTextChange(TEXT_STORE_TEXT_CHANGE_FLAGS(0), &change);
        }
    }

    fn notify_selection_change(&self) {
        let Some(connection) = self.sink_snapshot() else {
            return;
        };
        if connection.mask & TS_AS_SEL_CHANGE == 0 || self.has_read_lock() {
            return;
        }
        unsafe {
            let _ = connection.sink.OnSelectionChange();
        }
    }

    fn notify_layout_change(&self) {
        let Some(connection) = self.sink_snapshot() else {
            return;
        };
        if connection.mask & TS_AS_LAYOUT_CHANGE == 0 || self.has_read_lock() {
            return;
        }
        unsafe {
            let _ = connection.sink.OnLayoutChange(TS_LC_CHANGE, VIEW_COOKIE);
        }
    }
}

#[implement(ITextStoreACP, ITfContextOwnerCompositionSink)]
struct TextStore {
    state: Rc<TextStoreState>,
}

impl TextStore {
    fn text_from_ptr(text: &PCWSTR, count: u32) -> WinResult<String> {
        if count == 0 {
            return Ok(String::new());
        }
        if text.is_null() {
            return Err(invalid_arg());
        }
        let count = usize::try_from(count).map_err(|_| invalid_arg())?;
        let units = unsafe { slice::from_raw_parts(text.0, count) };
        String::from_utf16(units).map_err(|_| invalid_arg())
    }

    fn replace(&self, range: AcpRange, text: &str, query_only: bool) -> WinResult<TS_TEXTCHANGE> {
        if query_only {
            let inserted = i32::try_from(text.encode_utf16().count()).map_err(|_| invalid_arg())?;
            return Ok(TS_TEXTCHANGE {
                acpStart: range.start(),
                acpOldEnd: range.end(),
                acpNewEnd: range
                    .start()
                    .checked_add(inserted)
                    .ok_or_else(invalid_arg)?,
            });
        }
        self.state.require_write_lock()?;
        self.state.with_app(|app| {
            app.input_replace(
                range,
                text,
                self.state.composition_session.load(Ordering::SeqCst),
            )
            .map_err(|_| WindowsError::from_hresult(E_FAIL))
        })
    }
}

impl ITextStoreACP_Impl for TextStore_Impl {
    fn AdviseSink(
        &self,
        riid: *const windows::core::GUID,
        punk: Option<&IUnknown>,
        dwmask: u32,
    ) -> WinResult<()> {
        if riid.is_null() || unsafe { *riid } != ITextStoreACPSink::IID {
            return Err(invalid_arg());
        }
        let punk = punk.ok_or_else(invalid_arg)?;
        let sink: ITextStoreACPSink = punk.cast()?;
        let identity: IUnknown = punk.cast()?;

        let mut connection = self
            .state
            .sink
            .lock()
            .map_err(|_| WindowsError::from_hresult(E_FAIL))?;
        if let Some(existing) = connection.as_mut() {
            if Interface::as_raw(&existing.identity) != Interface::as_raw(&identity) {
                return Err(WindowsError::from_hresult(E_FAIL));
            }
            existing.mask = dwmask;
            return Ok(());
        }
        *connection = Some(SinkConnection {
            identity,
            sink,
            mask: dwmask,
        });
        Ok(())
    }

    fn UnadviseSink(&self, punk: Option<&IUnknown>) -> WinResult<()> {
        let punk = punk.ok_or_else(invalid_arg)?;
        let identity: IUnknown = punk.cast()?;
        let mut connection = self
            .state
            .sink
            .lock()
            .map_err(|_| WindowsError::from_hresult(E_FAIL))?;
        let Some(existing) = connection.as_ref() else {
            return Err(WindowsError::from_hresult(E_UNEXPECTED));
        };
        if Interface::as_raw(&existing.identity) != Interface::as_raw(&identity) {
            return Err(WindowsError::from_hresult(E_UNEXPECTED));
        }
        *connection = None;
        Ok(())
    }

    fn RequestLock(&self, dwlockflags: u32) -> WinResult<HRESULT> {
        self.state.request_lock(dwlockflags)
    }

    fn GetStatus(&self) -> WinResult<TS_STATUS> {
        Ok(TS_STATUS {
            dwDynamicFlags: 0,
            dwStaticFlags: 0,
        })
    }

    fn QueryInsert(
        &self,
        acpteststart: i32,
        acptestend: i32,
        _cch: u32,
        pacpresultstart: *mut i32,
        pacpresultend: *mut i32,
    ) -> WinResult<()> {
        self.state.require_read_lock()?;
        if pacpresultstart.is_null() || pacpresultend.is_null() {
            return Err(invalid_arg());
        }
        let projection = self.state.projection()?;
        let range =
            query_insert_range(&projection, acpteststart, acptestend).map_err(map_acp_error)?;
        unsafe {
            *pacpresultstart = range.start();
            *pacpresultend = range.end();
        }
        Ok(())
    }

    fn GetSelection(
        &self,
        ulindex: u32,
        ulcount: u32,
        pselection: *mut TS_SELECTION_ACP,
        pcfetched: *mut u32,
    ) -> WinResult<()> {
        self.state.require_read_lock()?;
        if ulcount == 0 || pselection.is_null() || pcfetched.is_null() {
            return Err(invalid_arg());
        }
        if ulindex != 0 && ulindex != TS_DEFAULT_SELECTION {
            return Err(error(TS_E_NOSELECTION));
        }
        let selection = self.state.projection()?.selection();
        unsafe {
            *pselection = TS_SELECTION_ACP {
                acpStart: selection.range.start(),
                acpEnd: selection.range.end(),
                style: TS_SELECTIONSTYLE {
                    ase: if selection.range.start() == selection.range.end() {
                        TS_AE_NONE
                    } else if selection.caret_at_start {
                        TS_AE_START
                    } else {
                        TS_AE_END
                    },
                    fInterimChar: BOOL(0),
                },
            };
            *pcfetched = 1;
        }
        Ok(())
    }

    fn SetSelection(&self, ulcount: u32, pselection: *const TS_SELECTION_ACP) -> WinResult<()> {
        self.state.require_write_lock()?;
        if ulcount != 1 || pselection.is_null() {
            return Err(invalid_arg());
        }
        let selection = unsafe { *pselection };
        let range = self
            .state
            .projection()?
            .normalized_range(selection.acpStart, selection.acpEnd)
            .map_err(map_acp_error)?;
        self.state.with_app(|app| {
            app.input_set_selection(AcpSelection {
                range,
                caret_at_start: selection.style.ase == TS_AE_START,
            })
            .map_err(|_| WindowsError::from_hresult(E_FAIL))
        })
    }

    fn GetText(
        &self,
        acpstart: i32,
        acpend: i32,
        pchplain: PWSTR,
        cchplainreq: u32,
        pcchplainret: *mut u32,
        prgruninfo: *mut TS_RUNINFO,
        cruninforeq: u32,
        pcruninforet: *mut u32,
        pacpnext: *mut i32,
    ) -> WinResult<()> {
        self.state.require_read_lock()?;
        if pcchplainret.is_null() || pcruninforet.is_null() || pacpnext.is_null() {
            return Err(invalid_arg());
        }
        let projection = self.state.projection()?;
        let range = projection
            .normalized_range(acpstart, acpend)
            .map_err(map_acp_error)?;
        let start = usize::try_from(range.start()).map_err(|_| invalid_arg())?;
        let end = usize::try_from(range.end()).map_err(|_| invalid_arg())?;
        let source = projection.text().get(start..end).ok_or_else(invalid_pos)?;
        let requested = usize::try_from(cchplainreq).map_err(|_| invalid_arg())?;
        let copied = utf16_prefix_len(source, cmp::min(requested, source.len()));
        if copied > 0 {
            if pchplain.is_null() {
                return Err(invalid_arg());
            }
            unsafe {
                ptr::copy_nonoverlapping(source.as_ptr(), pchplain.0, copied);
            }
        }

        let mut run_count = 0_u32;
        if cruninforeq > 0 && !prgruninfo.is_null() && !source.is_empty() {
            unsafe {
                *prgruninfo = TS_RUNINFO {
                    uCount: u32::try_from(copied).map_err(|_| invalid_arg())?,
                    r#type: TS_RT_PLAIN,
                };
            }
            run_count = 1;
        }

        unsafe {
            *pcchplainret = u32::try_from(copied).map_err(|_| invalid_arg())?;
            *pcruninforet = run_count;
            *pacpnext = range
                .start()
                .checked_add(i32::try_from(copied).map_err(|_| invalid_arg())?)
                .ok_or_else(invalid_arg)?;
        }
        Ok(())
    }

    fn SetText(
        &self,
        _dwflags: u32,
        acpstart: i32,
        acpend: i32,
        pchtext: &PCWSTR,
        cch: u32,
    ) -> WinResult<TS_TEXTCHANGE> {
        self.state.require_write_lock()?;
        let projection = self.state.projection()?;
        let range = projection
            .normalized_range(acpstart, acpend)
            .map_err(map_acp_error)?;
        let text = TextStore::text_from_ptr(pchtext, cch)?;
        self.replace(range, &text, false)
    }

    fn GetFormattedText(&self, _acpstart: i32, _acpend: i32) -> WinResult<IDataObject> {
        Err(WindowsError::from_hresult(E_NOTIMPL))
    }

    fn GetEmbedded(
        &self,
        _acppos: i32,
        _rguidservice: *const windows::core::GUID,
        _riid: *const windows::core::GUID,
    ) -> WinResult<IUnknown> {
        Err(error(TS_E_NOSERVICE))
    }

    fn QueryInsertEmbedded(
        &self,
        _pguidservice: *const windows::core::GUID,
        _pformatetc: *const FORMATETC,
    ) -> WinResult<BOOL> {
        Ok(BOOL(0))
    }

    fn InsertEmbedded(
        &self,
        _dwflags: u32,
        _acpstart: i32,
        _acpend: i32,
        _pdataobject: Option<&IDataObject>,
    ) -> WinResult<TS_TEXTCHANGE> {
        Err(WindowsError::from_hresult(E_NOTIMPL))
    }

    fn InsertTextAtSelection(
        &self,
        dwflags: u32,
        pchtext: &PCWSTR,
        cch: u32,
        pacpstart: *mut i32,
        pacpend: *mut i32,
        pchange: *mut TS_TEXTCHANGE,
    ) -> WinResult<()> {
        self.state.require_write_lock()?;
        let text = TextStore::text_from_ptr(pchtext, cch)?;
        let selection = self.state.projection()?.selection().range;
        let query_only = dwflags & TS_IAS_QUERYONLY != 0;
        let change = self.replace(selection, &text, query_only)?;
        if dwflags & TS_IAS_NOQUERY == 0 {
            if pacpstart.is_null() || pacpend.is_null() {
                return Err(invalid_arg());
            }
            unsafe {
                *pacpstart = change.acpStart;
                *pacpend = change.acpNewEnd;
            }
        }
        if !pchange.is_null() {
            unsafe {
                *pchange = change;
            }
        }
        Ok(())
    }

    fn InsertEmbeddedAtSelection(
        &self,
        _dwflags: u32,
        _pdataobject: Option<&IDataObject>,
        _pacpstart: *mut i32,
        _pacpend: *mut i32,
        _pchange: *mut TS_TEXTCHANGE,
    ) -> WinResult<()> {
        Err(WindowsError::from_hresult(E_NOTIMPL))
    }

    fn RequestSupportedAttrs(
        &self,
        _dwflags: u32,
        _cfilterattrs: u32,
        _pafilterattrs: *const windows::core::GUID,
    ) -> WinResult<()> {
        Ok(())
    }

    fn RequestAttrsAtPosition(
        &self,
        _acppos: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const windows::core::GUID,
        _dwflags: u32,
    ) -> WinResult<()> {
        self.state.require_read_lock()
    }

    fn RequestAttrsTransitioningAtPosition(
        &self,
        _acppos: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const windows::core::GUID,
        _dwflags: u32,
    ) -> WinResult<()> {
        self.state.require_read_lock()
    }

    fn FindNextAttrTransition(
        &self,
        acpstart: i32,
        acphalt: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const windows::core::GUID,
        _dwflags: u32,
        pacpnext: *mut i32,
        pffound: *mut BOOL,
        plfoundoffset: *mut i32,
    ) -> WinResult<()> {
        self.state.require_read_lock()?;
        if pacpnext.is_null() || pffound.is_null() || plfoundoffset.is_null() {
            return Err(invalid_arg());
        }
        let projection = self.state.projection()?;
        projection
            .normalized_range(acpstart, acphalt)
            .map_err(map_acp_error)?;
        unsafe {
            *pacpnext = acphalt;
            *pffound = BOOL(0);
            *plfoundoffset = 0;
        }
        Ok(())
    }

    fn RetrieveRequestedAttrs(
        &self,
        _ulcount: u32,
        _paattrvals: *mut TS_ATTRVAL,
        pcfetched: *mut u32,
    ) -> WinResult<()> {
        if pcfetched.is_null() {
            return Err(invalid_arg());
        }
        unsafe {
            *pcfetched = 0;
        }
        Ok(())
    }

    fn GetEndACP(&self) -> WinResult<i32> {
        self.state.require_read_lock()?;
        Ok(self.state.projection()?.end_acp())
    }

    fn GetActiveView(&self) -> WinResult<u32> {
        Ok(VIEW_COOKIE)
    }

    fn GetACPFromPoint(
        &self,
        vcview: u32,
        ptscreen: *const POINT,
        _dwflags: u32,
    ) -> WinResult<i32> {
        self.state.require_read_lock()?;
        if vcview != VIEW_COOKIE || ptscreen.is_null() {
            return Err(invalid_arg());
        }
        self.state.with_app(|app| {
            app.input_acp_from_screen(unsafe { *ptscreen })
                .map_err(|_| error(TS_E_NOLAYOUT))
        })
    }

    fn GetTextExt(
        &self,
        vcview: u32,
        acpstart: i32,
        acpend: i32,
        prc: *mut RECT,
        pfclipped: *mut BOOL,
    ) -> WinResult<()> {
        self.state.require_read_lock()?;
        if vcview != VIEW_COOKIE || prc.is_null() || pfclipped.is_null() {
            return Err(invalid_arg());
        }
        let range = self
            .state
            .projection()?
            .normalized_range(acpstart, acpend)
            .map_err(map_acp_error)?;
        let rect = self
            .state
            .with_app(|app| app.input_text_ext(range).map_err(|_| error(TS_E_NOLAYOUT)))?;
        unsafe {
            *prc = rect;
            *pfclipped = BOOL(0);
        }
        Ok(())
    }

    fn GetScreenExt(&self, vcview: u32) -> WinResult<RECT> {
        if vcview != VIEW_COOKIE {
            return Err(invalid_arg());
        }
        self.state
            .with_app(|app| app.input_screen_ext().map_err(|_| error(TS_E_NOLAYOUT)))
    }

    fn GetWnd(&self, vcview: u32) -> WinResult<HWND> {
        if vcview != VIEW_COOKIE {
            return Err(invalid_arg());
        }
        Ok(self.state.hwnd)
    }
}

impl ITfContextOwnerCompositionSink_Impl for TextStore_Impl {
    fn OnStartComposition(&self, _composition: Option<&ITfCompositionView>) -> WinResult<BOOL> {
        self.state.composition_session.store(true, Ordering::SeqCst);
        Ok(BOOL(1))
    }

    fn OnUpdateComposition(
        &self,
        _composition: Option<&ITfCompositionView>,
        _range_new: Option<&ITfRange>,
    ) -> WinResult<()> {
        Ok(())
    }

    fn OnEndComposition(&self, _composition: Option<&ITfCompositionView>) -> WinResult<()> {
        self.state
            .composition_session
            .store(false, Ordering::SeqCst);
        self.state.with_app(|app| {
            app.input_end_composition()
                .map_err(|_| WindowsError::from_hresult(E_FAIL))
        })
    }
}

pub(crate) struct TsfHost {
    thread_mgr: ITfThreadMgr,
    document_mgr: ITfDocumentMgr,
    _context: ITfContext,
    keystroke_mgr: ITfKeystrokeMgr,
    _store: ITextStoreACP,
    state: Rc<TextStoreState>,
}

impl TsfHost {
    pub(crate) fn new(app: *mut AppWindow, hwnd: HWND) -> WinResult<Self> {
        let thread_mgr: ITfThreadMgr =
            unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)? };
        let client_id = unsafe { thread_mgr.Activate()? };
        let document_mgr = unsafe { thread_mgr.CreateDocumentMgr()? };
        let state = Rc::new(TextStoreState::new(app, hwnd));
        let store: ITextStoreACP = ComObject::new(TextStore {
            state: Rc::clone(&state),
        })
        .into_interface();

        let mut context = None;
        let mut edit_cookie = 0_u32;
        unsafe {
            document_mgr.CreateContext(client_id, 0, &store, &mut context, &mut edit_cookie)?;
        }
        let context = context.ok_or_else(|| WindowsError::from_hresult(E_UNEXPECTED))?;
        unsafe {
            document_mgr.Push(&context)?;
            let _ = thread_mgr.AssociateFocus(hwnd, &document_mgr)?;
        }
        let keystroke_mgr: ITfKeystrokeMgr = thread_mgr.cast()?;
        Ok(Self {
            thread_mgr,
            document_mgr,
            _context: context,
            keystroke_mgr,
            _store: store,
            state,
        })
    }

    pub(crate) fn focus(&self) {
        unsafe {
            let _ = self.thread_mgr.SetFocus(&self.document_mgr);
        }
    }

    pub(crate) fn filter_key_down(&self, wparam: WPARAM, lparam: LPARAM) -> bool {
        unsafe {
            self.keystroke_mgr
                .TestKeyDown(wparam, lparam)
                .is_ok_and(|eaten| {
                    eaten.as_bool()
                        && self
                            .keystroke_mgr
                            .KeyDown(wparam, lparam)
                            .is_ok_and(|handled| handled.as_bool())
                })
        }
    }

    pub(crate) fn filter_key_up(&self, wparam: WPARAM, lparam: LPARAM) -> bool {
        unsafe {
            self.keystroke_mgr
                .TestKeyUp(wparam, lparam)
                .is_ok_and(|eaten| {
                    eaten.as_bool()
                        && self
                            .keystroke_mgr
                            .KeyUp(wparam, lparam)
                            .is_ok_and(|handled| handled.as_bool())
                })
        }
    }

    pub(crate) fn grant_pending_lock(&self) {
        self.state.grant_pending();
    }

    pub(crate) fn notify_external_change(
        &self,
        before_end: i32,
        after_end: i32,
        selection_changed: bool,
    ) {
        self.state.notify_text_change(TS_TEXTCHANGE {
            acpStart: 0,
            acpOldEnd: before_end,
            acpNewEnd: after_end,
        });
        if selection_changed {
            self.state.notify_selection_change();
        }
        self.state.notify_layout_change();
    }

    pub(crate) fn notify_selection_change(&self) {
        self.state.notify_selection_change();
        self.state.notify_layout_change();
    }

    pub(crate) fn notify_layout_change(&self) {
        self.state.notify_layout_change();
    }
}

impl Drop for TsfHost {
    fn drop(&mut self) {
        unsafe {
            let _ = self.document_mgr.Pop(TF_POPF_ALL);
            let _ = self.thread_mgr.Deactivate();
        }
    }
}
