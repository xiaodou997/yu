//! UIA exposes immutable shared snapshots. Native mutations are marshalled to
//! the owning HWND; no COM object keeps an AppWindow pointer.
#![allow(non_snake_case, non_upper_case_globals)]
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use windows::Win32::Foundation::{BOOL, E_FAIL, E_INVALIDARG, E_POINTER, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement};
use windows::Win32::System::Variant::{VT_I4, VT_R8, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{
    SMTO_ABORTIFHUNG, SMTO_BLOCK, SendMessageTimeoutW, WM_APP,
};
use windows::core::{
    BSTR, ComObject, Error, HRESULT, IUnknown, IUnknown_Vtbl, Interface, Result, VARIANT,
    implement, interface,
};
use yu_core::{ByteOffset, Revision, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    AccessibilitySemanticKind as Kind, AccessibilitySemanticSnapshot, AccessibilityTextSnapshot,
    AccessibilityTextUnit, Bias, LayoutPoint, LayoutSnapshot,
};
use yu_text::TextSnapshot;

pub(crate) const WM_APP_UIA_ACTION: u32 = WM_APP + 3;
fn unavailable() -> Error {
    Error::from_hresult(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32))
}
fn invalid() -> Error {
    Error::from_hresult(E_INVALIDARG)
}
fn failed() -> Error {
    Error::from_hresult(E_FAIL)
}
fn unsupported() -> Error {
    Error::from_hresult(HRESULT(UIA_E_NOTSUPPORTED as i32))
}

// windows 0.58 projects nullable interface outputs as non-null Result<T>.
// These ABI-identical interfaces let us return S_OK + null for absent patterns,
// neighbours and searches without manufacturing invalid Rust interface values.
#[interface("d6dd68d1-86fd-4332-8666-9abedea2d24c")]
unsafe trait SimpleAbi: IUnknown {
    fn ProviderOptions(&self, out: *mut ProviderOptions) -> HRESULT;
    fn GetPatternProvider(&self, id: UIA_PATTERN_ID, out: *mut *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID, out: *mut VARIANT) -> HRESULT;
    fn HostRawElementProvider(&self, out: *mut *mut c_void) -> HRESULT;
}
#[interface("f7063da8-8359-439c-9297-bbc5299a7d87")]
unsafe trait FragmentAbi: IUnknown {
    fn Navigate(&self, direction: NavigateDirection, out: *mut *mut c_void) -> HRESULT;
    fn GetRuntimeId(&self, out: *mut *mut SAFEARRAY) -> HRESULT;
    fn BoundingRectangle(&self, out: *mut UiaRect) -> HRESULT;
    fn GetEmbeddedFragmentRoots(&self, out: *mut *mut SAFEARRAY) -> HRESULT;
    fn SetFocus(&self) -> HRESULT;
    fn FragmentRoot(&self, out: *mut *mut c_void) -> HRESULT;
}
#[interface("620ce2a5-ab8f-40a9-86cb-de3c75599b58")]
unsafe trait RootAbi: IUnknown {
    fn ElementProviderFromPoint(&self, x: f64, y: f64, out: *mut *mut c_void) -> HRESULT;
    fn GetFocus(&self, out: *mut *mut c_void) -> HRESULT;
}
#[interface("5347ad7b-c355-46f8-aff5-909033582f63")]
unsafe trait RangeAbi: IUnknown {
    fn Clone(&self, out: *mut *mut c_void) -> HRESULT;
    fn Compare(&self, other: *mut c_void, out: *mut BOOL) -> HRESULT;
    fn CompareEndpoints(
        &self,
        end: TextPatternRangeEndpoint,
        other: *mut c_void,
        other_end: TextPatternRangeEndpoint,
        out: *mut i32,
    ) -> HRESULT;
    fn ExpandToEnclosingUnit(&self, unit: TextUnit) -> HRESULT;
    fn FindAttribute(
        &self,
        id: UIA_TEXTATTRIBUTE_ID,
        value: *const VARIANT,
        backwards: BOOL,
        out: *mut *mut c_void,
    ) -> HRESULT;
    fn FindText(
        &self,
        text: *const u16,
        backwards: BOOL,
        ignore_case: BOOL,
        out: *mut *mut c_void,
    ) -> HRESULT;
    fn GetAttributeValue(&self, id: UIA_TEXTATTRIBUTE_ID, out: *mut VARIANT) -> HRESULT;
    fn GetBoundingRectangles(&self, out: *mut *mut SAFEARRAY) -> HRESULT;
    fn GetEnclosingElement(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetText(&self, maximum: i32, out: *mut BSTR) -> HRESULT;
    fn Move(&self, unit: TextUnit, count: i32, out: *mut i32) -> HRESULT;
    fn MoveEndpointByUnit(
        &self,
        endpoint: TextPatternRangeEndpoint,
        unit: TextUnit,
        count: i32,
        out: *mut i32,
    ) -> HRESULT;
    fn MoveEndpointByRange(
        &self,
        endpoint: TextPatternRangeEndpoint,
        other: *mut c_void,
        target: TextPatternRangeEndpoint,
    ) -> HRESULT;
    fn Select(&self) -> HRESULT;
    fn AddToSelection(&self) -> HRESULT;
    fn RemoveFromSelection(&self) -> HRESULT;
    fn ScrollIntoView(&self, top: BOOL) -> HRESULT;
    fn GetChildren(&self, out: *mut *mut SAFEARRAY) -> HRESULT;
}

unsafe fn output<T>(out: *mut T, result: Result<T>) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    match result {
        Ok(value) => {
            unsafe {
                out.write(value);
            }
            HRESULT(0)
        }
        Err(error) => error.code(),
    }
}
unsafe fn interface_output<T: Interface>(
    out: *mut *mut c_void,
    result: Result<Option<T>>,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe {
        out.write(ptr::null_mut());
    }
    unsafe {
        output(
            out,
            result.map(|value| value.map_or(ptr::null_mut(), Interface::into_raw)),
        )
    }
}
fn array<T>(
    kind: windows::Win32::System::Variant::VARENUM,
    values: &[T],
) -> Result<*mut SAFEARRAY> {
    let array = unsafe {
        SafeArrayCreateVector(kind, 0, u32::try_from(values.len()).map_err(|_| invalid())?)
    };
    if array.is_null() {
        return Err(failed());
    }
    for (index, value) in values.iter().enumerate() {
        let result =
            unsafe { SafeArrayPutElement(array, &(index as i32), ptr::from_ref(value).cast()) };
        if let Err(error) = result {
            unsafe {
                let _ = SafeArrayDestroy(array);
            }
            return Err(error);
        }
    }
    Ok(array)
}
fn interfaces<T: Interface>(values: &[T]) -> Result<*mut SAFEARRAY> {
    let array = unsafe {
        SafeArrayCreateVector(
            VT_UNKNOWN,
            0,
            u32::try_from(values.len()).map_err(|_| invalid())?,
        )
    };
    if array.is_null() {
        return Err(failed());
    }
    for (index, value) in values.iter().enumerate() {
        // VT_UNKNOWN is passed as the interface pointer itself, not &pointer.
        if let Err(error) = unsafe { SafeArrayPutElement(array, &(index as i32), value.as_raw()) } {
            unsafe {
                let _ = SafeArrayDestroy(array);
            }
            return Err(error);
        }
    }
    Ok(array)
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub identity: u64,
    pub source: TextSnapshot,
    pub text: AccessibilityTextSnapshot,
    pub semantic: Arc<AccessibilitySemanticSnapshot>,
    pub labels: Arc<HashMap<u32, String>>,
    pub layout: Arc<LayoutSnapshot>,
    pub bounds: UiaRect,
    pub scale: f64,
    pub scroll_y: f32,
    pub focused: bool,
    pub visible: bool,
    pub caret: ByteOffset,
    pub name: String,
}
impl Snapshot {
    fn source_range(&self, start: u64, end: u64) -> Result<TextRange> {
        let range =
            Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end)).ok_or_else(invalid)?;
        self.text
            .bind_range(range)
            .and_then(|r| self.text.source_range(r))
            .map_err(|_| invalid())
    }
    fn utf16(&self, byte: ByteOffset) -> Result<u64> {
        self.source
            .utf16_offset(byte)
            .map(|v| v.get())
            .map_err(|_| invalid())
    }
    fn screen_rect(&self, x: f32, y: f32, width: f32, height: f32) -> Option<UiaRect> {
        if !self.visible {
            return None;
        }
        let left = (self.bounds.left + f64::from(x) * self.scale).max(self.bounds.left);
        let top =
            (self.bounds.top + f64::from(y - self.scroll_y) * self.scale).max(self.bounds.top);
        let right = (self.bounds.left + f64::from(x + width) * self.scale)
            .min(self.bounds.left + self.bounds.width);
        let bottom = (self.bounds.top + f64::from(y + height - self.scroll_y) * self.scale)
            .min(self.bounds.top + self.bounds.height);
        (right > left && bottom > top).then_some(UiaRect {
            left,
            top,
            width: right - left,
            height: bottom - top,
        })
    }
    fn rectangles(&self, source: TextRange) -> Result<Vec<UiaRect>> {
        let mut rectangles: Vec<UiaRect> = Vec::new();
        if source.is_empty() {
            if let Some(block) = self.layout.block_for_source(source.start()) {
                let caret = block
                    .layout()
                    .caret_for_source(source.start(), Bias::After)
                    .map_err(|_| failed())?;
                let point = block.document_point(caret.point());
                if let Some(rect) =
                    self.screen_rect(point.x(), point.y(), 1.0, block.caret_height(caret))
                {
                    rectangles.push(rect);
                }
            }
            return Ok(rectangles);
        }
        for block in self.layout.blocks() {
            for cluster in block.layout().layout().clusters() {
                let visual = cluster.visual();
                let start = block
                    .layout()
                    .visual()
                    .visual_to_source(visual.start(), Bias::After)
                    .map_err(|_| failed())?;
                let end = block
                    .layout()
                    .visual()
                    .visual_to_source(visual.end(), Bias::Before)
                    .map_err(|_| failed())?;
                if start >= source.end() || end <= source.start() {
                    continue;
                }
                let line = &block.layout().layout().lines()[cluster.line()];
                if let Some(rect) = self.screen_rect(
                    cluster.x(),
                    block.content_y() + line.y(),
                    cluster.width(),
                    line.height(),
                ) {
                    // Preserve disjoint bidi rectangles, merging only adjacent
                    // clusters on the same visual line.
                    if let Some(previous) = rectangles.last_mut()
                        && (previous.top - rect.top).abs() < 0.1
                        && (previous.height - rect.height).abs() < 0.1
                        && rect.left <= previous.left + previous.width + 0.5
                        && rect.left >= previous.left - 0.5
                    {
                        previous.width = (rect.left + rect.width)
                            .max(previous.left + previous.width)
                            - previous.left;
                    } else {
                        rectangles.push(rect);
                    }
                }
            }
        }
        Ok(rectangles)
    }
    fn at_point(&self, point: &UiaPoint) -> Result<u64> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(invalid());
        }
        let local = LayoutPoint::new(
            ((point.x - self.bounds.left) / self.scale) as f32,
            ((point.y - self.bounds.top) / self.scale) as f32 + self.scroll_y,
        );
        let (_, hit) = self
            .layout
            .hit_test(local)
            .map_err(|_| failed())?
            .ok_or_else(unavailable)?;
        self.utf16(hit.source())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Action {
    Focus,
    Select {
        identity: u64,
        revision: Revision,
        source: TextRange,
    },
    Scroll {
        identity: u64,
        revision: Revision,
        source: TextRange,
        top: bool,
    },
    Toggle {
        identity: u64,
        revision: Revision,
        block: usize,
    },
}
struct Request {
    action: Action,
    result: Option<HRESULT>,
}
pub(crate) struct Shared {
    hwnd: usize,
    pub snapshot: RwLock<Option<Arc<Snapshot>>>,
    closed: AtomicBool,
    next: AtomicUsize,
    pending: Mutex<HashMap<usize, Request>>,
}
impl Shared {
    fn current(&self) -> Result<Arc<Snapshot>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        self.snapshot
            .read()
            .map_err(|_| failed())?
            .clone()
            .ok_or_else(unavailable)
    }
    fn dispatch(&self, action: Action) -> Result<()> {
        self.current()?;
        let token = self.next.fetch_add(1, Ordering::Relaxed);
        self.pending.lock().map_err(|_| failed())?.insert(
            token,
            Request {
                action,
                result: None,
            },
        );
        let mut result = 0;
        unsafe {
            SendMessageTimeoutW(
                HWND(self.hwnd as *mut _),
                WM_APP_UIA_ACTION,
                WPARAM(token),
                LPARAM(0),
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                5000,
                Some(&mut result),
            );
        }
        let request = self
            .pending
            .lock()
            .map_err(|_| failed())?
            .remove(&token)
            .ok_or_else(unavailable)?;
        request.result.ok_or_else(unavailable)?.ok()
    }
    pub fn action(&self, token: usize) -> Option<Action> {
        self.pending
            .lock()
            .ok()?
            .get(&token)
            .map(|r| r.action.clone())
    }
    pub fn complete(&self, token: usize, result: Result<()>) {
        if let Ok(mut requests) = self.pending.lock()
            && let Some(request) = requests.get_mut(&token)
        {
            request.result = Some(result.map_or_else(|e| e.code(), |()| HRESULT(0)));
        }
    }
}
pub(crate) struct AccessibilityHost {
    pub shared: Arc<Shared>,
    pub provider: IRawElementProviderSimple,
}
impl AccessibilityHost {
    pub fn new(hwnd: HWND) -> Self {
        let shared = Arc::new(Shared {
            hwnd: hwnd.0 as usize,
            snapshot: RwLock::new(None),
            closed: AtomicBool::new(false),
            next: AtomicUsize::new(1),
            pending: Mutex::new(HashMap::new()),
        });
        let provider = element(shared.clone(), None)
            .cast()
            .expect("provider identity");
        Self { shared, provider }
    }
    pub fn publish(&self, snapshot: Snapshot) {
        let previous = self
            .shared
            .snapshot
            .write()
            .ok()
            .and_then(|mut current| current.replace(Arc::new(snapshot)));
        let Ok(current) = self.shared.current() else {
            return;
        };
        if !unsafe { UiaClientsAreListening() }.as_bool() {
            return;
        }
        if previous.as_ref().is_none_or(|old| {
            old.bounds.left != current.bounds.left
                || old.bounds.top != current.bounds.top
                || old.bounds.width != current.bounds.width
                || old.bounds.height != current.bounds.height
                || old.scale != current.scale
                || old.scroll_y != current.scroll_y
                || old.visible != current.visible
        }) {
            unsafe {
                let _ = UiaRaiseAutomationEvent(&self.provider, UIA_LayoutInvalidatedEventId);
            }
        }
        if let Some(old) = previous
            .as_ref()
            .filter(|old| old.focused != current.focused)
        {
            unsafe {
                let _ = UiaRaiseAutomationPropertyChangedEvent(
                    &self.provider,
                    UIA_HasKeyboardFocusPropertyId,
                    &VARIANT::from(old.focused),
                    &VARIANT::from(current.focused),
                );
            }
        }
        if let Some(old) = previous
            .as_ref()
            .filter(|old| old.visible != current.visible)
        {
            unsafe {
                let _ = UiaRaiseAutomationPropertyChangedEvent(
                    &self.provider,
                    UIA_IsOffscreenPropertyId,
                    &VARIANT::from(!old.visible),
                    &VARIANT::from(!current.visible),
                );
            }
        }
        if previous.as_ref().is_none_or(|old| {
            old.identity != current.identity || old.source.revision() != current.source.revision()
        }) {
            unsafe {
                let _ = UiaRaiseAutomationEvent(&self.provider, UIA_Text_TextChangedEventId);
                let _ = UiaRaiseStructureChangedEvent(
                    &self.provider,
                    StructureChangeType_ChildrenInvalidated,
                    ptr::null_mut(),
                    0,
                );
            }
        }
        if previous
            .as_ref()
            .is_none_or(|old| old.text.selected_range() != current.text.selected_range())
        {
            unsafe {
                let _ =
                    UiaRaiseAutomationEvent(&self.provider, UIA_Text_TextSelectionChangedEventId);
            }
        }
        if current.focused && previous.as_ref().is_none_or(|old| !old.focused) {
            unsafe {
                let _ = UiaRaiseAutomationEvent(&self.provider, UIA_AutomationFocusChangedEventId);
            }
        }
    }
    pub fn close(&self) {
        self.shared.closed.store(true, Ordering::Release);
        if let Ok(mut snapshot) = self.shared.snapshot.write() {
            *snapshot = None;
        }
        unsafe {
            let _ = UiaDisconnectProvider(&self.provider);
        }
    }
}
impl Drop for AccessibilityHost {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Clone, Copy)]
struct Node {
    identity: u64,
    revision: Revision,
    index: u32,
}
#[implement(SimpleAbi, FragmentAbi, RootAbi, ITextProvider2, IToggleProvider)]
struct Element {
    shared: Arc<Shared>,
    node: Option<Node>,
}
fn element(shared: Arc<Shared>, node: Option<Node>) -> ComObject<Element> {
    ComObject::new(Element { shared, node })
}
impl Element {
    fn snapshot(&self) -> Result<Arc<Snapshot>> {
        let snapshot = self.shared.current()?;
        if let Some(node) = self.node
            && (node.identity != snapshot.identity || node.revision != snapshot.source.revision())
        {
            return Err(unavailable());
        }
        Ok(snapshot)
    }
    fn range(&self) -> Result<(Arc<Snapshot>, TextRange)> {
        let snapshot = self.snapshot()?;
        let source = if let Some(node) = self.node {
            snapshot
                .text
                .source_range(
                    snapshot
                        .semantic
                        .node(node.index)
                        .ok_or_else(unavailable)?
                        .source_range(),
                )
                .map_err(|_| unavailable())?
        } else {
            TextRange::new(ByteOffset::ZERO, snapshot.source.len_bytes()).ok_or_else(invalid)?
        };
        Ok((snapshot, source))
    }
    fn child(&self, index: u32, snapshot: &Snapshot) -> ComObject<Element> {
        element(
            self.shared.clone(),
            Some(Node {
                identity: snapshot.identity,
                revision: snapshot.source.revision(),
                index,
            }),
        )
    }
    fn property(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let snapshot = self.snapshot()?;
        let node = self.node.and_then(|n| snapshot.semantic.node(n.index));
        Ok(match id {
            UIA_ControlTypePropertyId => VARIANT::from(match node.map(|n| n.kind()) {
                None => UIA_DocumentControlTypeId.0,
                Some(Kind::TaskListItem) => UIA_CheckBoxControlTypeId.0,
                Some(Kind::Image | Kind::ReferenceImage) => UIA_ImageControlTypeId.0,
                Some(Kind::Link | Kind::ReferenceLink | Kind::Autolink) => {
                    UIA_HyperlinkControlTypeId.0
                }
                _ => UIA_TextControlTypeId.0,
            }),
            UIA_NamePropertyId => VARIANT::from(BSTR::from(if let Some(node) = node {
                snapshot
                    .labels
                    .get(&node.index())
                    .cloned()
                    .unwrap_or_default()
            } else {
                snapshot.name.clone()
            })),
            UIA_AutomationIdPropertyId => {
                VARIANT::from(BSTR::from(if let Some(node) = self.node {
                    format!("markdown-node-{}", node.index)
                } else {
                    "markdown-editor".into()
                }))
            }
            UIA_FrameworkIdPropertyId => VARIANT::from("Yu"),
            UIA_IsControlElementPropertyId
            | UIA_IsContentElementPropertyId
            | UIA_IsEnabledPropertyId => VARIANT::from(true),
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(self.node.is_none()),
            UIA_HasKeyboardFocusPropertyId => {
                VARIANT::from(self.node.is_none() && snapshot.focused)
            }
            UIA_IsOffscreenPropertyId => VARIANT::from(
                !snapshot.visible
                    || (self.node.is_some() && snapshot.rectangles(self.range()?.1)?.is_empty()),
            ),
            UIA_IsTextPatternAvailablePropertyId | UIA_IsTextPattern2AvailablePropertyId => {
                VARIANT::from(self.node.is_none())
            }
            UIA_HeadingLevelPropertyId => VARIANT::from(
                node.filter(|n| n.kind() == Kind::Heading)
                    .map_or(HeadingLevel_None.0, |n| {
                        HeadingLevel1.0 + i32::from(n.level().saturating_sub(1))
                    }),
            ),
            _ => VARIANT::default(),
        })
    }
    fn navigate(
        &self,
        direction: NavigateDirection,
    ) -> Result<Option<IRawElementProviderFragment>> {
        let snapshot = self.snapshot()?;
        let index = self.node.map_or(0, |n| n.index);
        let node = snapshot.semantic.node(index).ok_or_else(unavailable)?;
        let result = match direction {
            NavigateDirection_Parent if self.node.is_some() => node.parent(),
            NavigateDirection_FirstChild => snapshot
                .semantic
                .nodes()
                .iter()
                .find(|n| n.parent() == Some(index))
                .map(|n| n.index()),
            NavigateDirection_LastChild => snapshot
                .semantic
                .nodes()
                .iter()
                .rev()
                .find(|n| n.parent() == Some(index))
                .map(|n| n.index()),
            NavigateDirection_NextSibling => snapshot
                .semantic
                .nodes()
                .iter()
                .find(|n| n.index() > index && n.parent() == node.parent())
                .map(|n| n.index()),
            NavigateDirection_PreviousSibling => snapshot
                .semantic
                .nodes()
                .iter()
                .rev()
                .find(|n| n.index() < index && n.parent() == node.parent())
                .map(|n| n.index()),
            _ => None,
        };
        result
            .map(|index| {
                if index == 0 {
                    element(self.shared.clone(), None).cast()
                } else {
                    self.child(index, &snapshot).cast()
                }
            })
            .transpose()
    }
    fn bounding(&self) -> Result<UiaRect> {
        let (snapshot, source) = self.range()?;
        if self.node.is_none() {
            return Ok(snapshot.bounds);
        }
        let rectangles = snapshot.rectangles(source)?;
        let Some(first) = rectangles.first() else {
            return Ok(UiaRect::default());
        };
        Ok(rectangles.iter().skip(1).fold(*first, |a, b| {
            let right = (a.left + a.width).max(b.left + b.width);
            let bottom = (a.top + a.height).max(b.top + b.height);
            let left = a.left.min(b.left);
            let top = a.top.min(b.top);
            UiaRect {
                left,
                top,
                width: right - left,
                height: bottom - top,
            }
        }))
    }
}
impl SimpleAbi_Impl for Element_Impl {
    unsafe fn ProviderOptions(&self, out: *mut ProviderOptions) -> HRESULT {
        unsafe {
            output(
                out,
                Ok(ProviderOptions_ServerSideProvider
                    | ProviderOptions_UseComThreading
                    | ProviderOptions_ProviderOwnsSetFocus),
            )
        }
    }
    unsafe fn GetPatternProvider(&self, id: UIA_PATTERN_ID, out: *mut *mut c_void) -> HRESULT {
        let result = self.snapshot().and_then(|snapshot| match id {
            UIA_TextPatternId | UIA_TextPattern2Id if self.node.is_none() => {
                unsafe { self.cast::<ITextProvider2>() }
                    .and_then(|v| v.cast())
                    .map(Some)
            }
            UIA_TogglePatternId
                if self
                    .node
                    .and_then(|n| snapshot.semantic.node(n.index))
                    .is_some_and(|n| n.kind() == Kind::TaskListItem) =>
            {
                unsafe { self.cast::<IToggleProvider>() }
                    .and_then(|v| v.cast())
                    .map(Some)
            }
            _ => Ok(None::<IUnknown>),
        });
        unsafe { interface_output(out, result) }
    }
    unsafe fn GetPropertyValue(&self, id: UIA_PROPERTY_ID, out: *mut VARIANT) -> HRESULT {
        unsafe { output(out, self.property(id)) }
    }
    unsafe fn HostRawElementProvider(&self, out: *mut *mut c_void) -> HRESULT {
        unsafe {
            interface_output(
                out,
                self.snapshot().and_then(|_| {
                    if self.node.is_none() {
                        UiaHostProviderFromHwnd(HWND(self.shared.hwnd as *mut _)).map(Some)
                    } else {
                        Ok(None)
                    }
                }),
            )
        }
    }
}
impl FragmentAbi_Impl for Element_Impl {
    unsafe fn Navigate(&self, direction: NavigateDirection, out: *mut *mut c_void) -> HRESULT {
        unsafe { interface_output(out, self.navigate(direction)) }
    }
    unsafe fn GetRuntimeId(&self, out: *mut *mut SAFEARRAY) -> HRESULT {
        let result = self.snapshot().and_then(|s| {
            if let Some(n) = self.node {
                array(
                    VT_I4,
                    &[
                        UiaAppendRuntimeId as i32,
                        s.identity as i32,
                        (s.identity >> 32) as i32,
                        n.revision.get() as i32,
                        (n.revision.get() >> 32) as i32,
                        n.index as i32,
                    ],
                )
            } else {
                Ok(ptr::null_mut())
            }
        });
        unsafe { output(out, result) }
    }
    unsafe fn BoundingRectangle(&self, out: *mut UiaRect) -> HRESULT {
        unsafe { output(out, self.bounding()) }
    }
    unsafe fn GetEmbeddedFragmentRoots(&self, out: *mut *mut SAFEARRAY) -> HRESULT {
        unsafe { output(out, self.snapshot().map(|_| ptr::null_mut())) }
    }
    unsafe fn SetFocus(&self) -> HRESULT {
        self.snapshot()
            .and_then(|_| self.shared.dispatch(Action::Focus))
            .map_or_else(|e| e.code(), |()| HRESULT(0))
    }
    unsafe fn FragmentRoot(&self, out: *mut *mut c_void) -> HRESULT {
        unsafe {
            interface_output(
                out,
                self.snapshot()
                    .and_then(|_| {
                        element(self.shared.clone(), None).cast::<IRawElementProviderFragmentRoot>()
                    })
                    .map(Some),
            )
        }
    }
}
impl RootAbi_Impl for Element_Impl {
    unsafe fn ElementProviderFromPoint(&self, x: f64, y: f64, out: *mut *mut c_void) -> HRESULT {
        let result = self.snapshot().and_then(|s| {
            if !s.visible {
                return Ok(None);
            }
            if !x.is_finite() || !y.is_finite() {
                return Err(invalid());
            }
            if x < s.bounds.left
                || y < s.bounds.top
                || x >= s.bounds.left + s.bounds.width
                || y >= s.bounds.top + s.bounds.height
            {
                return Ok(None);
            }
            let position = s.at_point(&UiaPoint { x, y })?;
            let byte = s
                .source
                .byte_offset_for_utf16(Utf16Offset::new(position))
                .map_err(|_| invalid())?;
            let node = s.semantic.nodes().iter().rev().find(|n| {
                n.index() != 0
                    && s.text
                        .source_range(n.source_range())
                        .is_ok_and(|r| r.contains(byte))
            });
            if let Some(node) = node {
                self.child(node.index(), &s)
                    .cast::<IRawElementProviderFragment>()
                    .map(Some)
            } else {
                element(self.shared.clone(), None)
                    .cast::<IRawElementProviderFragment>()
                    .map(Some)
            }
        });
        unsafe { interface_output(out, result) }
    }
    unsafe fn GetFocus(&self, out: *mut *mut c_void) -> HRESULT {
        unsafe {
            interface_output(
                out,
                self.snapshot().and_then(|s| {
                    if s.focused {
                        element(self.shared.clone(), None)
                            .cast::<IRawElementProviderFragment>()
                            .map(Some)
                    } else {
                        Ok(None)
                    }
                }),
            )
        }
    }
}
impl ITextProvider_Impl for Element_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let s = self.snapshot()?;
        let r = s.text.selected_range().range();
        interfaces(&[text_range(
            self.shared.clone(),
            &s,
            r.start().get(),
            r.end().get(),
        )])
    }
    fn GetVisibleRanges(&self) -> Result<*mut SAFEARRAY> {
        let s = self.snapshot()?;
        let mut ranges = Vec::new();
        for block in s.layout.blocks() {
            for (index, line) in block.layout().layout().lines().iter().enumerate() {
                if s.screen_rect(
                    0.0,
                    block.content_y() + line.y(),
                    s.bounds.width as f32 / s.scale as f32,
                    line.height(),
                )
                .is_none()
                {
                    continue;
                }
                let mut source: Option<(ByteOffset, ByteOffset)> = None;
                for cluster in block
                    .layout()
                    .layout()
                    .clusters()
                    .iter()
                    .filter(|c| c.line() == index)
                {
                    let a = block
                        .layout()
                        .visual()
                        .visual_to_source(cluster.visual().start(), Bias::After)
                        .map_err(|_| failed())?;
                    let b = block
                        .layout()
                        .visual()
                        .visual_to_source(cluster.visual().end(), Bias::Before)
                        .map_err(|_| failed())?;
                    source = Some(
                        source.map_or((a, b.max(a)), |(old_a, old_b)| (old_a.min(a), old_b.max(b))),
                    );
                }
                if let Some((a, b)) = source {
                    ranges.push(text_range(
                        self.shared.clone(),
                        &s,
                        s.utf16(a)?,
                        s.utf16(b)?,
                    ));
                }
            }
        }
        interfaces(&ranges)
    }
    fn RangeFromChild(
        &self,
        child: Option<&IRawElementProviderSimple>,
    ) -> Result<ITextRangeProvider> {
        let s = self.snapshot()?;
        let child = child.ok_or_else(invalid)?.cast_object::<Element>()?;
        if !Arc::ptr_eq(&child.shared, &self.shared) {
            return Err(invalid());
        }
        let (_, source) = child.range()?;
        Ok(text_range(
            self.shared.clone(),
            &s,
            s.utf16(source.start())?,
            s.utf16(source.end())?,
        ))
    }
    fn RangeFromPoint(&self, point: &UiaPoint) -> Result<ITextRangeProvider> {
        let s = self.snapshot()?;
        let pos = s.at_point(point)?;
        Ok(text_range(self.shared.clone(), &s, pos, pos))
    }
    fn DocumentRange(&self) -> Result<ITextRangeProvider> {
        let s = self.snapshot()?;
        Ok(text_range(
            self.shared.clone(),
            &s,
            0,
            s.text.number_of_characters().get(),
        ))
    }
    fn SupportedTextSelection(&self) -> Result<SupportedTextSelection> {
        self.snapshot()?;
        Ok(SupportedTextSelection_Single)
    }
}
impl ITextProvider2_Impl for Element_Impl {
    fn RangeFromAnnotation(
        &self,
        _: Option<&IRawElementProviderSimple>,
    ) -> Result<ITextRangeProvider> {
        Err(unsupported())
    }
    fn GetCaretRange(&self, active: *mut BOOL) -> Result<ITextRangeProvider> {
        if active.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let s = self.snapshot()?;
        unsafe {
            active.write(BOOL::from(s.focused));
        }
        let caret = s.utf16(s.caret)?;
        Ok(text_range(self.shared.clone(), &s, caret, caret))
    }
}
impl IToggleProvider_Impl for Element_Impl {
    fn Toggle(&self) -> Result<()> {
        let s = self.snapshot()?;
        let node = self
            .node
            .and_then(|n| s.semantic.node(n.index))
            .ok_or_else(unsupported)?;
        self.shared.dispatch(Action::Toggle {
            identity: s.identity,
            revision: s.source.revision(),
            block: node.action_block().ok_or_else(unsupported)?,
        })
    }
    fn ToggleState(&self) -> Result<ToggleState> {
        let s = self.snapshot()?;
        let node = self
            .node
            .and_then(|n| s.semantic.node(n.index))
            .ok_or_else(unsupported)?;
        Ok(
            if node.flags() & yu_editor::ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE != 0 {
                ToggleState_On
            } else {
                ToggleState_Off
            },
        )
    }
}

#[implement(RangeAbi)]
struct TextRangeProvider {
    shared: Arc<Shared>,
    identity: u64,
    revision: Revision,
    endpoints: Mutex<(u64, u64)>,
}
fn text_range(shared: Arc<Shared>, s: &Snapshot, start: u64, end: u64) -> ITextRangeProvider {
    ComObject::new(TextRangeProvider {
        shared,
        identity: s.identity,
        revision: s.source.revision(),
        endpoints: Mutex::new((start, end)),
    })
    .cast()
    .expect("range identity")
}
impl TextRangeProvider {
    fn current(&self) -> Result<(Arc<Snapshot>, u64, u64)> {
        let s = self.shared.current()?;
        if s.identity != self.identity || s.source.revision() != self.revision {
            return Err(unavailable());
        }
        let (start, end) = *self.endpoints.lock().map_err(|_| failed())?;
        s.source_range(start, end)?;
        Ok((s, start, end))
    }
    fn target(&self, raw: *mut c_void) -> Result<(u64, u64)> {
        let other = unsafe { ITextRangeProvider::from_raw_borrowed(&raw) }
            .ok_or_else(invalid)?
            .cast_object::<TextRangeProvider>()?;
        if !Arc::ptr_eq(&other.shared, &self.shared)
            || other.get().identity != self.identity
            || other.revision != self.revision
        {
            return Err(invalid());
        }
        let (_, a, b) = other.current()?;
        Ok((a, b))
    }
    fn set(&self, start: u64, end: u64) -> Result<()> {
        let (s, _, _) = self.current()?;
        s.source_range(start, end)?;
        *self.endpoints.lock().map_err(|_| failed())? = (start, end);
        Ok(())
    }
    fn endpoint(endpoint: TextPatternRangeEndpoint, a: u64, b: u64) -> Result<u64> {
        match endpoint {
            TextPatternRangeEndpoint_Start => Ok(a),
            TextPatternRangeEndpoint_End => Ok(b),
            _ => Err(invalid()),
        }
    }
    fn boundaries(&self, s: &Snapshot, unit: TextUnit) -> Result<Vec<u64>> {
        let unit = match unit {
            TextUnit_Character => AccessibilityTextUnit::Character,
            TextUnit_Word => AccessibilityTextUnit::Word,
            TextUnit_Format | TextUnit_Line | TextUnit_Paragraph => {
                AccessibilityTextUnit::Paragraph
            }
            TextUnit_Page | TextUnit_Document => AccessibilityTextUnit::Document,
            _ => return Err(invalid()),
        };
        s.text
            .unit_boundaries(unit)
            .map(|v| v.into_iter().map(|v| v.get()).collect())
            .map_err(|_| failed())
    }
    fn enclosing(boundaries: &[u64], start: u64) -> (u64, u64) {
        if boundaries.len() < 2 {
            return (start, start);
        }
        let index = boundaries
            .partition_point(|p| *p <= start)
            .saturating_sub(1)
            .min(boundaries.len() - 2);
        (boundaries[index], boundaries[index + 1])
    }
    fn move_endpoint(
        &self,
        endpoint: TextPatternRangeEndpoint,
        unit: TextUnit,
        count: i32,
    ) -> Result<i32> {
        let (s, a, b) = self.current()?;
        let pos = TextRangeProvider::endpoint(endpoint, a, b)?;
        let points = self.boundaries(&s, unit)?;
        if count == 0 {
            return Ok(0);
        }
        let insertion = points.partition_point(|p| *p < pos);
        let base = if count > 0 {
            points.partition_point(|p| *p <= pos).saturating_sub(1)
        } else {
            insertion.min(points.len() - 1)
        };
        let index = (base as i64 + i64::from(count)).clamp(0, points.len().saturating_sub(1) as i64)
            as usize;
        let target = points[index];
        if endpoint == TextPatternRangeEndpoint_Start {
            self.set(target, b.max(target))?;
        } else {
            self.set(a.min(target), target)?;
        }
        Ok((index as i64 - base as i64) as i32)
    }
    fn move_range(&self, unit: TextUnit, count: i32) -> Result<i32> {
        let (s, a, b) = self.current()?;
        if count == 0 {
            return Ok(0);
        }
        let points = self.boundaries(&s, unit)?;
        if a == b {
            return self
                .move_endpoint(TextPatternRangeEndpoint_Start, unit, count)
                .and_then(|m| {
                    let (_, a, _) = self.current()?;
                    self.set(a, a)?;
                    Ok(m)
                });
        }
        if points.len() < 2 {
            return Ok(0);
        }
        let base = points
            .partition_point(|p| *p <= a)
            .saturating_sub(1)
            .min(points.len() - 2);
        let index = (base as i64 + i64::from(count)).clamp(0, (points.len() - 2) as i64) as usize;
        self.set(points[index], points[index + 1])?;
        Ok((index as i64 - base as i64) as i32)
    }
    fn text(&self, max: i32) -> Result<BSTR> {
        if max < -1 {
            return Err(invalid());
        }
        let (s, a, b) = self.current()?;
        let range = s
            .text
            .bind_range(
                Utf16Range::new(Utf16Offset::new(a), Utf16Offset::new(b)).ok_or_else(invalid)?,
            )
            .map_err(|_| invalid())?;
        let mut text = s.text.text_for_range(range).map_err(|_| failed())?;
        if max >= 0 {
            let mut units = 0;
            let mut end = 0;
            for (index, ch) in text.char_indices() {
                if units + ch.len_utf16() > max as usize {
                    break;
                }
                units += ch.len_utf16();
                end = index + ch.len_utf8();
            }
            text.truncate(end);
        }
        Ok(BSTR::from(text))
    }
    fn attribute(&self, id: UIA_TEXTATTRIBUTE_ID) -> Result<VARIANT> {
        let (s, a, b) = self.current()?;
        let range = s.source_range(a, b)?;
        if id == UIA_IsReadOnlyAttributeId {
            return Ok(VARIANT::from(false));
        }
        if id == UIA_StyleIdAttributeId {
            let mut styles = Vec::new();
            for n in s.semantic.nodes().iter().filter(|n| n.parent() == Some(0)) {
                let r = s
                    .text
                    .source_range(n.source_range())
                    .map_err(|_| unavailable())?;
                if (a == b && r.contains(range.start()))
                    || (r.start() < range.end() && r.end() > range.start())
                {
                    styles.push(if n.kind() == Kind::Heading {
                        StyleId_Heading1.0 + i32::from(n.level().saturating_sub(1))
                    } else {
                        StyleId_Normal.0
                    });
                }
            }
            styles.dedup();
            if styles.len() > 1 {
                return unsafe { UiaGetReservedMixedAttributeValue() }.map(VARIANT::from);
            }
            return Ok(VARIANT::from(
                styles.first().copied().unwrap_or(StyleId_Normal.0),
            ));
        }
        unsafe { UiaGetReservedNotSupportedValue() }.map(VARIANT::from)
    }
    fn find_attribute(
        &self,
        id: UIA_TEXTATTRIBUTE_ID,
        value: &VARIANT,
        backwards: bool,
    ) -> Result<Option<ITextRangeProvider>> {
        let (s, a, b) = self.current()?;
        if id == UIA_IsReadOnlyAttributeId || a == b {
            return Ok(
                (self.attribute(id)? == *value).then(|| text_range(self.shared.clone(), &s, a, b))
            );
        }
        if id != UIA_StyleIdAttributeId {
            return Err(unsupported());
        }
        let mut spans = Vec::new();
        let mut cursor = a;
        for node in s
            .semantic
            .nodes()
            .iter()
            .filter(|n| n.parent() == Some(0) && n.kind() == Kind::Heading)
        {
            let range = node.source_range().range();
            let start = range.start().get().max(a);
            let end = range.end().get().min(b);
            if start >= end {
                continue;
            }
            if cursor < start {
                spans.push((cursor, start, StyleId_Normal.0));
            }
            spans.push((
                start,
                end,
                StyleId_Heading1.0 + i32::from(node.level().saturating_sub(1)),
            ));
            cursor = end;
        }
        if cursor < b {
            spans.push((cursor, b, StyleId_Normal.0));
        }
        let matches = |span: &&(u64, u64, i32)| VARIANT::from(span.2) == *value;
        let found = if backwards {
            spans.iter().rfind(matches)
        } else {
            spans.iter().find(matches)
        };
        Ok(found.map(|&(start, end, _)| text_range(self.shared.clone(), &s, start, end)))
    }
    fn find_text(
        &self,
        text: *const u16,
        backwards: BOOL,
        ignore_case: BOOL,
    ) -> Result<Option<ITextRangeProvider>> {
        if text.is_null() {
            return Err(invalid());
        }
        let needle = std::mem::ManuallyDrop::new(unsafe { BSTR::from_raw(text.cast_mut()) });
        let query = String::from_utf16(needle.as_wide()).map_err(|_| invalid())?;
        if query.is_empty() {
            return Err(invalid());
        }
        let (s, a, b) = self.current()?;
        let range = s
            .text
            .bind_range(
                Utf16Range::new(Utf16Offset::new(a), Utf16Offset::new(b)).ok_or_else(invalid)?,
            )
            .map_err(|_| invalid())?;
        let found = s
            .text
            .find_text(range, &query, backwards.as_bool(), ignore_case.as_bool())
            .map_err(|_| failed())?;
        Ok(found.map(|range| {
            text_range(
                self.shared.clone(),
                &s,
                range.range().start().get(),
                range.range().end().get(),
            )
        }))
    }
    fn children(&self) -> Result<*mut SAFEARRAY> {
        let (s, a, b) = self.current()?;
        let source = s.source_range(a, b)?;
        let children: Vec<IRawElementProviderSimple> = s
            .semantic
            .nodes()
            .iter()
            .filter(|n| n.index() != 0)
            .filter(|n| {
                s.text
                    .source_range(n.source_range())
                    .is_ok_and(|r| r.start() >= source.start() && r.end() <= source.end())
            })
            .filter(|n| {
                n.parent()
                    .filter(|parent| *parent != 0)
                    .and_then(|parent| s.semantic.node(parent))
                    .is_none_or(|parent| {
                        !s.text
                            .source_range(parent.source_range())
                            .is_ok_and(|r| r.start() >= source.start() && r.end() <= source.end())
                    })
            })
            .map(|n| {
                element(
                    self.shared.clone(),
                    Some(Node {
                        identity: s.identity,
                        revision: s.source.revision(),
                        index: n.index(),
                    }),
                )
                .cast()
                .expect("semantic identity")
            })
            .collect();
        interfaces(&children)
    }
    fn action(&self, scroll: Option<bool>) -> Result<()> {
        let (s, a, b) = self.current()?;
        let source = s.source_range(a, b)?;
        self.shared.dispatch(if let Some(top) = scroll {
            Action::Scroll {
                identity: self.identity,
                revision: self.revision,
                source,
                top,
            }
        } else {
            Action::Select {
                identity: self.identity,
                revision: self.revision,
                source,
            }
        })
    }
}
impl RangeAbi_Impl for TextRangeProvider_Impl {
    unsafe fn Clone(&self, out: *mut *mut c_void) -> HRESULT {
        unsafe {
            interface_output(
                out,
                self.current()
                    .map(|(s, a, b)| Some(text_range(self.shared.clone(), &s, a, b))),
            )
        }
    }
    unsafe fn Compare(&self, other: *mut c_void, out: *mut BOOL) -> HRESULT {
        unsafe {
            output(
                out,
                self.current()
                    .and_then(|(_, a, b)| self.target(other).map(|r| BOOL::from(r == (a, b)))),
            )
        }
    }
    unsafe fn CompareEndpoints(
        &self,
        end: TextPatternRangeEndpoint,
        other: *mut c_void,
        other_end: TextPatternRangeEndpoint,
        out: *mut i32,
    ) -> HRESULT {
        let result = self.current().and_then(|(_, a, b)| {
            let (a2, b2) = self.target(other)?;
            i32::try_from(
                i128::from(TextRangeProvider::endpoint(end, a, b)?)
                    - i128::from(TextRangeProvider::endpoint(other_end, a2, b2)?),
            )
            .map_err(|_| invalid())
        });
        unsafe { output(out, result) }
    }
    unsafe fn ExpandToEnclosingUnit(&self, unit: TextUnit) -> HRESULT {
        self.current()
            .and_then(|(s, a, _)| {
                let bounds = self.boundaries(&s, unit)?;
                let (a, b) = TextRangeProvider::enclosing(&bounds, a);
                self.set(a, b)
            })
            .map_or_else(|e| e.code(), |()| HRESULT(0))
    }
    unsafe fn FindAttribute(
        &self,
        id: UIA_TEXTATTRIBUTE_ID,
        value: *const VARIANT,
        backwards: BOOL,
        out: *mut *mut c_void,
    ) -> HRESULT {
        let result = if value.is_null() {
            Err(Error::from_hresult(E_POINTER))
        } else {
            self.find_attribute(id, unsafe { &*value }, backwards.as_bool())
        };
        unsafe { interface_output(out, result) }
    }
    unsafe fn FindText(
        &self,
        text: *const u16,
        backwards: BOOL,
        ignore: BOOL,
        out: *mut *mut c_void,
    ) -> HRESULT {
        unsafe { interface_output(out, self.find_text(text, backwards, ignore)) }
    }
    unsafe fn GetAttributeValue(&self, id: UIA_TEXTATTRIBUTE_ID, out: *mut VARIANT) -> HRESULT {
        unsafe { output(out, self.attribute(id)) }
    }
    unsafe fn GetBoundingRectangles(&self, out: *mut *mut SAFEARRAY) -> HRESULT {
        let result = self.current().and_then(|(s, a, b)| {
            let rects = s.rectangles(s.source_range(a, b)?)?;
            let values: Vec<f64> = rects
                .iter()
                .flat_map(|r| [r.left, r.top, r.width, r.height])
                .collect();
            array(VT_R8, &values)
        });
        unsafe { output(out, result) }
    }
    unsafe fn GetEnclosingElement(&self, out: *mut *mut c_void) -> HRESULT {
        unsafe {
            interface_output(
                out,
                self.current()
                    .and_then(|_| {
                        element(self.shared.clone(), None).cast::<IRawElementProviderSimple>()
                    })
                    .map(Some),
            )
        }
    }
    unsafe fn GetText(&self, max: i32, out: *mut BSTR) -> HRESULT {
        unsafe { output(out, self.text(max)) }
    }
    unsafe fn Move(&self, unit: TextUnit, count: i32, out: *mut i32) -> HRESULT {
        unsafe { output(out, self.move_range(unit, count)) }
    }
    unsafe fn MoveEndpointByUnit(
        &self,
        end: TextPatternRangeEndpoint,
        unit: TextUnit,
        count: i32,
        out: *mut i32,
    ) -> HRESULT {
        unsafe { output(out, self.move_endpoint(end, unit, count)) }
    }
    unsafe fn MoveEndpointByRange(
        &self,
        end: TextPatternRangeEndpoint,
        other: *mut c_void,
        target: TextPatternRangeEndpoint,
    ) -> HRESULT {
        self.current()
            .and_then(|(_, a, b)| {
                let (c, d) = self.target(other)?;
                let value = TextRangeProvider::endpoint(target, c, d)?;
                match end {
                    TextPatternRangeEndpoint_Start => self.set(value, b.max(value)),
                    TextPatternRangeEndpoint_End => self.set(a.min(value), value),
                    _ => Err(invalid()),
                }
            })
            .map_or_else(|e| e.code(), |()| HRESULT(0))
    }
    unsafe fn Select(&self) -> HRESULT {
        self.action(None).map_or_else(|e| e.code(), |()| HRESULT(0))
    }
    unsafe fn AddToSelection(&self) -> HRESULT {
        unsupported().code()
    }
    unsafe fn RemoveFromSelection(&self) -> HRESULT {
        unsupported().code()
    }
    unsafe fn ScrollIntoView(&self, top: BOOL) -> HRESULT {
        self.action(Some(top.as_bool()))
            .map_or_else(|e| e.code(), |()| HRESULT(0))
    }
    unsafe fn GetChildren(&self, out: *mut *mut SAFEARRAY) -> HRESULT {
        unsafe { output(out, self.children()) }
    }
}
