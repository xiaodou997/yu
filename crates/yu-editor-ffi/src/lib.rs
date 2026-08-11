#![allow(unsafe_code)]

use std::ptr;

use yu_core::{TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    CaretAffinity, CommandResult, EditorCommand, EditorDocument, EditorDocumentError, EditorKey,
    EditorSelection, KeyEvent, KeyModifiers, KeyRouteResult, SelectionError, SourceSync,
};
use yu_text::{EditError, TextSnapshot};

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

    fn session(source: &str) -> *mut YuCompositionSession {
        let mut output = ptr::null_mut();
        let status =
            unsafe { yu_composition_session_new(source.as_ptr(), source.len(), &mut output) };
        assert_eq!(status, YU_FFI_OK);
        output
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
