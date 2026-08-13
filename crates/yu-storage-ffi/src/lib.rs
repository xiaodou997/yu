#![allow(unsafe_code)]

//! Narrow C ABI for the macOS document-shell spike.
//!
//! `YuStorageSession` owns the only mutable `DocumentSession`. Native code may
//! request owned UTF-8 snapshots and revision-bound state, but it cannot
//! mutate source or reproduce dirty/conflict policy. The eventual text-input
//! bridge can be composed with this boundary after the AppKit lifecycle is
//! proven; this phase intentionally keeps the host source view read-only.

use std::path::PathBuf;
use std::ptr;

use yu_storage::{
    ClosePrompt, CloseRequest, CloseState, CloseStateMachine, DiskState, DocumentSession,
    ExternalFileState, SaveOutcome, StorageError, Utf8Bom,
};

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
pub struct YuStorageSession {
    document: DocumentSession,
    close: CloseStateMachine,
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
        StorageError::Editor(_) => YU_STORAGE_EDITOR_ERROR,
    }
}

fn disk_state(session: &DocumentSession) -> Result<u8, StorageError> {
    Ok(match session.disk_state()? {
        DiskState::Unchanged => YU_STORAGE_DISK_UNCHANGED,
        DiskState::Changed => YU_STORAGE_DISK_CHANGED,
        DiskState::Missing => YU_STORAGE_DISK_MISSING,
    })
}

fn close_state(session: &CloseStateMachine) -> u8 {
    match session.state() {
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
    let document = match DocumentSession::open(path) {
        Ok(document) => document,
        Err(error) => return status_from_error(error),
    };
    let session = Box::new(YuStorageSession {
        document,
        close: CloseStateMachine::new(),
    });
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
    let path = session.document.path().to_string_lossy();
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
    let path = session.document.path().to_string_lossy();
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
    let source = session.document.editor().snapshot();
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
    let source = session.document.editor().snapshot();
    write_bytes(source.as_str().as_bytes(), output, capacity, written)
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
    let disk_state = match disk_state(&session.document) {
        Ok(value) => value,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe {
        *output = YuStorageState {
            revision: session.document.revision().get(),
            saved_revision: session.document.saved_revision().get(),
            dirty: u8::from(session.document.is_dirty()),
            disk_state,
            bom: match session.document.bom() {
                Utf8Bom::Absent => YU_STORAGE_BOM_ABSENT,
                Utf8Bom::Present => YU_STORAGE_BOM_PRESENT,
            },
            close_state: close_state(&session.close),
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
    let outcome = match session.document.save() {
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
    let outcome = match session.document.reload() {
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
    let result = match session.document.close_request(&mut session.close) {
        Ok(CloseRequest::CloseNow) => YU_STORAGE_CLOSE_NOW,
        Ok(CloseRequest::Prompt(_)) => YU_STORAGE_CLOSE_PROMPT,
        Ok(CloseRequest::AlreadyClosed) => YU_STORAGE_CLOSE_ALREADY_CLOSED,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe {
        *output = YuStorageCloseRequest {
            result,
            close_state: close_state(&session.close),
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
        .close
        .cancel()
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
    if let Err(error) = session.document.save() {
        return status_from_error(error);
    }
    session
        .close
        .save_succeeded()
        .map(|_| YU_STORAGE_OK)
        .unwrap_or(YU_STORAGE_INVALID_STATE)
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
        .close
        .discard()
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
        .close
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
        let document = DocumentSession::open(&path).expect("open");
        let session = YuStorageSession {
            document,
            close: CloseStateMachine::new(),
        };
        assert_eq!(close_state(&session.close), YU_STORAGE_CLOSE_OPEN);
        assert_eq!(
            disk_state(&session.document).expect("disk state"),
            YU_STORAGE_DISK_UNCHANGED
        );
        assert_eq!(
            session.document.editor().snapshot().as_str(),
            "羽 日本語 🙂\n"
        );
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
}
