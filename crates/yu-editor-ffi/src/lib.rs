#![allow(unsafe_code)]

use std::ptr;

use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    CaretAffinity, CaretScrollRequest, CommandResult, EditorCommand, EditorDocument,
    EditorDocumentError, EditorKey, EditorSelection, KeyEvent, KeyModifiers, KeyRouteResult,
    LayoutConfig, LayoutSnapshot, Projection, SelectionError, SourceSync, ViewportConfig,
    ViewportRect, VisualRunKind,
};
use yu_text::{EditError, TextSnapshot};

#[cfg(target_os = "macos")]
use yu_font::FontRequest;
#[cfg(target_os = "macos")]
use yu_font_macos::{CoreTextShaper, CoreTextViewportMetrics};

pub const YU_FFI_OK: i32 = 0;
pub const YU_FFI_NULL_POINTER: i32 = 1;
pub const YU_FFI_INVALID_UTF8: i32 = 2;
pub const YU_FFI_INVALID_RANGE: i32 = 3;
pub const YU_FFI_INVALID_SELECTION: i32 = 4;
pub const YU_FFI_NO_OVERLAY: i32 = 5;
pub const YU_FFI_BUFFER_TOO_SMALL: i32 = 6;
pub const YU_FFI_EDIT_FAILED: i32 = 7;
pub const YU_FFI_STALE_REVISION: i32 = 8;
pub const YU_FFI_KEY_UNHANDLED: i32 = 9;
pub const YU_FFI_INVALID_COMMAND: i32 = 10;
pub const YU_FFI_INVALID_KEY: i32 = 11;
pub const YU_FFI_INVALID_VIEWPORT_CONFIG: i32 = 12;
pub const YU_FFI_CORE_TEXT_UNAVAILABLE: i32 = 13;
pub const YU_FFI_LAYOUT_FAILED: i32 = 14;
pub const YU_COMMAND_UNAVAILABLE: u8 = 0;
pub const YU_COMMAND_AVAILABLE: u8 = 1;
pub const YU_SOURCE_SYNC_NONE: u8 = 0;
pub const YU_SOURCE_SYNC_RANGE: u8 = 1;
pub const YU_SOURCE_SYNC_FULL: u8 = 2;
pub const YU_CARET_AFFINITY_UPSTREAM: u8 = 0;
pub const YU_CARET_AFFINITY_DOWNSTREAM: u8 = 1;
pub const YU_KEY_CHARACTER: u8 = 0;
pub const YU_KEY_ENTER: u8 = 1;
pub const YU_KEY_TAB: u8 = 2;
pub const YU_KEY_BACKSPACE: u8 = 3;
pub const YU_KEY_DELETE: u8 = 4;
pub const YU_KEY_LEFT: u8 = 5;
pub const YU_KEY_RIGHT: u8 = 6;
pub const YU_KEY_UP: u8 = 7;
pub const YU_KEY_DOWN: u8 = 8;
pub const YU_KEY_ESCAPE: u8 = 9;
pub const YU_KEY_MODIFIER_COMMAND: u8 = 1 << 0;
pub const YU_KEY_MODIFIER_SHIFT: u8 = 1 << 1;
pub const YU_KEY_MODIFIER_CONTROL: u8 = 1 << 2;
pub const YU_KEY_MODIFIER_OPTION: u8 = 1 << 3;
pub const YU_EDITOR_COMMAND_DELETE_BACKWARD: u8 = 1;
pub const YU_EDITOR_COMMAND_DELETE_FORWARD: u8 = 2;
pub const YU_EDITOR_COMMAND_MOVE_LEFT: u8 = 3;
pub const YU_EDITOR_COMMAND_MOVE_RIGHT: u8 = 4;
pub const YU_EDITOR_COMMAND_MOVE_WORD_LEFT: u8 = 11;
pub const YU_EDITOR_COMMAND_MOVE_WORD_RIGHT: u8 = 12;
pub const YU_EDITOR_COMMAND_INSERT_NEWLINE: u8 = 5;
pub const YU_EDITOR_COMMAND_INDENT_LIST: u8 = 6;
pub const YU_EDITOR_COMMAND_OUTDENT_LIST: u8 = 7;
pub const YU_EDITOR_COMMAND_UNDO: u8 = 8;
pub const YU_EDITOR_COMMAND_REDO: u8 = 9;
pub const YU_EDITOR_COMMAND_TOGGLE_TASK: u8 = 10;
pub const YU_EDITOR_COMMAND_MOVE_UP: u8 = 13;
pub const YU_EDITOR_COMMAND_MOVE_DOWN: u8 = 14;
pub const YU_EDITOR_COMMAND_MOVE_UP_EXTEND: u8 = 15;
pub const YU_EDITOR_COMMAND_MOVE_DOWN_EXTEND: u8 = 16;

/// Revision and UTF-16 selection returned after one native command.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuEditorCommandResult {
    pub revision: u64,
    pub selection_start_utf16: u64,
    pub selection_end_utf16: u64,
    pub affinity: u8,
    pub changed: u8,
    pub source_sync: u8,
    pub source_start_utf16: u64,
    pub source_old_end_utf16: u64,
    pub source_new_start_utf16: u64,
    pub source_new_end_utf16: u64,
}

/// Revision-bound caret geometry and the absolute scroll target required to
/// reveal it in a native viewport.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuEditorCaretScrollRequest {
    pub revision: u64,
    pub source_utf16: u64,
    pub block_index: u64,
    pub caret_x: f32,
    pub caret_y: f32,
    pub caret_width: f32,
    pub caret_height: f32,
    pub current_scroll_y: f32,
    pub target_scroll_y: f32,
    pub margin: f32,
    pub needs_scroll: u8,
}

/// Owned CoreText metrics copied across the macOS FFI boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuCoreTextViewportMetrics {
    pub line_height: f32,
    pub default_advance: f32,
}

/// One source-backed line returned by the macOS CoreText shaped-layout probe.
/// Ranges use UTF-16 units so AppKit/TextKit can compare them without a second
/// native coordinate conversion.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuCoreTextShapedLine {
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub width: f32,
}

/// One source-backed line returned by the projection-aware macOS shaped-layout
/// probe. Source and visual ranges use UTF-16 units for direct TextKit
/// comparison; the visual text itself is returned by the same count/fill call.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuCoreTextProjectedLine {
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub visual_start_utf16: u64,
    pub visual_end_utf16: u64,
    pub width: f32,
}

/// Opaque state owned by the Rust side of the native composition bridge.
#[repr(C)]
pub struct YuCompositionSession {
    document: EditorDocument,
}

fn read_utf8<'a>(pointer: *const u8, length: usize) -> Result<&'a str, i32> {
    if length == 0 {
        return Ok("");
    }
    if pointer.is_null() {
        return Err(YU_FFI_NULL_POINTER);
    }

    // SAFETY: callers provide a pointer/length pair for an immutable UTF-8
    // byte buffer that remains valid for the duration of the call.
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    std::str::from_utf8(bytes).map_err(|_| YU_FFI_INVALID_UTF8)
}

fn write_bytes(bytes: &[u8], output: *mut u8, capacity: usize) -> i32 {
    if bytes.is_empty() {
        return YU_FFI_OK;
    }
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    if capacity < bytes.len() {
        return YU_FFI_BUFFER_TOO_SMALL;
    }

    // SAFETY: capacity was checked against the source length and output is
    // required to point to writable storage owned by the caller.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
    YU_FFI_OK
}

fn source_range_from_utf16(
    session: &YuCompositionSession,
    start: u64,
    end: u64,
) -> Result<TextRange, i32> {
    let range = Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end))
        .ok_or(YU_FFI_INVALID_RANGE)?;
    let snapshot = session.document.snapshot();
    let source_start = snapshot
        .byte_offset_for_utf16(range.start())
        .map_err(|_| YU_FFI_INVALID_RANGE)?;
    let source_end = snapshot
        .byte_offset_for_utf16(range.end())
        .map_err(|_| YU_FFI_INVALID_RANGE)?;
    TextRange::new(source_start, source_end).ok_or(YU_FFI_INVALID_RANGE)
}

fn validate_revision(session: &YuCompositionSession, expected: u64) -> Result<(), i32> {
    if session.document.revision().get() != expected {
        return Err(YU_FFI_STALE_REVISION);
    }
    Ok(())
}

fn status_from_document_error(error: EditorDocumentError) -> i32 {
    match error {
        EditorDocumentError::Composition(_) => YU_FFI_INVALID_SELECTION,
        EditorDocumentError::Edit(EditError::StaleRevision { .. }) => YU_FFI_STALE_REVISION,
        EditorDocumentError::Edit(_) | EditorDocumentError::CompositionActive => YU_FFI_EDIT_FAILED,
        EditorDocumentError::Layout(_) => YU_FFI_INVALID_RANGE,
        EditorDocumentError::Markdown(_) => YU_FFI_EDIT_FAILED,
        EditorDocumentError::Position(_) => YU_FFI_INVALID_RANGE,
        EditorDocumentError::Projection(_) => YU_FFI_INVALID_RANGE,
        EditorDocumentError::Viewport(_) => YU_FFI_INVALID_RANGE,
        EditorDocumentError::BlockOutOfBounds { .. }
        | EditorDocumentError::BlockNotTaskList { .. } => YU_FFI_INVALID_RANGE,
        EditorDocumentError::Selection(SelectionError::StaleRevision { .. }) => {
            YU_FFI_STALE_REVISION
        }
        EditorDocumentError::Selection(SelectionError::Position(_)) => YU_FFI_INVALID_RANGE,
        EditorDocumentError::Selection(SelectionError::AnchorMap(_))
        | EditorDocumentError::Selection(SelectionError::InvalidRange) => YU_FFI_EDIT_FAILED,
        EditorDocumentError::CompositionNotActive => YU_FFI_NO_OVERLAY,
    }
}

fn status_from_selection_error(error: SelectionError) -> i32 {
    status_from_document_error(EditorDocumentError::Selection(error))
}

fn write_command_result(
    session: &YuCompositionSession,
    result: CommandResult,
    output: *mut YuEditorCommandResult,
) -> i32 {
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    let snapshot = session.document.snapshot();
    let range = match result.selection().utf16_range(&snapshot) {
        Ok(range) => range,
        Err(error) => return status_from_selection_error(error),
    };
    // SAFETY: output was checked for null and belongs to the caller.
    let (
        source_sync,
        source_start_utf16,
        source_old_end_utf16,
        source_new_start_utf16,
        source_new_end_utf16,
    ) = match result.source_sync() {
        SourceSync::Full if result.changed() => (YU_SOURCE_SYNC_FULL, 0, 0, 0, 0),
        SourceSync::Range(change) => {
            let old_range = change.old_range();
            let new_range = change.new_range();
            (
                YU_SOURCE_SYNC_RANGE,
                old_range.start().get(),
                old_range.end().get(),
                new_range.start().get(),
                new_range.end().get(),
            )
        }
        SourceSync::None | SourceSync::Full => (YU_SOURCE_SYNC_NONE, 0, 0, 0, 0),
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe {
        *output = YuEditorCommandResult {
            revision: result.revision().get(),
            selection_start_utf16: range.start().get(),
            selection_end_utf16: range.end().get(),
            affinity: match result.selection().affinity() {
                CaretAffinity::Upstream => YU_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_CARET_AFFINITY_DOWNSTREAM,
            },
            changed: u8::from(result.changed()),
            source_sync,
            source_start_utf16,
            source_old_end_utf16,
            source_new_start_utf16,
            source_new_end_utf16,
        };
    }
    YU_FFI_OK
}

fn write_caret_scroll_request(
    session: &YuCompositionSession,
    request: CaretScrollRequest,
    output: *mut YuEditorCaretScrollRequest,
) -> i32 {
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    let snapshot = session.document.snapshot();
    let source_utf16 = match snapshot.utf16_offset(request.caret().source()) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_FFI_INVALID_RANGE,
    };
    let caret = request.caret();
    let block_index = match u64::try_from(caret.block()) {
        Ok(index) => index,
        Err(_) => return YU_FFI_INVALID_RANGE,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe {
        *output = YuEditorCaretScrollRequest {
            revision: request.revision().get(),
            source_utf16,
            block_index,
            caret_x: caret.x(),
            caret_y: caret.y(),
            caret_width: caret.width(),
            caret_height: caret.height(),
            current_scroll_y: request.current_scroll_y(),
            target_scroll_y: request.target_scroll_y(),
            margin: request.margin(),
            needs_scroll: u8::from(request.needs_scroll()),
        };
    }
    YU_FFI_OK
}

fn editor_command_from_ffi(command: u8, block: u64) -> Result<EditorCommand, i32> {
    match command {
        YU_EDITOR_COMMAND_DELETE_BACKWARD => Ok(EditorCommand::DeleteBackward),
        YU_EDITOR_COMMAND_DELETE_FORWARD => Ok(EditorCommand::DeleteForward),
        YU_EDITOR_COMMAND_MOVE_LEFT => Ok(EditorCommand::MoveLeft),
        YU_EDITOR_COMMAND_MOVE_RIGHT => Ok(EditorCommand::MoveRight),
        YU_EDITOR_COMMAND_MOVE_WORD_LEFT => Ok(EditorCommand::move_word_left()),
        YU_EDITOR_COMMAND_MOVE_WORD_RIGHT => Ok(EditorCommand::move_word_right()),
        YU_EDITOR_COMMAND_MOVE_UP => Ok(EditorCommand::move_up()),
        YU_EDITOR_COMMAND_MOVE_DOWN => Ok(EditorCommand::move_down()),
        YU_EDITOR_COMMAND_MOVE_UP_EXTEND => Ok(EditorCommand::move_up_extend()),
        YU_EDITOR_COMMAND_MOVE_DOWN_EXTEND => Ok(EditorCommand::move_down_extend()),
        YU_EDITOR_COMMAND_INSERT_NEWLINE => Ok(EditorCommand::insert_newline()),
        YU_EDITOR_COMMAND_INDENT_LIST => Ok(EditorCommand::indent_list()),
        YU_EDITOR_COMMAND_OUTDENT_LIST => Ok(EditorCommand::outdent_list()),
        YU_EDITOR_COMMAND_UNDO => Ok(EditorCommand::undo()),
        YU_EDITOR_COMMAND_REDO => Ok(EditorCommand::redo()),
        YU_EDITOR_COMMAND_TOGGLE_TASK => usize::try_from(block)
            .map(EditorCommand::toggle_task)
            .map_err(|_| YU_FFI_INVALID_RANGE),
        _ => Err(YU_FFI_INVALID_COMMAND),
    }
}

fn editor_key_from_ffi(kind: u8, value: u32) -> Result<EditorKey, i32> {
    match kind {
        YU_KEY_CHARACTER => char::from_u32(value)
            .map(EditorKey::Character)
            .ok_or(YU_FFI_INVALID_KEY),
        YU_KEY_ENTER => Ok(EditorKey::Enter),
        YU_KEY_TAB => Ok(EditorKey::Tab),
        YU_KEY_BACKSPACE => Ok(EditorKey::Backspace),
        YU_KEY_DELETE => Ok(EditorKey::Delete),
        YU_KEY_LEFT => Ok(EditorKey::Left),
        YU_KEY_RIGHT => Ok(EditorKey::Right),
        YU_KEY_UP => Ok(EditorKey::Up),
        YU_KEY_DOWN => Ok(EditorKey::Down),
        YU_KEY_ESCAPE => Ok(EditorKey::Escape),
        _ => Err(YU_FFI_INVALID_KEY),
    }
}

fn write_snapshot_range(
    snapshot: &TextSnapshot,
    range: TextRange,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if written.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    let start = match usize::try_from(range.start()) {
        Ok(start) => start,
        Err(_) => return YU_FFI_INVALID_RANGE,
    };
    let end = match usize::try_from(range.end()) {
        Ok(end) => end,
        Err(_) => return YU_FFI_INVALID_RANGE,
    };
    let required = end.saturating_sub(start);
    // SAFETY: written was checked for null and belongs to the caller.
    unsafe { *written = required };
    if required == 0 {
        return YU_FFI_OK;
    }
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    if capacity < required {
        return YU_FFI_BUFFER_TOO_SMALL;
    }

    let mut copied = 0_usize;
    let mut chunks = match snapshot.chunk_cursor(range.start()) {
        Ok(chunks) => chunks,
        Err(_) => return YU_FFI_INVALID_RANGE,
    };
    for chunk in &mut chunks {
        let chunk_start = match usize::try_from(chunk.start()) {
            Ok(start) => start,
            Err(_) => return YU_FFI_INVALID_RANGE,
        };
        if chunk_start >= end {
            break;
        }
        let chunk_end = chunk_start.saturating_add(chunk.text().len());
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start >= local_end {
            continue;
        }
        let bytes = &chunk.text().as_bytes()[local_start..local_end];
        // SAFETY: the caller supplied at least `required` writable bytes, and
        // `copied + bytes.len()` is bounded by that required length.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), output.add(copied), bytes.len());
        }
        copied += bytes.len();
    }
    if copied != required {
        return YU_FFI_INVALID_RANGE;
    }
    YU_FFI_OK
}

fn selection_from_utf16(start: u64, end: u64) -> Result<Utf16Range, i32> {
    Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end)).ok_or(YU_FFI_INVALID_SELECTION)
}

fn caret_affinity_from_ffi(value: u8) -> Result<CaretAffinity, i32> {
    match value {
        YU_CARET_AFFINITY_UPSTREAM => Ok(CaretAffinity::Upstream),
        YU_CARET_AFFINITY_DOWNSTREAM => Ok(CaretAffinity::Downstream),
        _ => Err(YU_FFI_INVALID_SELECTION),
    }
}

fn editor_selection_from_utf16(
    session: &YuCompositionSession,
    start: u64,
    end: u64,
    affinity: u8,
) -> Result<EditorSelection, i32> {
    let range = selection_from_utf16(start, end)?;
    let affinity = caret_affinity_from_ffi(affinity)?;
    let snapshot = session.document.snapshot();
    let source_start = snapshot
        .byte_offset_for_utf16(range.start())
        .map_err(|_| YU_FFI_INVALID_SELECTION)?;
    let source_end = snapshot
        .byte_offset_for_utf16(range.end())
        .map_err(|_| YU_FFI_INVALID_SELECTION)?;
    EditorSelection::range(&snapshot, source_start, source_end, affinity)
        .map_err(|_| YU_FFI_INVALID_SELECTION)
}

/// Creates an opaque composition session for a UTF-8 source buffer.
///
/// # Safety
/// `source` must point to `source_length` readable bytes (unless the length is
/// zero), and `output` must point to writable storage for the returned handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_new(
    source: *const u8,
    source_length: usize,
    output: *mut *mut YuCompositionSession,
) -> i32 {
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: output was checked for null and points to caller-owned storage.
    unsafe { *output = ptr::null_mut() };
    let source = match read_utf8(source, source_length) {
        Ok(source) => source,
        Err(status) => return status,
    };
    let session = Box::new(YuCompositionSession {
        document: EditorDocument::new(source),
    });

    // SAFETY: output was checked for null and points to caller-owned storage
    // for the opaque handle.
    unsafe { *output = Box::into_raw(session) };
    YU_FFI_OK
}

/// Destroys a session returned by `yu_composition_session_new`.
///
/// # Safety
/// `session` must be null or a live handle returned by this crate that has not
/// already been destroyed. No caller may use the handle after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_destroy(session: *mut YuCompositionSession) {
    if !session.is_null() {
        // SAFETY: the handle came from `yu_composition_session_new` and is
        // destroyed at most once by the caller.
        unsafe { drop(Box::from_raw(session)) };
    }
}

/// Replaces the canonical source when no composition is active.
///
/// # Safety
/// `session` must be null or a live handle. `source` must point to
/// `source_length` readable bytes (unless the length is zero).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_reset_source(
    session: *mut YuCompositionSession,
    source: *const u8,
    source_length: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let source = match read_utf8(source, source_length) {
        Ok(source) => source,
        Err(status) => return status,
    };
    session
        .document
        .reset_source(source)
        .map_or_else(status_from_document_error, |_| YU_FFI_OK)
}

/// Executes one revision-independent editor command and returns the resulting
/// UTF-16 selection. The command is applied to canonical source; an active
/// composition must be committed or cancelled by the native text-input layer
/// before calling this entry point.
///
/// `block` is only read for `YU_EDITOR_COMMAND_TOGGLE_TASK` and is otherwise
/// ignored.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one [`YuEditorCommandResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_execute_command(
    session: *mut YuCompositionSession,
    command: u8,
    block: u64,
    output: *mut YuEditorCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if session.document.composition().is_some() {
        return YU_FFI_EDIT_FAILED;
    }
    let command = match editor_command_from_ffi(command, block) {
        Ok(command) => command,
        Err(status) => return status,
    };
    let result = match session.document.execute(command) {
        Ok(result) => result,
        Err(error) => return status_from_document_error(error),
    };
    write_command_result(session, result, output)
}

/// Reports whether a command is currently enabled for native menu or
/// selector validation. The query does not mutate source, selection, history,
/// or composition state.
///
/// `block` is only read for `YU_EDITOR_COMMAND_TOGGLE_TASK` and is otherwise
/// ignored.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one command availability byte.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_command_available(
    session: *mut YuCompositionSession,
    command: u8,
    block: u64,
    output: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    let command = match editor_command_from_ffi(command, block) {
        Ok(command) => command,
        Err(status) => return status,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe {
        *output = if session.document.command_available(&command) {
            YU_COMMAND_AVAILABLE
        } else {
            YU_COMMAND_UNAVAILABLE
        };
    }
    YU_FFI_OK
}

/// Resolves one native logical key and, when it is a Yu command shortcut,
/// executes it against canonical source. A return value of
/// [`YU_FFI_KEY_UNHANDLED`] means the caller should pass the event to its
/// native text-input/default command path; source and selection are unchanged.
///
/// `key_kind` uses the `YU_KEY_*` constants. For `YU_KEY_CHARACTER`, `key`
/// contains one Unicode scalar value; for special keys it is ignored.
/// `modifiers` uses the `YU_KEY_MODIFIER_*` bits.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one [`YuEditorCommandResult`] when the key is handled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_route_key(
    session: *mut YuCompositionSession,
    key_kind: u8,
    key: u32,
    modifiers: u8,
    output: *mut YuEditorCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let key = match editor_key_from_ffi(key_kind, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    let event = KeyEvent::new(key, KeyModifiers::from_bits(modifiers));
    let route = match session.document.route_key(event) {
        Ok(route) => route,
        Err(error) => return status_from_document_error(error),
    };
    let KeyRouteResult::Executed(result) = route else {
        return YU_FFI_KEY_UNHANDLED;
    };
    write_command_result(session, result, output)
}

/// Starts a composition overlay using UTF-16 source and selection ranges.
///
/// # Safety
/// `session` must be null or a live handle. `preedit` must point to
/// `preedit_length` readable bytes (unless the length is zero).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_begin(
    session: *mut YuCompositionSession,
    replacement_start_utf16: u64,
    replacement_end_utf16: u64,
    preedit: *const u8,
    preedit_length: usize,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let replacement =
        match source_range_from_utf16(session, replacement_start_utf16, replacement_end_utf16) {
            Ok(range) => range,
            Err(status) => return status,
        };
    let preedit = match read_utf8(preedit, preedit_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let selection = match selection_from_utf16(selection_start_utf16, selection_end_utf16) {
        Ok(selection) => selection,
        Err(status) => return status,
    };
    session
        .document
        .begin_composition(replacement, preedit, selection)
        .map_or_else(status_from_document_error, |_| YU_FFI_OK)
}

/// Updates the preedit text and its UTF-16 selection.
///
/// # Safety
/// `session` must be null or a live handle. `preedit` must point to
/// `preedit_length` readable bytes (unless the length is zero).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_update(
    session: *mut YuCompositionSession,
    preedit: *const u8,
    preedit_length: usize,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let preedit = match read_utf8(preedit, preedit_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let selection = match selection_from_utf16(selection_start_utf16, selection_end_utf16) {
        Ok(selection) => selection,
        Err(status) => return status,
    };
    session
        .document
        .update_composition(preedit, selection)
        .map_or_else(status_from_document_error, |_| YU_FFI_OK)
}

/// Commits the active composition as one permanent text transaction.
///
/// # Safety
/// `session` must be null or a live handle. `committed_text` must point to
/// `committed_length` readable bytes (unless the length is zero).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_commit(
    session: *mut YuCompositionSession,
    committed_text: *const u8,
    committed_length: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let committed_text = match read_utf8(committed_text, committed_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    session
        .document
        .commit_composition(committed_text)
        .map_or_else(status_from_document_error, |_| YU_FFI_OK)
}

/// Cancels and drops the active composition overlay.
///
/// # Safety
/// `session` must be null or a live handle owned by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_cancel(session: *mut YuCompositionSession) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let _ = session.document.cancel_composition();
    YU_FFI_OK
}

/// Reads the canonical source revision.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_revision(
    session: *const YuCompositionSession,
    output: *mut u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = session.document.revision().get() };
    YU_FFI_OK
}

/// Reads the canonical source selection in UTF-16 coordinates.
///
/// The returned revision is the revision that owns both endpoints. Native
/// adapters should compare it with their last source revision before using the
/// range for a follow-up edit. The affinity output uses the
/// `YU_CARET_AFFINITY_*` constants.
///
/// # Safety
/// `session` must be null or a live handle. The revision/start/end output
/// pointers must point to writable storage for one `u64`; affinity output must
/// point to writable storage for one `u8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_selection(
    session: *const YuCompositionSession,
    revision_output: *mut u64,
    start_output: *mut u64,
    end_output: *mut u64,
    affinity_output: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if revision_output.is_null()
        || start_output.is_null()
        || end_output.is_null()
        || affinity_output.is_null()
    {
        return YU_FFI_NULL_POINTER;
    }
    let snapshot = session.document.snapshot();
    let selection = session.document.selection();
    let range = match selection.utf16_range(&snapshot) {
        Ok(range) => range,
        Err(error) => return status_from_selection_error(error),
    };
    // SAFETY: all output pointers were checked for null and belong to caller.
    unsafe {
        *revision_output = snapshot.revision().get();
        *start_output = range.start().get();
        *end_output = range.end().get();
        *affinity_output = match selection.affinity() {
            CaretAffinity::Upstream => YU_CARET_AFFINITY_UPSTREAM,
            CaretAffinity::Downstream => YU_CARET_AFFINITY_DOWNSTREAM,
        };
    }
    YU_FFI_OK
}

/// Sets the canonical source selection from a revision-bound UTF-16 range.
///
/// The native adapter must provide the revision it used to calculate the
/// range and one of the `YU_CARET_AFFINITY_*` constants. A stale revision or
/// unknown affinity leaves the current selection unchanged.
///
/// # Safety
/// `session` must be null or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_set_selection(
    session: *mut YuCompositionSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    affinity: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let selection = match editor_selection_from_utf16(session, start_utf16, end_utf16, affinity) {
        Ok(selection) => selection,
        Err(status) => return status,
    };
    session
        .document
        .set_selection(selection)
        .map_or_else(status_from_selection_error, |_| YU_FFI_OK)
}

/// Applies native viewport metrics to the revision-bound Rust layout policy.
///
/// The metrics-only backend uses `default_advance` for grapheme clusters that
/// do not yet have a platform shaper. This call does not change source or
/// selection revision; callers still provide the revision they used to derive
/// the metrics so a stale host cannot reconfigure a newer document.
///
/// # Safety
/// `session` must be null or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_set_viewport_config(
    session: *mut YuCompositionSession,
    expected_revision: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    estimated_block_height: f32,
    overscan: f32,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let config = ViewportConfig::new(
        LayoutConfig::new(max_width, line_height).with_default_advance(default_advance),
        estimated_block_height,
        overscan,
    );
    session
        .document
        .set_viewport_config(config)
        .map_or(YU_FFI_INVALID_VIEWPORT_CONFIG, |_| YU_FFI_OK)
}

/// Measures a UTF-8 sample with the macOS CoreText shaper and returns owned
/// point-based metrics. This helper is independent of an editor session;
/// callers can publish the result through `yu_composition_session_set_viewport_config`.
///
/// # Safety
/// `family` and `sample` must point to readable UTF-8 buffers for their
/// respective lengths (unless a length is zero), and `output` must point to
/// writable storage for one [`YuCoreTextViewportMetrics`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_macos_core_text_viewport_metrics(
    family: *const u8,
    family_length: usize,
    size: f32,
    sample: *const u8,
    sample_length: usize,
    output: *mut YuCoreTextViewportMetrics,
) -> i32 {
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuCoreTextViewportMetrics::default() };

    #[cfg(target_os = "macos")]
    {
        let family = match read_utf8(family, family_length) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let sample = match read_utf8(sample, sample_length) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let request = match FontRequest::new(family, size) {
            Ok(request) => request,
            Err(_) => return YU_FFI_CORE_TEXT_UNAVAILABLE,
        };
        let metrics = match query_core_text_viewport_metrics(request, sample, false) {
            Ok(metrics) => metrics,
            Err(status) => return status,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe {
            *output = YuCoreTextViewportMetrics {
                line_height: metrics.line_height(),
                default_advance: metrics.default_advance(),
            };
        }
        YU_FFI_OK
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (family, family_length, size, sample, sample_length);
        YU_FFI_CORE_TEXT_UNAVAILABLE
    }
}

/// Measures the AppKit/CoreText system UI font without passing its private
/// internal family name through `CTFontCreateWithName`.
///
/// # Safety
/// `sample` must point to readable UTF-8 bytes for its length (unless the
/// length is zero), and `output` must point to writable storage for one
/// [`YuCoreTextViewportMetrics`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_macos_core_text_system_ui_viewport_metrics(
    size: f32,
    sample: *const u8,
    sample_length: usize,
    output: *mut YuCoreTextViewportMetrics,
) -> i32 {
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuCoreTextViewportMetrics::default() };

    #[cfg(target_os = "macos")]
    {
        let sample = match read_utf8(sample, sample_length) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let request = match FontRequest::new("System UI", size) {
            Ok(request) => request,
            Err(_) => return YU_FFI_CORE_TEXT_UNAVAILABLE,
        };
        let metrics = match query_core_text_viewport_metrics(request, sample, true) {
            Ok(metrics) => metrics,
            Err(status) => return status,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe {
            *output = YuCoreTextViewportMetrics {
                line_height: metrics.line_height(),
                default_advance: metrics.default_advance(),
            };
        }
        YU_FFI_OK
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (size, sample, sample_length);
        YU_FFI_CORE_TEXT_UNAVAILABLE
    }
}

/// Builds a diagnostic shaped layout with the macOS system UI font and copies
/// its source-backed line ranges/widths into caller-owned storage. This does
/// not mutate an editor session and is intended for comparing the shared Rust
/// layout contract with TextKit before a shaped viewport becomes canonical.
///
/// A zero-capacity call with a non-null `written` pointer returns the required
/// line count. If `capacity` is too small, the function writes the required
/// count and returns [`YU_FFI_BUFFER_TOO_SMALL`].
///
/// # Safety
/// `source` must point to readable UTF-8 bytes for its length (unless the
/// length is zero), `output` must point to `capacity` writable line values when
/// `capacity > 0`, and `written` must point to writable storage for one count.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_macos_core_text_shaped_lines(
    size: f32,
    max_width: f32,
    source: *const u8,
    source_length: usize,
    output: *mut YuCoreTextShapedLine,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if written.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    if capacity > 0 && output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: written was checked for null and belongs to the caller.
    unsafe { *written = 0 };

    #[cfg(target_os = "macos")]
    {
        let source = match read_utf8(source, source_length) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let lines = match query_core_text_shaped_lines(source, size, max_width) {
            Ok(lines) => lines,
            Err(status) => return status,
        };
        // SAFETY: written was checked for null and belongs to the caller.
        unsafe { *written = lines.len() };
        if capacity == 0 {
            return YU_FFI_OK;
        }
        if lines.len() > capacity {
            return YU_FFI_BUFFER_TOO_SMALL;
        }
        if lines.is_empty() {
            return YU_FFI_OK;
        }
        // SAFETY: capacity was checked against the number of values and output
        // was checked for null when capacity was non-zero.
        unsafe { ptr::copy_nonoverlapping(lines.as_ptr(), output, lines.len()) };
        YU_FFI_OK
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (size, max_width, source, source_length, output, capacity);
        YU_FFI_CORE_TEXT_UNAVAILABLE
    }
}

/// Builds a diagnostic shaped layout after applying the shared Markdown
/// projection. The call returns both source-backed line ranges and the
/// projected UTF-8 text that a native TextKit mirror can lay out directly.
/// This does not mutate an editor session or canonical source state.
///
/// `line_written` and `visual_written` are both required. A zero-capacity call
/// returns the required line count and projected UTF-8 byte count. If either
/// caller-owned capacity is too small, the function returns
/// [`YU_FFI_BUFFER_TOO_SMALL`] without partially writing either output.
///
/// # Safety
/// `source` must point to readable UTF-8 bytes for its length (unless the
/// length is zero). `lines` must point to `line_capacity` writable line values
/// when that capacity is non-zero; `visual_output` must point to
/// `visual_capacity` writable bytes when that capacity is non-zero. Both
/// written pointers must point to writable storage for one count.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_macos_core_text_projected_layout(
    size: f32,
    max_width: f32,
    source: *const u8,
    source_length: usize,
    lines: *mut YuCoreTextProjectedLine,
    line_capacity: usize,
    line_written: *mut usize,
    visual_output: *mut u8,
    visual_capacity: usize,
    visual_written: *mut usize,
) -> i32 {
    if line_written.is_null() || visual_written.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    if line_capacity > 0 && lines.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    if visual_capacity > 0 && visual_output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: both written pointers were checked for null and belong to the
    // caller.
    unsafe {
        *line_written = 0;
        *visual_written = 0;
    }

    #[cfg(target_os = "macos")]
    {
        let source = match read_utf8(source, source_length) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let (projected_lines, projected) =
            match query_core_text_projected_layout(source, size, max_width) {
                Ok(value) => value,
                Err(status) => return status,
            };
        let projected_bytes = projected.as_bytes();
        // SAFETY: both written pointers were checked for null and belong to
        // the caller.
        unsafe {
            *line_written = projected_lines.len();
            *visual_written = projected_bytes.len();
        }
        if line_capacity == 0 && visual_capacity == 0 {
            return YU_FFI_OK;
        }
        if projected_lines.len() > line_capacity || projected_bytes.len() > visual_capacity {
            return YU_FFI_BUFFER_TOO_SMALL;
        }
        if !projected_lines.is_empty() {
            // SAFETY: line capacity was checked against the number of values
            // and the output pointer was checked when capacity was non-zero.
            unsafe {
                ptr::copy_nonoverlapping(projected_lines.as_ptr(), lines, projected_lines.len())
            };
        }
        if !projected_bytes.is_empty() {
            // SAFETY: visual capacity was checked against the byte length and
            // the output pointer was checked when capacity was non-zero.
            unsafe {
                ptr::copy_nonoverlapping(
                    projected_bytes.as_ptr(),
                    visual_output,
                    projected_bytes.len(),
                )
            };
        }
        YU_FFI_OK
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            size,
            max_width,
            source,
            source_length,
            lines,
            line_capacity,
            visual_output,
            visual_capacity,
        );
        YU_FFI_CORE_TEXT_UNAVAILABLE
    }
}

#[cfg(target_os = "macos")]
fn query_core_text_viewport_metrics(
    request: FontRequest,
    sample: &str,
    system_ui: bool,
) -> Result<CoreTextViewportMetrics, i32> {
    let shaper = if system_ui {
        CoreTextShaper::from_system_ui(request)
    } else {
        CoreTextShaper::from_system(request)
    }
    .map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)?;
    shaper
        .viewport_metrics(sample)
        .map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)
}

#[cfg(target_os = "macos")]
fn query_core_text_shaped_lines(
    source: &str,
    size: f32,
    max_width: f32,
) -> Result<Vec<YuCoreTextShapedLine>, i32> {
    let request = FontRequest::new("System UI", size).map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)?;
    let shaper =
        CoreTextShaper::from_system_ui(request).map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)?;
    let metrics = shaper
        .viewport_metrics("M中🙂e\u{301}")
        .map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)?;
    if !max_width.is_finite() || max_width <= 0.0 {
        return Err(YU_FFI_LAYOUT_FAILED);
    }
    let config = LayoutConfig::new(max_width, metrics.line_height())
        .with_default_advance(metrics.default_advance());
    let mut document = EditorDocument::new(source.to_owned());
    let snapshot = document.snapshot();
    let block_count = document.markdown().blocks().len();
    let mut lines = Vec::new();
    for index in 0..block_count {
        let layout = document
            .block_layout_with_shaper(index, config, &shaper)
            .map_err(|_| YU_FFI_LAYOUT_FAILED)?;
        for line in layout.lines() {
            let start = snapshot
                .utf16_offset(line.source().start())
                .map_err(|_| YU_FFI_LAYOUT_FAILED)?;
            let end = snapshot
                .utf16_offset(line.source().end())
                .map_err(|_| YU_FFI_LAYOUT_FAILED)?;
            lines.push(YuCoreTextShapedLine {
                source_start_utf16: start.get(),
                source_end_utf16: end.get(),
                width: line.width(),
            });
        }
    }
    Ok(lines)
}

#[cfg(target_os = "macos")]
fn query_core_text_projected_layout(
    source: &str,
    size: f32,
    max_width: f32,
) -> Result<(Vec<YuCoreTextProjectedLine>, String), i32> {
    let request = FontRequest::new("System UI", size).map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)?;
    let shaper =
        CoreTextShaper::from_system_ui(request).map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)?;
    let metrics = shaper
        .viewport_metrics("M中🙂e\u{301}")
        .map_err(|_| YU_FFI_CORE_TEXT_UNAVAILABLE)?;
    if !max_width.is_finite() || max_width <= 0.0 {
        return Err(YU_FFI_LAYOUT_FAILED);
    }
    let document = EditorDocument::new(source.to_owned());
    let snapshot = document.snapshot();
    let source_range =
        TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).ok_or(YU_FFI_LAYOUT_FAILED)?;
    let projection =
        Projection::inline(&snapshot, source_range).map_err(|_| YU_FFI_LAYOUT_FAILED)?;
    let projected = projected_utf8(&projection)?;
    let config = LayoutConfig::new(max_width, metrics.line_height())
        .with_default_advance(metrics.default_advance());
    let layout = LayoutSnapshot::from_projection_with_shaper(&projection, config, &shaper)
        .map_err(|_| YU_FFI_LAYOUT_FAILED)?;
    let mut lines = Vec::with_capacity(layout.lines().len());
    for line in layout.lines() {
        let source_start = snapshot
            .utf16_offset(line.source().start())
            .map_err(|_| YU_FFI_LAYOUT_FAILED)?;
        let source_end = snapshot
            .utf16_offset(line.source().end())
            .map_err(|_| YU_FFI_LAYOUT_FAILED)?;
        let visual_start = utf16_offset_in_utf8(&projected, line.visual().start().get())?;
        let visual_end = utf16_offset_in_utf8(&projected, line.visual().end().get())?;
        lines.push(YuCoreTextProjectedLine {
            source_start_utf16: source_start.get(),
            source_end_utf16: source_end.get(),
            visual_start_utf16: visual_start,
            visual_end_utf16: visual_end,
            width: line.width(),
        });
    }
    Ok((lines, projected))
}

#[cfg(target_os = "macos")]
fn projected_utf8(projection: &Projection) -> Result<String, i32> {
    let mut bytes = Vec::new();
    for run in projection.runs() {
        if !matches!(
            run.kind(),
            VisualRunKind::Visible | VisualRunKind::LineBreak { .. }
        ) {
            continue;
        }
        let start = usize::try_from(run.source().start()).map_err(|_| YU_FFI_LAYOUT_FAILED)?;
        let end = usize::try_from(run.source().end()).map_err(|_| YU_FFI_LAYOUT_FAILED)?;
        let mut cursor = projection
            .source()
            .chunk_cursor(run.source().start())
            .map_err(|_| YU_FFI_LAYOUT_FAILED)?;
        for chunk in &mut cursor {
            let chunk_start = usize::try_from(chunk.start()).map_err(|_| YU_FFI_LAYOUT_FAILED)?;
            let chunk_end = chunk_start
                .checked_add(chunk.text().len())
                .ok_or(YU_FFI_LAYOUT_FAILED)?;
            if chunk_start >= end {
                break;
            }
            let local_start = start.max(chunk_start).saturating_sub(chunk_start);
            let local_end = end.min(chunk_end).saturating_sub(chunk_start);
            if local_start < local_end {
                bytes.extend_from_slice(&chunk.text().as_bytes()[local_start..local_end]);
            }
        }
    }
    String::from_utf8(bytes).map_err(|_| YU_FFI_LAYOUT_FAILED)
}

#[cfg(target_os = "macos")]
fn utf16_offset_in_utf8(source: &str, byte_offset: u64) -> Result<u64, i32> {
    let byte_offset = usize::try_from(byte_offset).map_err(|_| YU_FFI_LAYOUT_FAILED)?;
    let prefix = source
        .as_bytes()
        .get(..byte_offset)
        .ok_or(YU_FFI_LAYOUT_FAILED)?;
    let prefix = std::str::from_utf8(prefix).map_err(|_| YU_FFI_LAYOUT_FAILED)?;
    u64::try_from(prefix.encode_utf16().count()).map_err(|_| YU_FFI_LAYOUT_FAILED)
}

/// Resolves the current focus caret and returns an absolute document-space
/// scroll target for a native viewport. The query is bound to
/// `expected_revision`; stale UI geometry must be discarded by the caller.
/// `margin` is clamped to half the viewport height by the Rust policy.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one [`YuEditorCaretScrollRequest`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_caret_scroll_request(
    session: *mut YuCompositionSession,
    expected_revision: u64,
    scroll_y: f32,
    viewport_height: f32,
    margin: f32,
    output: *mut YuEditorCaretScrollRequest,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let request = match session
        .document
        .caret_scroll_request(ViewportRect::new(scroll_y, viewport_height), margin)
    {
        Ok(request) => request,
        Err(error) => return status_from_document_error(error),
    };
    write_caret_scroll_request(session, request, output)
}

/// Reads the canonical source byte length.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_source_length(
    session: *const YuCompositionSession,
    output: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = session.document.snapshot().len_bytes().get() as usize };
    YU_FFI_OK
}

/// Copies the canonical source into caller-owned storage.
///
/// # Safety
/// `session` must be null or a live handle. When the source is non-empty,
/// `output` must point to writable storage with at least `capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_copy_source(
    session: *const YuCompositionSession,
    output: *mut u8,
    capacity: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let snapshot = session.document.snapshot();
    let source = snapshot.as_str();
    write_bytes(source.as_bytes(), output, capacity)
}

/// Reads the UTF-8 byte length of a UTF-16 source range at an expected revision.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_source_range_length(
    session: *const YuCompositionSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    output: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let range = match source_range_from_utf16(session, start_utf16, end_utf16) {
        Ok(range) => range,
        Err(status) => return status,
    };
    let length = match usize::try_from(range.len()) {
        Ok(length) => length,
        Err(_) => return YU_FFI_INVALID_RANGE,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = length };
    YU_FFI_OK
}

/// Copies a UTF-16 source range without materializing the whole document.
///
/// `written` receives the required/actual UTF-8 byte count. The query is
/// rejected when `expected_revision` is stale.
///
/// # Safety
/// `session` must be null or a live handle. `written` must point to writable
/// storage for one `size_t`. When the range is non-empty, `output` must point
/// to writable storage with at least `capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_copy_source_range(
    session: *const YuCompositionSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let range = match source_range_from_utf16(session, start_utf16, end_utf16) {
        Ok(range) => range,
        Err(status) => return status,
    };
    let snapshot = session.document.snapshot();
    write_snapshot_range(&snapshot, range, output, capacity, written)
}

/// Reads the active preedit UTF-8 byte length, or zero when inactive.
///
/// # Safety
/// `session` must be null or a live handle. `output` must point to writable
/// storage for one `size_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_overlay_length(
    session: *const YuCompositionSession,
    output: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe {
        *output = session
            .document
            .composition()
            .map_or(0, |overlay| overlay.text().len());
    }
    YU_FFI_OK
}

/// Copies the active preedit into caller-owned storage.
///
/// # Safety
/// `session` must be null or a live handle. When the preedit is non-empty,
/// `output` must point to writable storage with at least `capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_copy_overlay(
    session: *const YuCompositionSession,
    output: *mut u8,
    capacity: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    let Some(overlay) = session.document.composition() else {
        return YU_FFI_NO_OVERLAY;
    };
    write_bytes(overlay.text().as_bytes(), output, capacity)
}

/// Reads the active preedit UTF-16 selection.
///
/// # Safety
/// `session` must be null or a live handle. Both output pointers must point to
/// writable storage for one `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_composition_session_overlay_selection(
    session: *const YuCompositionSession,
    start_output: *mut u64,
    end_output: *mut u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_FFI_NULL_POINTER;
    };
    if start_output.is_null() || end_output.is_null() {
        return YU_FFI_NULL_POINTER;
    }
    let Some(overlay) = session.document.composition() else {
        return YU_FFI_NO_OVERLAY;
    };
    let selection = overlay.selection_utf16();
    // SAFETY: both output pointers were checked for null and belong to caller.
    unsafe {
        *start_output = selection.start().get();
        *end_output = selection.end().get();
    }
    YU_FFI_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    use yu_font_macos::CoreTextFontCatalog;

    fn session(source: &str) -> *mut YuCompositionSession {
        let mut output = ptr::null_mut();
        let status =
            unsafe { yu_composition_session_new(source.as_ptr(), source.len(), &mut output) };
        assert_eq!(status, YU_FFI_OK);
        output
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_core_text_viewport_metrics_returns_owned_native_values() {
        let family = CoreTextFontCatalog::system()
            .expect("CoreText should expose families")
            .families()[0]
            .clone();
        let sample = "M中🙂e\u{301}";
        let mut metrics = YuCoreTextViewportMetrics::default();
        assert_eq!(
            unsafe {
                yu_macos_core_text_viewport_metrics(
                    family.as_bytes().as_ptr(),
                    family.len(),
                    22.0,
                    sample.as_bytes().as_ptr(),
                    sample.len(),
                    &mut metrics,
                )
            },
            YU_FFI_OK
        );
        assert!(metrics.line_height.is_finite() && metrics.line_height > 0.0);
        assert!(metrics.default_advance.is_finite() && metrics.default_advance > 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_core_text_system_ui_metrics_use_owned_native_values() {
        let sample = "M中🙂e\u{301}";
        let mut metrics = YuCoreTextViewportMetrics::default();
        assert_eq!(
            unsafe {
                yu_macos_core_text_system_ui_viewport_metrics(
                    22.0,
                    sample.as_bytes().as_ptr(),
                    sample.len(),
                    &mut metrics,
                )
            },
            YU_FFI_OK
        );
        assert!(metrics.line_height.is_finite() && metrics.line_height > 0.0);
        assert!(metrics.default_advance.is_finite() && metrics.default_advance > 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_core_text_shaped_lines_round_trip_utf16_ranges() {
        let source = "Yu 中文🙂 line that wraps\nsecond line";
        let mut required = 0_usize;
        assert_eq!(
            unsafe {
                yu_macos_core_text_shaped_lines(
                    22.0,
                    120.0,
                    source.as_bytes().as_ptr(),
                    source.len(),
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_FFI_OK
        );
        assert!(required >= 2);

        let mut short = vec![YuCoreTextShapedLine::default(); required - 1];
        let mut short_written = 0_usize;
        assert_eq!(
            unsafe {
                yu_macos_core_text_shaped_lines(
                    22.0,
                    120.0,
                    source.as_bytes().as_ptr(),
                    source.len(),
                    short.as_mut_ptr(),
                    short.len(),
                    &mut short_written,
                )
            },
            YU_FFI_BUFFER_TOO_SMALL
        );
        assert_eq!(short_written, required);

        let mut lines = vec![YuCoreTextShapedLine::default(); required];
        let mut written = 0_usize;
        assert_eq!(
            unsafe {
                yu_macos_core_text_shaped_lines(
                    22.0,
                    120.0,
                    source.as_bytes().as_ptr(),
                    source.len(),
                    lines.as_mut_ptr(),
                    lines.len(),
                    &mut written,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(written, required);
        assert!(lines.windows(2).all(|pair| {
            pair[0].source_start_utf16 <= pair[0].source_end_utf16
                && pair[0].source_end_utf16 <= pair[1].source_start_utf16
                && pair[0].width.is_finite()
                && pair[1].width.is_finite()
        }));
        assert!(
            lines
                .last()
                .is_some_and(|line| line.source_end_utf16 == source.encode_utf16().count() as u64)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_core_text_projected_layout_returns_visual_utf16_ranges() {
        let source = "This is **Yu** and [Rust](https://example.com) with 中文🙂 words.\n";
        let mut required_lines = 0_usize;
        let mut required_visual_bytes = 0_usize;
        assert_eq!(
            unsafe {
                yu_macos_core_text_projected_layout(
                    22.0,
                    240.0,
                    source.as_bytes().as_ptr(),
                    source.len(),
                    ptr::null_mut(),
                    0,
                    &mut required_lines,
                    ptr::null_mut(),
                    0,
                    &mut required_visual_bytes,
                )
            },
            YU_FFI_OK
        );
        assert!(required_lines > 0);
        assert!(required_visual_bytes > 0);

        let mut short_visual = vec![0_u8; required_visual_bytes - 1];
        let mut short_lines = vec![YuCoreTextProjectedLine::default(); required_lines];
        let mut short_line_written = 0_usize;
        let mut short_visual_written = 0_usize;
        assert_eq!(
            unsafe {
                yu_macos_core_text_projected_layout(
                    22.0,
                    240.0,
                    source.as_bytes().as_ptr(),
                    source.len(),
                    short_lines.as_mut_ptr(),
                    short_lines.len(),
                    &mut short_line_written,
                    short_visual.as_mut_ptr(),
                    short_visual.len(),
                    &mut short_visual_written,
                )
            },
            YU_FFI_BUFFER_TOO_SMALL
        );
        assert_eq!(short_line_written, required_lines);
        assert_eq!(short_visual_written, required_visual_bytes);

        let mut lines = vec![YuCoreTextProjectedLine::default(); required_lines];
        let mut visual = vec![0_u8; required_visual_bytes];
        let mut written_lines = 0_usize;
        let mut written_visual_bytes = 0_usize;
        assert_eq!(
            unsafe {
                yu_macos_core_text_projected_layout(
                    22.0,
                    240.0,
                    source.as_bytes().as_ptr(),
                    source.len(),
                    lines.as_mut_ptr(),
                    lines.len(),
                    &mut written_lines,
                    visual.as_mut_ptr(),
                    visual.len(),
                    &mut written_visual_bytes,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(written_lines, required_lines);
        assert_eq!(written_visual_bytes, required_visual_bytes);
        let projected = String::from_utf8(visual).expect("projection must remain UTF-8");
        assert_eq!(projected, "This is Yu and Rust with 中文🙂 words.\n");
        let source_len = source.encode_utf16().count() as u64;
        let visual_len = projected.encode_utf16().count() as u64;
        assert!(lines.iter().all(|line| {
            line.source_start_utf16 <= line.source_end_utf16
                && line.source_end_utf16 <= source_len
                && line.visual_start_utf16 <= line.visual_end_utf16
                && line.visual_end_utf16 <= visual_len
                && line.width.is_finite()
        }));
        assert!(lines.windows(2).all(|pair| {
            pair[0].source_end_utf16 <= pair[1].source_start_utf16
                && pair[0].visual_end_utf16 <= pair[1].visual_start_utf16
        }));
        assert!(lines.iter().any(|line| {
            line.source_end_utf16 - line.source_start_utf16
                > line.visual_end_utf16 - line.visual_start_utf16
        }));
    }

    #[test]
    fn ffi_session_maps_utf16_ranges_and_commits_once() {
        let handle = session("输入: ");
        assert_eq!(
            unsafe {
                yu_composition_session_begin(
                    handle,
                    4,
                    4,
                    "にほんご".as_ptr(),
                    "にほんご".len(),
                    4,
                    4,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe {
                yu_composition_session_update(handle, "にほんご".as_ptr(), "にほんご".len(), 4, 4)
            },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe {
                yu_composition_session_commit(handle, "日本語".as_ptr(), "日本語".len())
            },
            YU_FFI_OK
        );

        let mut length = 0;
        assert_eq!(
            unsafe { yu_composition_session_source_length(handle, &mut length) },
            YU_FFI_OK
        );
        let mut bytes = vec![0_u8; length];
        assert_eq!(
            unsafe { yu_composition_session_copy_source(handle, bytes.as_mut_ptr(), bytes.len()) },
            YU_FFI_OK
        );
        assert_eq!(
            std::str::from_utf8(&bytes).expect("source should stay UTF-8"),
            "输入: 日本語"
        );

        let mut selection_revision = 0;
        let mut selection_start = 0;
        let mut selection_end = 0;
        let mut selection_affinity = 0;
        assert_eq!(
            unsafe {
                yu_composition_session_selection(
                    handle,
                    &mut selection_revision,
                    &mut selection_start,
                    &mut selection_end,
                    &mut selection_affinity,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(selection_revision, 1);
        assert_eq!(selection_start, 7);
        assert_eq!(selection_end, 7);
        assert_eq!(selection_affinity, YU_CARET_AFFINITY_DOWNSTREAM);

        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_key_route_executes_macos_undo_redo_and_leaves_text_keys_unhandled() {
        let handle = session("a");
        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 1, 1, ptr::null(), 0, 0, 0) },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe { yu_composition_session_commit(handle, b"b".as_ptr(), 1) },
            YU_FFI_OK
        );

        let mut result = YuEditorCommandResult::default();
        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    handle,
                    YU_KEY_CHARACTER,
                    u32::from(b'z'),
                    YU_KEY_MODIFIER_COMMAND,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            (
                result.revision,
                result.selection_start_utf16,
                result.selection_end_utf16,
                result.changed,
                result.source_sync,
            ),
            (2, 1, 1, 1, YU_SOURCE_SYNC_FULL)
        );
        let mut source_length = 0;
        assert_eq!(
            unsafe { yu_composition_session_source_length(handle, &mut source_length) },
            YU_FFI_OK
        );
        let mut source = vec![0_u8; source_length];
        assert_eq!(
            unsafe {
                yu_composition_session_copy_source(handle, source.as_mut_ptr(), source.len())
            },
            YU_FFI_OK
        );
        assert_eq!(source, b"a");

        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    handle,
                    YU_KEY_CHARACTER,
                    u32::from(b'z'),
                    YU_KEY_MODIFIER_COMMAND | YU_KEY_MODIFIER_SHIFT,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            (
                result.revision,
                result.selection_start_utf16,
                result.source_sync
            ),
            (3, 2, YU_SOURCE_SYNC_FULL)
        );
        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    handle,
                    YU_KEY_CHARACTER,
                    u32::from(b'x'),
                    0,
                    &mut result,
                )
            },
            YU_FFI_KEY_UNHANDLED
        );

        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_key_route_returns_local_ranges_and_does_not_consume_plain_tab() {
        let plain = session("paragraph");
        let mut result = YuEditorCommandResult::default();
        assert_eq!(
            unsafe { yu_composition_session_route_key(plain, YU_KEY_TAB, 0, 0, &mut result) },
            YU_FFI_KEY_UNHANDLED
        );
        unsafe { yu_composition_session_destroy(plain) };

        let list = session("- item");
        assert_eq!(
            unsafe { yu_composition_session_route_key(list, YU_KEY_TAB, 0, 0, &mut result) },
            YU_FFI_OK
        );
        assert_eq!(
            (
                result.source_sync,
                result.source_start_utf16,
                result.source_old_end_utf16,
                result.source_new_start_utf16,
                result.source_new_end_utf16,
            ),
            (YU_SOURCE_SYNC_RANGE, 0, 0, 0, 2)
        );
        let mut length = 0;
        assert_eq!(
            unsafe { yu_composition_session_source_length(list, &mut length) },
            YU_FFI_OK
        );
        let mut bytes = vec![0_u8; length];
        assert_eq!(
            unsafe { yu_composition_session_copy_source(list, bytes.as_mut_ptr(), bytes.len()) },
            YU_FFI_OK
        );
        assert_eq!(bytes, b"  - item");

        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    list,
                    YU_KEY_TAB,
                    0,
                    YU_KEY_MODIFIER_SHIFT,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(result.source_sync, YU_SOURCE_SYNC_RANGE);
        unsafe { yu_composition_session_destroy(list) };

        let local = session("ab");
        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(local, 0, 2, 2, YU_CARET_AFFINITY_DOWNSTREAM)
            },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe { yu_composition_session_route_key(local, YU_KEY_BACKSPACE, 0, 0, &mut result) },
            YU_FFI_OK
        );
        assert_eq!(
            (
                result.source_sync,
                result.source_start_utf16,
                result.source_old_end_utf16,
                result.source_new_start_utf16,
                result.source_new_end_utf16,
            ),
            (YU_SOURCE_SYNC_RANGE, 1, 2, 1, 1)
        );
        unsafe { yu_composition_session_destroy(local) };
    }

    #[test]
    fn ffi_command_availability_is_context_bound_and_read_only() {
        let handle = session("");
        let mut available = 255;
        assert_eq!(
            unsafe {
                yu_composition_session_command_available(
                    handle,
                    YU_EDITOR_COMMAND_UNDO,
                    0,
                    &mut available,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(available, YU_COMMAND_UNAVAILABLE);
        assert_eq!(
            unsafe {
                yu_composition_session_command_available(
                    handle,
                    YU_EDITOR_COMMAND_INSERT_NEWLINE,
                    0,
                    &mut available,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(available, YU_COMMAND_AVAILABLE);

        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 0, 0, ptr::null(), 0, 0, 0) },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe {
                yu_composition_session_commit(handle, "羽".as_bytes().as_ptr(), "羽".len())
            },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe {
                yu_composition_session_command_available(
                    handle,
                    YU_EDITOR_COMMAND_UNDO,
                    0,
                    &mut available,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(available, YU_COMMAND_AVAILABLE);
        assert_eq!(
            unsafe {
                yu_composition_session_command_available(
                    handle,
                    YU_EDITOR_COMMAND_MOVE_LEFT,
                    0,
                    &mut available,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(available, YU_COMMAND_AVAILABLE);

        assert_eq!(
            unsafe { yu_composition_session_reset_source(handle, b"- item".as_ptr(), 6) },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe {
                yu_composition_session_command_available(
                    handle,
                    YU_EDITOR_COMMAND_INDENT_LIST,
                    0,
                    &mut available,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(available, YU_COMMAND_AVAILABLE);
        assert_eq!(
            unsafe {
                yu_composition_session_command_available(
                    handle,
                    YU_EDITOR_COMMAND_OUTDENT_LIST,
                    0,
                    &mut available,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(available, YU_COMMAND_UNAVAILABLE);
        assert_eq!(
            unsafe {
                yu_composition_session_command_available(
                    handle,
                    YU_EDITOR_COMMAND_OUTDENT_LIST,
                    0,
                    ptr::null_mut(),
                )
            },
            YU_FFI_NULL_POINTER
        );
        assert_eq!(
            unsafe { yu_composition_session_command_available(handle, 255, 0, &mut available) },
            YU_FFI_INVALID_COMMAND
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_key_route_maps_macos_option_word_movement() {
        let handle = session("hello world");
        let mut result = YuEditorCommandResult::default();
        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    handle,
                    YU_KEY_LEFT,
                    0,
                    YU_KEY_MODIFIER_OPTION,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            (
                result.selection_start_utf16,
                result.selection_end_utf16,
                result.changed
            ),
            (6, 6, 0)
        );
        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    handle,
                    YU_KEY_RIGHT,
                    0,
                    YU_KEY_MODIFIER_OPTION,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            (result.selection_start_utf16, result.selection_end_utf16),
            (11, 11)
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_key_route_moves_vertically_with_layout_preferred_x() {
        let handle = session("abcdefghij\nxy\n1234567890");
        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(
                    handle,
                    0,
                    10,
                    10,
                    YU_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_FFI_OK
        );

        let mut result = YuEditorCommandResult::default();
        assert_eq!(
            unsafe { yu_composition_session_route_key(handle, YU_KEY_DOWN, 0, 0, &mut result) },
            YU_FFI_OK
        );
        assert_eq!(
            (
                result.selection_start_utf16,
                result.selection_end_utf16,
                result.changed,
                result.source_sync,
            ),
            (13, 13, 0, YU_SOURCE_SYNC_NONE)
        );
        assert_eq!(
            unsafe { yu_composition_session_route_key(handle, YU_KEY_DOWN, 0, 0, &mut result) },
            YU_FFI_OK
        );
        assert_eq!(
            (result.selection_start_utf16, result.selection_end_utf16),
            (24, 24)
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_key_route_crosses_adjacent_markdown_blocks() {
        let handle = session("# title\ntext");
        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(handle, 0, 8, 8, YU_CARET_AFFINITY_DOWNSTREAM)
            },
            YU_FFI_OK
        );

        let mut result = YuEditorCommandResult::default();
        assert_eq!(
            unsafe { yu_composition_session_route_key(handle, YU_KEY_UP, 0, 0, &mut result) },
            YU_FFI_OK
        );
        assert_eq!(
            (result.selection_start_utf16, result.selection_end_utf16),
            (0, 0)
        );
        assert_eq!(
            unsafe { yu_composition_session_route_key(handle, YU_KEY_DOWN, 0, 0, &mut result) },
            YU_FFI_OK
        );
        assert_eq!(
            (result.selection_start_utf16, result.selection_end_utf16),
            (8, 8)
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_key_route_shift_vertical_extends_selection() {
        let handle = session("one\ntwo\nthree");
        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(handle, 0, 0, 0, YU_CARET_AFFINITY_DOWNSTREAM)
            },
            YU_FFI_OK
        );

        let mut result = YuEditorCommandResult::default();
        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    handle,
                    YU_KEY_DOWN,
                    0,
                    YU_KEY_MODIFIER_SHIFT,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            (result.selection_start_utf16, result.selection_end_utf16),
            (0, 4)
        );
        assert_eq!(
            unsafe {
                yu_composition_session_route_key(
                    handle,
                    YU_KEY_DOWN,
                    0,
                    YU_KEY_MODIFIER_SHIFT,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            (result.selection_start_utf16, result.selection_end_utf16),
            (0, 8)
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_viewport_config_is_revision_bound_and_controls_metrics_layout() {
        let handle = session("abcdefgh");
        assert_eq!(
            unsafe {
                yu_composition_session_set_viewport_config(handle, 0, 6.0, 2.0, 2.0, 2.0, 0.0)
            },
            YU_FFI_OK
        );

        let mut request = YuEditorCaretScrollRequest::default();
        assert_eq!(
            unsafe {
                yu_composition_session_caret_scroll_request(handle, 0, 0.0, 2.0, 0.0, &mut request)
            },
            YU_FFI_OK
        );
        assert_eq!(request.caret_height, 2.0);
        assert_eq!(request.target_scroll_y, 4.0);
        assert_eq!(request.needs_scroll, 1);

        assert_eq!(
            unsafe {
                yu_composition_session_set_viewport_config(handle, 1, 6.0, 2.0, 2.0, 2.0, 0.0)
            },
            YU_FFI_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_composition_session_set_viewport_config(handle, 0, 0.0, 2.0, 2.0, 2.0, 0.0)
            },
            YU_FFI_INVALID_VIEWPORT_CONFIG
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_caret_scroll_request_returns_revision_bound_absolute_target() {
        let handle = session("one\n\ntwo\n\nthree");
        let mut request = YuEditorCaretScrollRequest::default();
        assert_eq!(
            unsafe {
                yu_composition_session_caret_scroll_request(handle, 0, 0.0, 1.0, 0.0, &mut request)
            },
            YU_FFI_OK
        );
        assert_eq!(request.revision, 0);
        assert_eq!(request.source_utf16, 15);
        assert_eq!(request.block_index, 4);
        assert_eq!(request.caret_y, 4.0);
        assert_eq!(request.target_scroll_y, 4.0);
        assert_eq!(request.needs_scroll, 1);

        let target = request.target_scroll_y;
        assert_eq!(
            unsafe {
                yu_composition_session_caret_scroll_request(
                    handle,
                    request.revision,
                    target,
                    1.0,
                    0.0,
                    &mut request,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(request.needs_scroll, 0);
        assert_eq!(request.target_scroll_y, target);

        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(
                    handle,
                    request.revision,
                    0,
                    0,
                    YU_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe {
                yu_composition_session_caret_scroll_request(
                    handle,
                    0,
                    target,
                    1.0,
                    0.0,
                    &mut request,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(request.target_scroll_y, 0.0);
        assert_eq!(request.needs_scroll, 1);
        assert_eq!(
            unsafe {
                yu_composition_session_caret_scroll_request(handle, 1, 0.0, 1.0, 0.0, &mut request)
            },
            YU_FFI_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_composition_session_caret_scroll_request(handle, 0, 0.0, 1.0, -1.0, &mut request)
            },
            YU_FFI_INVALID_RANGE
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_execute_command_rejects_active_composition_and_unknown_commands() {
        let handle = session("a");
        let mut result = YuEditorCommandResult::default();
        assert_eq!(
            unsafe {
                yu_composition_session_execute_command(
                    handle,
                    YU_EDITOR_COMMAND_UNDO,
                    0,
                    &mut result,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe { yu_composition_session_execute_command(handle, 255, 0, &mut result) },
            YU_FFI_INVALID_COMMAND
        );
        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 1, 1, b"x".as_ptr(), 1, 1, 1) },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe {
                yu_composition_session_execute_command(
                    handle,
                    YU_EDITOR_COMMAND_DELETE_BACKWARD,
                    0,
                    &mut result,
                )
            },
            YU_FFI_EDIT_FAILED
        );
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_set_selection_is_revision_bound_and_rejects_surrogate_splits() {
        let handle = session("a😊羽");
        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(handle, 0, 1, 3, YU_CARET_AFFINITY_UPSTREAM)
            },
            YU_FFI_OK
        );

        let mut revision = 0;
        let mut start = 0;
        let mut end = 0;
        let mut affinity = YU_CARET_AFFINITY_DOWNSTREAM;
        assert_eq!(
            unsafe {
                yu_composition_session_selection(
                    handle,
                    &mut revision,
                    &mut start,
                    &mut end,
                    &mut affinity,
                )
            },
            YU_FFI_OK
        );
        assert_eq!((revision, start, end), (0, 1, 3));
        assert_eq!(affinity, YU_CARET_AFFINITY_UPSTREAM);

        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(handle, 1, 0, 0, YU_CARET_AFFINITY_DOWNSTREAM)
            },
            YU_FFI_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_composition_session_set_selection(handle, 0, 2, 2, YU_CARET_AFFINITY_DOWNSTREAM)
            },
            YU_FFI_INVALID_SELECTION
        );
        assert_eq!(
            unsafe { yu_composition_session_set_selection(handle, 0, 1, 3, 99,) },
            YU_FFI_INVALID_SELECTION
        );
        assert_eq!(
            unsafe {
                yu_composition_session_selection(
                    handle,
                    &mut revision,
                    &mut start,
                    &mut end,
                    &mut affinity,
                )
            },
            YU_FFI_OK
        );
        assert_eq!((revision, start, end), (0, 1, 3));
        assert_eq!(affinity, YU_CARET_AFFINITY_UPSTREAM);

        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_cancel_does_not_advance_revision() {
        let handle = session("hello");
        let mut revision = 99;
        assert_eq!(
            unsafe { yu_composition_session_revision(handle, &mut revision) },
            YU_FFI_OK
        );
        assert_eq!(revision, 0);
        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 5, 5, "e\u{301}".as_ptr(), 3, 2, 2) },
            YU_FFI_OK
        );
        assert_eq!(unsafe { yu_composition_session_cancel(handle) }, YU_FFI_OK);
        assert_eq!(
            unsafe { yu_composition_session_revision(handle, &mut revision) },
            YU_FFI_OK
        );
        assert_eq!(revision, 0);
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_rejects_invalid_ranges_and_reports_buffer_errors() {
        let handle = session("😀");
        let mut source_length = 0;

        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 1, 1, b"x".as_ptr(), 1, 1, 1) },
            YU_FFI_INVALID_RANGE
        );
        assert_eq!(
            unsafe {
                yu_composition_session_begin(handle, 2, 2, "😀".as_ptr(), "😀".len(), 1, 1)
            },
            YU_FFI_INVALID_SELECTION
        );
        assert_eq!(
            unsafe { yu_composition_session_update(handle, b"x".as_ptr(), 1, 1, 1) },
            YU_FFI_NO_OVERLAY
        );

        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 2, 2, b"x".as_ptr(), 1, 1, 1) },
            YU_FFI_OK
        );
        let mut byte = 0;
        assert_eq!(
            unsafe { yu_composition_session_copy_overlay(handle, &mut byte, 0) },
            YU_FFI_BUFFER_TOO_SMALL
        );
        assert_eq!(unsafe { yu_composition_session_cancel(handle) }, YU_FFI_OK);
        assert_eq!(
            unsafe { yu_composition_session_copy_overlay(handle, &mut byte, 1) },
            YU_FFI_NO_OVERLAY
        );
        assert_eq!(
            unsafe { yu_composition_session_source_length(handle, &mut source_length) },
            YU_FFI_OK
        );
        assert_eq!(source_length, "😀".len());

        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_new_clears_output_when_source_is_invalid() {
        let invalid = [0xff_u8];
        let mut output = std::ptr::NonNull::<YuCompositionSession>::dangling().as_ptr();
        assert_eq!(
            unsafe { yu_composition_session_new(invalid.as_ptr(), invalid.len(), &mut output) },
            YU_FFI_INVALID_UTF8
        );
        assert!(output.is_null());
    }

    #[test]
    fn ffi_local_source_query_requires_revision_and_preserves_utf8_boundaries() {
        let handle = session("输入: 😀\nend");
        let mut length = 0;
        assert_eq!(
            unsafe { yu_composition_session_source_range_length(handle, 0, 0, 6, &mut length) },
            YU_FFI_OK
        );
        assert_eq!(length, "输入: 😀".len());

        let mut bytes = vec![0_u8; length];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_composition_session_copy_source_range(
                    handle,
                    0,
                    0,
                    6,
                    bytes.as_mut_ptr(),
                    bytes.len(),
                    &mut written,
                )
            },
            YU_FFI_OK
        );
        assert_eq!(written, length);
        assert_eq!(
            std::str::from_utf8(&bytes).expect("range should stay UTF-8"),
            "输入: 😀"
        );

        assert_eq!(
            unsafe { yu_composition_session_source_range_length(handle, 1, 0, 6, &mut length) },
            YU_FFI_STALE_REVISION
        );
        assert_eq!(
            unsafe { yu_composition_session_source_range_length(handle, 0, 5, 5, &mut length) },
            YU_FFI_INVALID_RANGE
        );

        let mut small = [0_u8; 1];
        assert_eq!(
            unsafe {
                yu_composition_session_copy_source_range(
                    handle,
                    0,
                    0,
                    6,
                    small.as_mut_ptr(),
                    small.len(),
                    &mut written,
                )
            },
            YU_FFI_BUFFER_TOO_SMALL
        );
        assert_eq!(written, "输入: 😀".len());

        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 6, 6, b"x".as_ptr(), 1, 1, 1) },
            YU_FFI_OK
        );
        assert_eq!(
            unsafe { yu_composition_session_reset_source(handle, b"new".as_ptr(), 3) },
            YU_FFI_EDIT_FAILED
        );
        assert_eq!(unsafe { yu_composition_session_cancel(handle) }, YU_FFI_OK);
        unsafe { yu_composition_session_destroy(handle) };
    }

    #[test]
    fn ffi_commit_exposes_stale_revision_and_keeps_overlay() {
        let handle = session("hello");
        assert_eq!(
            unsafe { yu_composition_session_begin(handle, 5, 5, b"yu".as_ptr(), 2, 2, 2) },
            YU_FFI_OK
        );

        let session = unsafe { &mut *handle };
        let transaction = yu_text::Transaction::new(
            session.document.revision(),
            [yu_text::Edit::new(
                yu_core::TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::ZERO)
                    .expect("empty edit range should be valid"),
                "!",
            )],
        );
        session
            .document
            .apply_transaction(&transaction)
            .expect("unrelated edit should advance the document");

        assert_eq!(
            unsafe { yu_composition_session_commit(handle, "羽".as_ptr(), "羽".len()) },
            YU_FFI_STALE_REVISION
        );
        let mut overlay_length = 0;
        assert_eq!(
            unsafe { yu_composition_session_overlay_length(handle, &mut overlay_length) },
            YU_FFI_OK
        );
        assert_eq!(overlay_length, 2);
        assert_eq!(unsafe { yu_composition_session_cancel(handle) }, YU_FFI_OK);
        unsafe { yu_composition_session_destroy(handle) };
    }
}
