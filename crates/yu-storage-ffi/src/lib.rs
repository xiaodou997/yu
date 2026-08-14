#![allow(unsafe_code)]

//! Narrow C ABI for the macOS document-shell spike.
//!
//! `YuStorageSession` owns the only mutable `DocumentEditorSession`, which in
//! turn owns one `DocumentSession` and one `EditorDocument`. Native code may
//! request owned UTF-8 snapshots and revision-bound state, and can route
//! editor commands/IME composition through the same handle without creating a
//! second source. The AppKit host consumes only owned snapshots and explicit
//! result structs; its TextKit mirror is disposable and never canonical.

use std::path::PathBuf;
use std::ptr;

use yu_core::{LineIndex, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    AccessibilityTextError, AccessibilityTextSnapshot, CaretAffinity, CommandResult, EditorCommand,
    EditorDocumentError, EditorKey, KeyEvent, KeyModifiers, KeyRouteResult, SelectionError,
    SourceSync,
};
use yu_storage::{
    ClosePrompt, CloseRequest, CloseState, DiskState, DocumentEditorSession, ExternalFileState,
    SaveOutcome, StorageError, Utf8Bom,
};
use yu_text::{EditError, TextSnapshot};

pub const YU_STORAGE_OK: i32 = 0;
pub const YU_STORAGE_NULL_POINTER: i32 = 1;
pub const YU_STORAGE_INVALID_UTF8: i32 = 2;
pub const YU_STORAGE_IO_ERROR: i32 = 3;
pub const YU_STORAGE_EXTERNAL_CHANGE: i32 = 4;
pub const YU_STORAGE_UNSAVED_CHANGES: i32 = 5;
pub const YU_STORAGE_INVALID_PATH: i32 = 6;
pub const YU_STORAGE_EDITOR_ERROR: i32 = 7;
pub const YU_STORAGE_BUFFER_TOO_SMALL: i32 = 8;
pub const YU_STORAGE_INVALID_STATE: i32 = 9;
pub const YU_STORAGE_KEY_UNHANDLED: i32 = 10;
pub const YU_STORAGE_INVALID_COMMAND: i32 = 11;
pub const YU_STORAGE_INVALID_KEY: i32 = 12;
pub const YU_STORAGE_STALE_REVISION: i32 = 13;
pub const YU_STORAGE_INVALID_SELECTION: i32 = 14;
pub const YU_STORAGE_NO_OVERLAY: i32 = 15;
pub const YU_STORAGE_STALE_COMPOSITION: i32 = 16;

pub const YU_STORAGE_KEY_CHARACTER: u8 = 0;
pub const YU_STORAGE_KEY_ENTER: u8 = 1;
pub const YU_STORAGE_KEY_TAB: u8 = 2;
pub const YU_STORAGE_KEY_BACKSPACE: u8 = 3;
pub const YU_STORAGE_KEY_DELETE: u8 = 4;
pub const YU_STORAGE_KEY_LEFT: u8 = 5;
pub const YU_STORAGE_KEY_RIGHT: u8 = 6;
pub const YU_STORAGE_KEY_UP: u8 = 7;
pub const YU_STORAGE_KEY_DOWN: u8 = 8;
pub const YU_STORAGE_KEY_ESCAPE: u8 = 9;
pub const YU_STORAGE_KEY_MODIFIER_COMMAND: u8 = 1 << 0;
pub const YU_STORAGE_KEY_MODIFIER_SHIFT: u8 = 1 << 1;
pub const YU_STORAGE_KEY_MODIFIER_CONTROL: u8 = 1 << 2;
pub const YU_STORAGE_KEY_MODIFIER_OPTION: u8 = 1 << 3;
pub const YU_STORAGE_COMMAND_DELETE_BACKWARD: u8 = 1;
pub const YU_STORAGE_COMMAND_DELETE_FORWARD: u8 = 2;
pub const YU_STORAGE_COMMAND_MOVE_LEFT: u8 = 3;
pub const YU_STORAGE_COMMAND_MOVE_RIGHT: u8 = 4;
pub const YU_STORAGE_COMMAND_INSERT_NEWLINE: u8 = 5;
pub const YU_STORAGE_COMMAND_INDENT_LIST: u8 = 6;
pub const YU_STORAGE_COMMAND_OUTDENT_LIST: u8 = 7;
pub const YU_STORAGE_COMMAND_UNDO: u8 = 8;
pub const YU_STORAGE_COMMAND_REDO: u8 = 9;
pub const YU_STORAGE_COMMAND_TOGGLE_TASK: u8 = 10;
pub const YU_STORAGE_COMMAND_MOVE_WORD_LEFT: u8 = 11;
pub const YU_STORAGE_COMMAND_MOVE_WORD_RIGHT: u8 = 12;
pub const YU_STORAGE_COMMAND_MOVE_UP: u8 = 13;
pub const YU_STORAGE_COMMAND_MOVE_DOWN: u8 = 14;
pub const YU_STORAGE_COMMAND_MOVE_UP_EXTEND: u8 = 15;
pub const YU_STORAGE_COMMAND_MOVE_DOWN_EXTEND: u8 = 16;
pub const YU_STORAGE_SOURCE_SYNC_NONE: u8 = 0;
pub const YU_STORAGE_SOURCE_SYNC_RANGE: u8 = 1;
pub const YU_STORAGE_SOURCE_SYNC_FULL: u8 = 2;
pub const YU_STORAGE_CARET_AFFINITY_UPSTREAM: u8 = 0;
pub const YU_STORAGE_CARET_AFFINITY_DOWNSTREAM: u8 = 1;

pub const YU_STORAGE_DISK_UNCHANGED: u8 = 0;
pub const YU_STORAGE_DISK_CHANGED: u8 = 1;
pub const YU_STORAGE_DISK_MISSING: u8 = 2;
pub const YU_STORAGE_BOM_ABSENT: u8 = 0;
pub const YU_STORAGE_BOM_PRESENT: u8 = 1;
pub const YU_STORAGE_CLOSE_OPEN: u8 = 0;
pub const YU_STORAGE_CLOSE_CLOSED: u8 = 1;
pub const YU_STORAGE_CLOSE_PROMPT_SAVE: u8 = 2;
pub const YU_STORAGE_CLOSE_PROMPT_EXTERNAL_CHANGED: u8 = 3;
pub const YU_STORAGE_CLOSE_PROMPT_EXTERNAL_MISSING: u8 = 4;
pub const YU_STORAGE_CLOSE_NOW: u8 = 0;
pub const YU_STORAGE_CLOSE_PROMPT: u8 = 1;
pub const YU_STORAGE_CLOSE_ALREADY_CLOSED: u8 = 2;
pub const YU_STORAGE_EXTERNAL_CHANGED: u8 = YU_STORAGE_DISK_CHANGED;
pub const YU_STORAGE_EXTERNAL_MISSING: u8 = YU_STORAGE_DISK_MISSING;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageState {
    pub revision: u64,
    pub saved_revision: u64,
    pub dirty: u8,
    pub disk_state: u8,
    pub bom: u8,
    pub close_state: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCloseRequest {
    pub result: u8,
    pub close_state: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageSelection {
    pub revision: u64,
    pub start_utf16: u64,
    pub end_utf16: u64,
    pub affinity: u8,
}

/// Revision-bound source coordinates used by the native Accessibility adapter.
/// The snapshot is intentionally compact: text and line contents are queried
/// separately through the existing expected-revision source-range API.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageAccessibilitySnapshot {
    pub revision: u64,
    pub number_of_characters_utf16: u64,
    pub selection_start_utf16: u64,
    pub selection_end_utf16: u64,
    pub line_count: u64,
    pub selection_affinity: u8,
}

/// A logical line range bound to one Accessibility snapshot revision.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageAccessibilityRange {
    pub revision: u64,
    pub start_utf16: u64,
    pub end_utf16: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCommandResult {
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCompositionState {
    pub revision: u64,
    pub generation: u64,
    pub replacement_start_utf16: u64,
    pub replacement_end_utf16: u64,
    pub selection_start_utf16: u64,
    pub selection_end_utf16: u64,
    pub preedit_utf8_length: u64,
    pub active: u8,
}

#[repr(C)]
pub struct YuStorageSession {
    session: DocumentEditorSession,
}

fn read_utf8<'a>(pointer: *const u8, length: usize) -> Result<&'a str, i32> {
    if length == 0 {
        return Ok("");
    }
    if pointer.is_null() {
        return Err(YU_STORAGE_NULL_POINTER);
    }
    // SAFETY: the native caller supplies a readable pointer/length pair that
    // remains valid for this synchronous call.
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    std::str::from_utf8(bytes).map_err(|_| YU_STORAGE_INVALID_UTF8)
}

fn write_bytes(bytes: &[u8], output: *mut u8, capacity: usize, written: *mut usize) -> i32 {
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` is a caller-owned output pointer checked above.
    unsafe { *written = bytes.len() };
    if bytes.is_empty() {
        return YU_STORAGE_OK;
    }
    // A null output with zero capacity is the ABI's length-query form. This
    // lets native callers size an owned snapshot without requiring a dummy
    // allocation or exposing Rust storage across the boundary.
    if output.is_null() {
        return if capacity == 0 {
            YU_STORAGE_OK
        } else {
            YU_STORAGE_NULL_POINTER
        };
    }
    if capacity < bytes.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    // SAFETY: capacity was checked against the source length.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
    YU_STORAGE_OK
}

fn status_from_error(error: StorageError) -> i32 {
    match error {
        StorageError::Io { .. } => YU_STORAGE_IO_ERROR,
        StorageError::InvalidUtf8 { .. } => YU_STORAGE_INVALID_UTF8,
        StorageError::InvalidPath(_) => YU_STORAGE_INVALID_PATH,
        StorageError::ExternalChange { .. } => YU_STORAGE_EXTERNAL_CHANGE,
        StorageError::UnsavedChanges { .. } => YU_STORAGE_UNSAVED_CHANGES,
        StorageError::CloseState(_) => YU_STORAGE_INVALID_STATE,
        StorageError::Editor(_) => YU_STORAGE_EDITOR_ERROR,
    }
}

fn disk_state(session: &DocumentEditorSession) -> Result<u8, StorageError> {
    Ok(match session.disk_state()? {
        DiskState::Unchanged => YU_STORAGE_DISK_UNCHANGED,
        DiskState::Changed => YU_STORAGE_DISK_CHANGED,
        DiskState::Missing => YU_STORAGE_DISK_MISSING,
    })
}

fn close_state(session: CloseState) -> u8 {
    match session {
        CloseState::Open => YU_STORAGE_CLOSE_OPEN,
        CloseState::Closed => YU_STORAGE_CLOSE_CLOSED,
        CloseState::Prompting(ClosePrompt::SaveChanges) => YU_STORAGE_CLOSE_PROMPT_SAVE,
        CloseState::Prompting(ClosePrompt::ExternalChange {
            state: ExternalFileState::Changed,
        }) => YU_STORAGE_CLOSE_PROMPT_EXTERNAL_CHANGED,
        CloseState::Prompting(ClosePrompt::ExternalChange {
            state: ExternalFileState::Missing,
        }) => YU_STORAGE_CLOSE_PROMPT_EXTERNAL_MISSING,
    }
}

fn external_file_state_from_ffi(value: u8) -> Result<ExternalFileState, i32> {
    match value {
        YU_STORAGE_DISK_CHANGED => Ok(ExternalFileState::Changed),
        YU_STORAGE_DISK_MISSING => Ok(ExternalFileState::Missing),
        _ => Err(YU_STORAGE_INVALID_STATE),
    }
}

fn status_from_editor_error(error: EditorDocumentError) -> i32 {
    match error {
        EditorDocumentError::Edit(EditError::StaleRevision { .. })
        | EditorDocumentError::Selection(SelectionError::StaleRevision { .. }) => {
            YU_STORAGE_STALE_REVISION
        }
        EditorDocumentError::CompositionNotActive => YU_STORAGE_NO_OVERLAY,
        EditorDocumentError::Selection(_) | EditorDocumentError::Composition(_) => {
            YU_STORAGE_INVALID_SELECTION
        }
        _ => YU_STORAGE_EDITOR_ERROR,
    }
}

fn status_from_accessibility_error(error: AccessibilityTextError) -> i32 {
    match error {
        AccessibilityTextError::StaleRevision { .. } => YU_STORAGE_STALE_REVISION,
        AccessibilityTextError::InvalidSourceRange(_)
        | AccessibilityTextError::InvalidUtf16Range(_)
        | AccessibilityTextError::Position(_)
        | AccessibilityTextError::OffsetOverflow => YU_STORAGE_INVALID_SELECTION,
    }
}

fn storage_status(error: StorageError) -> i32 {
    match error {
        StorageError::Editor(error) => status_from_editor_error(error),
        other => status_from_error(other),
    }
}

fn validate_composition(
    session: &DocumentEditorSession,
    expected_revision: u64,
    expected_generation: u64,
) -> Result<(), i32> {
    validate_revision(session, expected_revision)?;
    if session.composition_generation() != expected_generation {
        return Err(YU_STORAGE_STALE_COMPOSITION);
    }
    if session.composition().is_none() {
        return Err(YU_STORAGE_NO_OVERLAY);
    }
    Ok(())
}

fn caret_affinity_from_ffi(value: u8) -> Result<CaretAffinity, i32> {
    match value {
        YU_STORAGE_CARET_AFFINITY_UPSTREAM => Ok(CaretAffinity::Upstream),
        YU_STORAGE_CARET_AFFINITY_DOWNSTREAM => Ok(CaretAffinity::Downstream),
        _ => Err(YU_STORAGE_INVALID_SELECTION),
    }
}

fn selection_from_ffi(
    session: &DocumentEditorSession,
    start_utf16: u64,
    end_utf16: u64,
    affinity: u8,
) -> Result<yu_editor::EditorSelection, i32> {
    let affinity = caret_affinity_from_ffi(affinity)?;
    let range = Utf16Range::new(Utf16Offset::new(start_utf16), Utf16Offset::new(end_utf16))
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    let snapshot = session.snapshot();
    let start = snapshot
        .byte_offset_for_utf16(range.start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let end = snapshot
        .byte_offset_for_utf16(range.end())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    yu_editor::EditorSelection::range(&snapshot, start, end, affinity)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)
}

fn source_range_from_ffi(
    session: &DocumentEditorSession,
    start_utf16: u64,
    end_utf16: u64,
) -> Result<TextRange, i32> {
    let range = Utf16Range::new(Utf16Offset::new(start_utf16), Utf16Offset::new(end_utf16))
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    let snapshot = session.snapshot();
    let start = snapshot
        .byte_offset_for_utf16(range.start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let end = snapshot
        .byte_offset_for_utf16(range.end())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    TextRange::new(start, end).ok_or(YU_STORAGE_INVALID_SELECTION)
}

fn selection_output(session: &DocumentEditorSession, output: *mut YuStorageSelection) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let snapshot = session.snapshot();
    let selection = session.selection();
    let range = match selection.utf16_range(&snapshot) {
        Ok(range) => range,
        Err(error) => return status_from_editor_error(EditorDocumentError::Selection(error)),
    };
    // SAFETY: output is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageSelection {
            revision: session.revision().get(),
            start_utf16: range.start().get(),
            end_utf16: range.end().get(),
            affinity: match selection.affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        };
    }
    YU_STORAGE_OK
}

fn accessibility_snapshot(
    session: &DocumentEditorSession,
) -> Result<AccessibilityTextSnapshot, i32> {
    let source = session.snapshot();
    let selection = session.selection();
    AccessibilityTextSnapshot::from_selection(source, selection)
        .map_err(status_from_accessibility_error)
}

fn accessibility_snapshot_output(
    session: &DocumentEditorSession,
    output: *mut YuStorageAccessibilitySnapshot,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let source = session.snapshot();
    let line_count = source.summary().line_count();
    let snapshot = match AccessibilityTextSnapshot::from_selection(source, session.selection()) {
        Ok(snapshot) => snapshot,
        Err(error) => return status_from_accessibility_error(error),
    };
    let selected = snapshot.selected_range().range();
    // SAFETY: `output` is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageAccessibilitySnapshot {
            revision: snapshot.revision().get(),
            number_of_characters_utf16: snapshot.number_of_characters().get(),
            selection_start_utf16: selected.start().get(),
            selection_end_utf16: selected.end().get(),
            line_count,
            selection_affinity: match session.selection().affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        };
    }
    YU_STORAGE_OK
}

fn accessibility_line_range_output(
    session: &DocumentEditorSession,
    expected_revision: u64,
    line: u64,
    output: *mut YuStorageAccessibilityRange,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let snapshot = match accessibility_snapshot(session) {
        Ok(snapshot) => snapshot,
        Err(status) => return status,
    };
    let range = match snapshot.range_for_line(LineIndex::new(line)) {
        Ok(range) => range.range(),
        Err(error) => return status_from_accessibility_error(error),
    };
    // SAFETY: `output` is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageAccessibilityRange {
            revision: snapshot.revision().get(),
            start_utf16: range.start().get(),
            end_utf16: range.end().get(),
        };
    }
    YU_STORAGE_OK
}

fn accessibility_line_for_position_output(
    session: &DocumentEditorSession,
    expected_revision: u64,
    offset_utf16: u64,
    output: *mut u64,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let snapshot = match accessibility_snapshot(session) {
        Ok(snapshot) => snapshot,
        Err(status) => return status,
    };
    let position = match snapshot.bind_position(Utf16Offset::new(offset_utf16)) {
        Ok(position) => position,
        Err(error) => return status_from_accessibility_error(error),
    };
    let line = match snapshot.line_for_position(position) {
        Ok(line) => line,
        Err(error) => return status_from_accessibility_error(error),
    };
    // SAFETY: `output` is checked above and belongs to the caller.
    unsafe { *output = line.get() };
    YU_STORAGE_OK
}

fn command_from_ffi(command: u8, block: u64) -> Result<EditorCommand, i32> {
    match command {
        YU_STORAGE_COMMAND_DELETE_BACKWARD => Ok(EditorCommand::DeleteBackward),
        YU_STORAGE_COMMAND_DELETE_FORWARD => Ok(EditorCommand::DeleteForward),
        YU_STORAGE_COMMAND_MOVE_LEFT => Ok(EditorCommand::MoveLeft),
        YU_STORAGE_COMMAND_MOVE_RIGHT => Ok(EditorCommand::MoveRight),
        YU_STORAGE_COMMAND_INSERT_NEWLINE => Ok(EditorCommand::insert_newline()),
        YU_STORAGE_COMMAND_INDENT_LIST => Ok(EditorCommand::indent_list()),
        YU_STORAGE_COMMAND_OUTDENT_LIST => Ok(EditorCommand::outdent_list()),
        YU_STORAGE_COMMAND_UNDO => Ok(EditorCommand::undo()),
        YU_STORAGE_COMMAND_REDO => Ok(EditorCommand::redo()),
        YU_STORAGE_COMMAND_TOGGLE_TASK => usize::try_from(block)
            .map(EditorCommand::toggle_task)
            .map_err(|_| YU_STORAGE_INVALID_SELECTION),
        YU_STORAGE_COMMAND_MOVE_WORD_LEFT => Ok(EditorCommand::move_word_left()),
        YU_STORAGE_COMMAND_MOVE_WORD_RIGHT => Ok(EditorCommand::move_word_right()),
        YU_STORAGE_COMMAND_MOVE_UP => Ok(EditorCommand::move_up()),
        YU_STORAGE_COMMAND_MOVE_DOWN => Ok(EditorCommand::move_down()),
        YU_STORAGE_COMMAND_MOVE_UP_EXTEND => Ok(EditorCommand::move_up_extend()),
        YU_STORAGE_COMMAND_MOVE_DOWN_EXTEND => Ok(EditorCommand::move_down_extend()),
        _ => Err(YU_STORAGE_INVALID_COMMAND),
    }
}

fn key_from_ffi(kind: u8, value: u32) -> Result<EditorKey, i32> {
    match kind {
        YU_STORAGE_KEY_CHARACTER => char::from_u32(value)
            .map(EditorKey::Character)
            .ok_or(YU_STORAGE_INVALID_KEY),
        YU_STORAGE_KEY_ENTER => Ok(EditorKey::Enter),
        YU_STORAGE_KEY_TAB => Ok(EditorKey::Tab),
        YU_STORAGE_KEY_BACKSPACE => Ok(EditorKey::Backspace),
        YU_STORAGE_KEY_DELETE => Ok(EditorKey::Delete),
        YU_STORAGE_KEY_LEFT => Ok(EditorKey::Left),
        YU_STORAGE_KEY_RIGHT => Ok(EditorKey::Right),
        YU_STORAGE_KEY_UP => Ok(EditorKey::Up),
        YU_STORAGE_KEY_DOWN => Ok(EditorKey::Down),
        YU_STORAGE_KEY_ESCAPE => Ok(EditorKey::Escape),
        _ => Err(YU_STORAGE_INVALID_KEY),
    }
}

fn command_result_output(
    session: &DocumentEditorSession,
    result: CommandResult,
    output: *mut YuStorageCommandResult,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let snapshot = session.snapshot();
    let selection = match result.selection().utf16_range(&snapshot) {
        Ok(selection) => selection,
        Err(error) => return status_from_editor_error(EditorDocumentError::Selection(error)),
    };
    let (source_sync, old_start, old_end, new_start, new_end) = match result.source_sync() {
        SourceSync::None => (YU_STORAGE_SOURCE_SYNC_NONE, 0, 0, 0, 0),
        SourceSync::Full if result.changed() => (YU_STORAGE_SOURCE_SYNC_FULL, 0, 0, 0, 0),
        SourceSync::Full => (YU_STORAGE_SOURCE_SYNC_NONE, 0, 0, 0, 0),
        SourceSync::Range(change) => (
            YU_STORAGE_SOURCE_SYNC_RANGE,
            change.old_range().start().get(),
            change.old_range().end().get(),
            change.new_range().start().get(),
            change.new_range().end().get(),
        ),
    };
    // SAFETY: output is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageCommandResult {
            revision: result.revision().get(),
            selection_start_utf16: selection.start().get(),
            selection_end_utf16: selection.end().get(),
            affinity: match result.selection().affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
            changed: u8::from(result.changed()),
            source_sync,
            source_start_utf16: old_start,
            source_old_end_utf16: old_end,
            source_new_start_utf16: new_start,
            source_new_end_utf16: new_end,
        };
    }
    YU_STORAGE_OK
}

fn write_snapshot_range(
    snapshot: &TextSnapshot,
    range: TextRange,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let start = match usize::try_from(range.start()) {
        Ok(start) => start,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let end = match usize::try_from(range.end()) {
        Ok(end) => end,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let required = end.saturating_sub(start);
    // SAFETY: `written` was checked above and belongs to the caller.
    unsafe { *written = required };
    if required == 0 {
        return YU_STORAGE_OK;
    }
    if output.is_null() {
        return if capacity == 0 {
            YU_STORAGE_OK
        } else {
            YU_STORAGE_NULL_POINTER
        };
    }
    if capacity < required {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    let mut copied = 0_usize;
    let mut chunks = match snapshot.chunk_cursor(range.start()) {
        Ok(chunks) => chunks,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    for chunk in &mut chunks {
        let chunk_start = match usize::try_from(chunk.start()) {
            Ok(start) => start,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let chunk_end = match chunk_start.checked_add(chunk.text().len()) {
            Some(end) => end,
            None => return YU_STORAGE_INVALID_SELECTION,
        };
        if chunk_start >= end {
            break;
        }
        let local_start = start.saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            let bytes = &chunk.text().as_bytes()[local_start..local_end];
            // SAFETY: capacity was checked against the requested byte range.
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), output.add(copied), bytes.len()) };
            copied += bytes.len();
        }
    }
    if copied == required {
        YU_STORAGE_OK
    } else {
        YU_STORAGE_INVALID_SELECTION
    }
}

fn validate_revision(session: &DocumentEditorSession, expected: u64) -> Result<(), i32> {
    if session.revision().get() != expected {
        return Err(YU_STORAGE_STALE_REVISION);
    }
    Ok(())
}

/// # Safety
///
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_selection(
    session: *const YuStorageSession,
    output: *mut YuStorageSelection,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    selection_output(&session.session, output)
}

/// # Safety
///
/// `session` must be a live handle. The expected revision and UTF-16 range
/// must describe a valid source selection; `affinity` must be a known value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_selection(
    session: *mut YuStorageSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    affinity: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let selection = match selection_from_ffi(&session.session, start_utf16, end_utf16, affinity) {
        Ok(selection) => selection,
        Err(status) => return status,
    };
    session
        .session
        .set_selection(selection)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_execute_command(
    session: *mut YuStorageSession,
    command: u8,
    block: u64,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let command = match command_from_ffi(command, block) {
        Ok(command) => command,
        Err(status) => return status,
    };
    let result = match session.session.execute(command) {
        Ok(result) => result,
        Err(error) => return storage_status(error),
    };
    command_result_output(&session.session, result, output)
}

/// Inserts permanent text through the same editor command path used by all
/// other native edits. `expected_revision` prevents an NSTextInputClient
/// callback queued for an older source mirror from applying late.
///
/// # Safety
/// `session` must be a live handle. `text` must point to a readable UTF-8
/// buffer of `text_length` bytes, and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_insert_text(
    session: *mut YuStorageSession,
    expected_revision: u64,
    text: *const u8,
    text_length: usize,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let text = match read_utf8(text, text_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let result = match session.session.execute(EditorCommand::insert_text(text)) {
        Ok(result) => result,
        Err(error) => return storage_status(error),
    };
    command_result_output(&session.session, result, output)
}

/// # Safety
///
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_command_available(
    session: *const YuStorageSession,
    command: u8,
    block: u64,
    output: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let command = match command_from_ffi(command, block) {
        Ok(command) => command,
        Err(status) => return status,
    };
    // SAFETY: output was checked above and belongs to the caller.
    unsafe {
        *output = u8::from(session.session.command_available(&command));
    }
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be a live handle and `output` must be writable when a key
/// is handled. Printable text is intentionally left to native text input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_route_key(
    session: *mut YuStorageSession,
    key_kind: u8,
    key: u32,
    modifiers: u8,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let key = match key_from_ffi(key_kind, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    let event = KeyEvent::new(key, KeyModifiers::from_bits(modifiers));
    let route = match session.session.route_key(event) {
        Ok(route) => route,
        Err(error) => return storage_status(error),
    };
    let KeyRouteResult::Executed(result) = route else {
        return YU_STORAGE_KEY_UNHANDLED;
    };
    command_result_output(&session.session, result, output)
}

/// Returns the current transient composition metadata. The source revision
/// remains unchanged while `active` is non-zero; `generation` changes on each
/// successful begin/update/commit/cancel transition.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_composition(
    session: *const YuStorageSession,
    output: *mut YuStorageCompositionState,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let mut state = YuStorageCompositionState {
        revision: session.session.revision().get(),
        generation: session.session.composition_generation(),
        ..YuStorageCompositionState::default()
    };
    if let Some(overlay) = session.session.composition() {
        let snapshot = session.session.snapshot();
        let replacement = overlay.replacement_range();
        let start = match snapshot.utf16_offset(replacement.start()) {
            Ok(value) => value,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let end = match snapshot.utf16_offset(replacement.end()) {
            Ok(value) => value,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let selection = overlay.selection_utf16();
        state.replacement_start_utf16 = start.get();
        state.replacement_end_utf16 = end.get();
        state.selection_start_utf16 = selection.start().get();
        state.selection_end_utf16 = selection.end().get();
        state.preedit_utf8_length = overlay.text().len() as u64;
        state.active = 1;
    }
    // SAFETY: output was checked above and belongs to the caller.
    unsafe { *output = state };
    YU_STORAGE_OK
}

/// Copies the active preedit after validating both source revision and
/// composition generation. It uses the same two-call length/copy convention
/// as the source snapshot functions.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_composition(
    session: *const YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let Some(overlay) = session.session.composition() else {
        return YU_STORAGE_NO_OVERLAY;
    };
    write_bytes(overlay.text().as_bytes(), output, capacity, written)
}

/// # Safety
///
/// `session` must be a live handle. `preedit` must point to readable UTF-8
/// bytes for the given length unless the length is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_begin_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    replacement_start_utf16: u64,
    replacement_end_utf16: u64,
    preedit: *const u8,
    preedit_length: usize,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let preedit = match read_utf8(preedit, preedit_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let replacement = match source_range_from_ffi(
        &session.session,
        replacement_start_utf16,
        replacement_end_utf16,
    ) {
        Ok(range) => range,
        Err(status) => return status,
    };
    let selection = match Utf16Range::new(
        Utf16Offset::new(selection_start_utf16),
        Utf16Offset::new(selection_end_utf16),
    ) {
        Some(selection) => selection,
        None => return YU_STORAGE_INVALID_SELECTION,
    };
    session
        .session
        .begin_composition(replacement, preedit, selection)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle. `preedit` must point to readable UTF-8
/// bytes for the given length unless the length is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_update_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    preedit: *const u8,
    preedit_length: usize,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let preedit = match read_utf8(preedit, preedit_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let selection = match Utf16Range::new(
        Utf16Offset::new(selection_start_utf16),
        Utf16Offset::new(selection_end_utf16),
    ) {
        Some(selection) => selection,
        None => return YU_STORAGE_INVALID_SELECTION,
    };
    session
        .session
        .update_composition(preedit, selection)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle. `committed_text` must point to readable
/// UTF-8 bytes for the given length unless the length is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_commit_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    committed_text: *const u8,
    committed_length: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let committed_text = match read_utf8(committed_text, committed_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    session
        .session
        .commit_composition(committed_text)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_cancel_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let _ = session.session.cancel_composition();
    YU_STORAGE_OK
}

/// # Safety
///
/// `output` must be writable for one session pointer. When `path_length` is
/// non-zero, `path` must point to a readable UTF-8 byte buffer of that size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_open(
    path: *const u8,
    path_length: usize,
    output: *mut *mut YuStorageSession,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `output` is a caller-owned pointer checked above.
    unsafe { *output = ptr::null_mut() };
    let path = match read_utf8(path, path_length) {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    let session = match DocumentEditorSession::open(path) {
        Ok(session) => session,
        Err(error) => return status_from_error(error),
    };
    let session = Box::new(YuStorageSession { session });
    // SAFETY: the pointer is transferred to the native caller as an opaque
    // handle and is reclaimed only by `yu_storage_session_destroy`.
    unsafe { *output = Box::into_raw(session) };
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle returned by
/// `yu_storage_session_open`, and must not be destroyed more than once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_destroy(session: *mut YuStorageSession) {
    if !session.is_null() {
        // SAFETY: the handle came from `yu_storage_session_open` and is
        // destroyed at most once by the caller.
        unsafe { drop(Box::from_raw(session)) };
    }
}

/// # Safety
///
/// `session` must be null or a live handle; `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_path_length(
    session: *const YuStorageSession,
    output: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let path = session.session.path().to_string_lossy();
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe { *output = path.len() };
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle. `output`/`written` must describe a
/// valid writable buffer, except for the documented null/zero-capacity query.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_path(
    session: *const YuStorageSession,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let path = session.session.path().to_string_lossy();
    write_bytes(path.as_bytes(), output, capacity, written)
}

/// # Safety
///
/// `session` must be null or a live handle and `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_source_length(
    session: *const YuStorageSession,
    output: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let source = session.session.snapshot();
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe { *output = source.as_str().len() };
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle. `output`/`written` must describe a
/// valid writable buffer, except for the documented null/zero-capacity query.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_source(
    session: *const YuStorageSession,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let source = session.session.snapshot();
    write_bytes(source.as_str().as_bytes(), output, capacity, written)
}

/// Copies a UTF-16-addressed source range without exposing Rust storage to the
/// native host. The range belongs to `expected_revision` and is suitable for a
/// local native mirror update after a command result reports `SOURCE_SYNC_RANGE`.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_source_range(
    session: *const YuStorageSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let range = match source_range_from_ffi(&session.session, start_utf16, end_utf16) {
        Ok(range) => range,
        Err(status) => return status,
    };
    let snapshot = session.session.snapshot();
    write_snapshot_range(&snapshot, range, output, capacity, written)
}

/// Copies the current Rust-owned selection as UTF-8. The expected revision
/// makes the operation safe for a native clipboard callback that was queued
/// before a source edit.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_selection(
    session: *const YuStorageSession,
    expected_revision: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let range = session.session.selection().ordered_range();
    let snapshot = session.session.snapshot();
    write_snapshot_range(&snapshot, range, output, capacity, written)
}

/// Returns a compact source-backed Accessibility snapshot. Every coordinate
/// in the result is valid only for `revision`; native queries must use the
/// revision-bound range/copy functions below rather than retaining Rust text.
///
/// # Safety
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_snapshot(
    session: *const YuStorageSession,
    output: *mut YuStorageAccessibilitySnapshot,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    accessibility_snapshot_output(&session.session, output)
}

/// Returns one logical LF-delimited line range from a source-backed
/// Accessibility snapshot. The line index is zero based and the terminating
/// LF belongs to the preceding line, matching `AccessibilityTextSnapshot`.
///
/// # Safety
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_line_range(
    session: *const YuStorageSession,
    expected_revision: u64,
    line: u64,
    output: *mut YuStorageAccessibilityRange,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    accessibility_line_range_output(&session.session, expected_revision, line, output)
}

/// Resolves a UTF-16 position to its zero-based logical LF-delimited line in
/// the same source-backed Accessibility snapshot.
///
/// # Safety
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_line_for_position(
    session: *const YuStorageSession,
    expected_revision: u64,
    offset_utf16: u64,
    output: *mut u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    accessibility_line_for_position_output(
        &session.session,
        expected_revision,
        offset_utf16,
        output,
    )
}

/// # Safety
///
/// `session` must be null or a live handle and `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_state(
    session: *const YuStorageSession,
    output: *mut YuStorageState,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let disk_state = match disk_state(&session.session) {
        Ok(value) => value,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe {
        *output = YuStorageState {
            revision: session.session.revision().get(),
            saved_revision: session.session.saved_revision().get(),
            dirty: u8::from(session.session.is_dirty()),
            disk_state,
            bom: match session.session.bom() {
                Utf8Bom::Absent => YU_STORAGE_BOM_ABSENT,
                Utf8Bom::Present => YU_STORAGE_BOM_PRESENT,
            },
            close_state: close_state(session.session.close_state()),
        };
    }
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle; all output pointers must be
/// writable when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_save(
    session: *mut YuStorageSession,
    revision_output: *mut u64,
    bytes_written_output: *mut usize,
    changed_output: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if revision_output.is_null() || bytes_written_output.is_null() || changed_output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let outcome = match session.session.save() {
        Ok(outcome) => outcome,
        Err(error) => return status_from_error(error),
    };
    let (revision, bytes_written, changed) = match outcome {
        SaveOutcome::Saved {
            revision,
            bytes_written,
        } => (revision.get(), bytes_written, 1),
        SaveOutcome::Unchanged { revision } => (revision.get(), 0, 0),
    };
    // SAFETY: all output pointers were checked above.
    unsafe {
        *revision_output = revision;
        *bytes_written_output = bytes_written;
        *changed_output = changed;
    }
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle and `revision_output` must be
/// writable when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_reload(
    session: *mut YuStorageSession,
    revision_output: *mut u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if revision_output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let outcome = match session.session.reload() {
        Ok(outcome) => outcome,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `revision_output` was checked and belongs to the caller.
    unsafe { *revision_output = outcome.revision.get() };
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle and `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_request_close(
    session: *mut YuStorageSession,
    output: *mut YuStorageCloseRequest,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let result = match session.session.close_request() {
        Ok(CloseRequest::CloseNow) => YU_STORAGE_CLOSE_NOW,
        Ok(CloseRequest::Prompt(_)) => YU_STORAGE_CLOSE_PROMPT,
        Ok(CloseRequest::AlreadyClosed) => YU_STORAGE_CLOSE_ALREADY_CLOSED,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe {
        *output = YuStorageCloseRequest {
            result,
            close_state: close_state(session.session.close_state()),
        };
    }
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_cancel_close(session: *mut YuStorageSession) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    session
        .session
        .cancel_close()
        .map(|_| YU_STORAGE_OK)
        .unwrap_or(YU_STORAGE_INVALID_STATE)
}

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_save_close(session: *mut YuStorageSession) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    match session.session.save_close() {
        Ok(_) => YU_STORAGE_OK,
        Err(error) => status_from_error(error),
    }
}

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_discard_close(session: *mut YuStorageSession) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    session
        .session
        .discard_close()
        .map(|_| YU_STORAGE_OK)
        .unwrap_or(YU_STORAGE_INVALID_STATE)
}

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_save_failed_external(
    session: *mut YuStorageSession,
    external_state: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let state = match external_file_state_from_ffi(external_state) {
        Ok(state) => state,
        Err(status) => return status,
    };
    session
        .session
        .save_failed_external(state)
        .map(|_| YU_STORAGE_OK)
        .unwrap_or(YU_STORAGE_INVALID_STATE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn status_and_state_contracts_are_stable() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-{id}.md"));
        fs::write(&path, "羽 日本語 🙂\n").expect("fixture");
        let editor_session = DocumentEditorSession::open(&path).expect("open");
        let session = YuStorageSession {
            session: editor_session,
        };
        assert_eq!(
            close_state(session.session.close_state()),
            YU_STORAGE_CLOSE_OPEN
        );
        assert_eq!(
            disk_state(&session.session).expect("disk state"),
            YU_STORAGE_DISK_UNCHANGED
        );
        assert_eq!(session.session.snapshot().as_str(), "羽 日本語 🙂\n");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn owned_byte_queries_support_null_zero_capacity_length_form() {
        let mut written = 0;
        assert_eq!(
            write_bytes("羽🙂".as_bytes(), ptr::null_mut(), 0, &mut written),
            YU_STORAGE_OK
        );
        assert_eq!(written, "羽🙂".len());
    }

    #[test]
    fn ffi_snapshot_queries_are_two_call_safe() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-query-{id}.md"));
        fs::write(&path, "# 羽\n日本語 🙂\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        let open_status =
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) };
        assert_eq!(open_status, YU_STORAGE_OK);
        assert!(!raw.is_null());

        let mut required = 0;
        assert_eq!(
            unsafe { yu_storage_session_copy_source(raw, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_OK
        );
        let mut source = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source(raw, source.as_mut_ptr(), source.len(), &mut written)
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, source.len());
        assert_eq!(
            String::from_utf8(source).expect("UTF-8 source"),
            "# 羽\n日本語 🙂\n"
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_accessibility_snapshot_and_line_ranges_are_revision_bound() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-ax-{id}.md"));
        fs::write(&path, "# 羽\n日本語 🙂\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut snapshot = YuStorageAccessibilitySnapshot::default();
        assert_eq!(
            unsafe { yu_storage_session_accessibility_snapshot(raw, &mut snapshot) },
            YU_STORAGE_OK
        );
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.number_of_characters_utf16, 11);
        assert_eq!(snapshot.selection_start_utf16, 11);
        assert_eq!(snapshot.selection_end_utf16, 11);
        assert_eq!(snapshot.line_count, 3);
        assert_eq!(
            snapshot.selection_affinity,
            YU_STORAGE_CARET_AFFINITY_DOWNSTREAM
        );

        let mut first_line = YuStorageAccessibilityRange::default();
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_range(
                    raw,
                    snapshot.revision,
                    0,
                    &mut first_line,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(first_line.revision, snapshot.revision);
        assert_eq!(first_line.start_utf16, 0);
        assert_eq!(first_line.end_utf16, 4);

        let mut third_line = YuStorageAccessibilityRange::default();
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_range(
                    raw,
                    snapshot.revision,
                    2,
                    &mut third_line,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(third_line.start_utf16, 11);
        assert_eq!(third_line.end_utf16, 11);

        let mut line = u64::MAX;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_for_position(
                    raw,
                    snapshot.revision,
                    5,
                    &mut line,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(line, 1);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, snapshot.revision, "x".as_ptr(), 1, &mut result)
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_range(
                    raw,
                    snapshot.revision,
                    0,
                    &mut first_line,
                )
            },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unified_ffi_command_and_composition_share_revision() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-editor-{id}.md"));
        fs::write(&path, "输入: ").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert!(!raw.is_null());

        let mut selection = YuStorageSelection::default();
        assert_eq!(
            unsafe { yu_storage_session_selection(raw, &mut selection) },
            YU_STORAGE_OK
        );
        assert_eq!(selection.revision, 0);
        assert_eq!(selection.start_utf16, 4);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(
                    raw,
                    YU_STORAGE_COMMAND_MOVE_LEFT,
                    0,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(result.revision, 0);
        assert_eq!(result.selection_end_utf16, 3);

        let preedit = "にほんご";
        assert_eq!(
            unsafe {
                yu_storage_session_begin_composition(
                    raw,
                    0,
                    4,
                    4,
                    preedit.as_ptr(),
                    preedit.len(),
                    4,
                    4,
                )
            },
            YU_STORAGE_OK
        );
        let mut state = YuStorageState::default();
        assert_eq!(
            unsafe { yu_storage_session_state(raw, &mut state) },
            YU_STORAGE_OK
        );
        assert_eq!(state.revision, 0);
        assert_eq!(state.dirty, 0);
        let mut composition = YuStorageCompositionState::default();
        assert_eq!(
            unsafe { yu_storage_session_composition(raw, &mut composition) },
            YU_STORAGE_OK
        );
        assert_eq!(composition.active, 1);
        assert_eq!(composition.generation, 1);
        let mut preedit_bytes = vec![0_u8; composition.preedit_utf8_length as usize];
        let mut preedit_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_composition(
                    raw,
                    composition.revision,
                    composition.generation,
                    preedit_bytes.as_mut_ptr(),
                    preedit_bytes.len(),
                    &mut preedit_written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(preedit_written, preedit_bytes.len());
        assert_eq!(String::from_utf8(preedit_bytes).expect("preedit"), preedit);
        assert_eq!(
            unsafe {
                yu_storage_session_update_composition(
                    raw,
                    composition.revision,
                    composition.generation.saturating_sub(1),
                    preedit.as_ptr(),
                    preedit.len(),
                    4,
                    4,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        let mut after_stale = YuStorageCompositionState::default();
        assert_eq!(
            unsafe { yu_storage_session_composition(raw, &mut after_stale) },
            YU_STORAGE_OK
        );
        assert_eq!(after_stale.generation, composition.generation);
        assert_eq!(
            after_stale.preedit_utf8_length,
            composition.preedit_utf8_length
        );
        assert_eq!(
            unsafe {
                yu_storage_session_commit_composition(
                    raw,
                    composition.revision,
                    composition.generation,
                    "日本語".as_ptr(),
                    "日本語".len(),
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_state(raw, &mut state) },
            YU_STORAGE_OK
        );
        assert_eq!(state.revision, 1);
        assert_eq!(state.dirty, 1);

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source_range(
                    raw,
                    1,
                    4,
                    7,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unified_ffi_insert_text_is_revision_bound_and_returns_range_sync() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-insert-{id}.md"));
        fs::write(&path, "a").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let mut result = YuStorageCommandResult::default();
        let text = "日本語";
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, 99, text.as_ptr(), text.len(), &mut result)
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, 0, text.as_ptr(), text.len(), &mut result)
            },
            YU_STORAGE_OK
        );
        assert_eq!(result.revision, 1);
        assert_eq!(result.source_sync, YU_STORAGE_SOURCE_SYNC_RANGE);
        assert_eq!(result.source_start_utf16, 1);
        assert_eq!(result.source_old_end_utf16, 1);
        assert_eq!(result.source_new_end_utf16, 4);

        let mut required = 0;
        assert_eq!(
            unsafe { yu_storage_session_copy_source(raw, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_OK
        );
        let mut source = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source(raw, source.as_mut_ptr(), source.len(), &mut written)
            },
            YU_STORAGE_OK
        );
        assert_eq!(String::from_utf8(source).expect("source"), "a日本語");
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unified_ffi_copy_selection_is_revision_bound_and_utf8_owned() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-copy-{id}.md"));
        fs::write(&path, "A🙂日本語Z").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection(raw, 0, 1, 6, YU_STORAGE_CARET_AFFINITY_DOWNSTREAM)
            },
            YU_STORAGE_OK
        );
        let mut required = 0;
        assert_eq!(
            unsafe { yu_storage_session_copy_selection(raw, 0, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_OK
        );
        let mut selected = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
                    selected.as_mut_ptr(),
                    selected.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            String::from_utf8(selected).expect("selected UTF-8"),
            "🙂日本語"
        );
        assert_eq!(
            unsafe { yu_storage_session_copy_selection(raw, 1, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_STALE_REVISION
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }
}
