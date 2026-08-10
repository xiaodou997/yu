#![allow(unsafe_code)]

use std::ptr;

use yu_core::{TextRange, Utf16Offset, Utf16Range};
use yu_editor::CompositionOverlay;
use yu_text::TextBuffer;

pub const YU_FFI_OK: i32 = 0;
pub const YU_FFI_NULL_POINTER: i32 = 1;
pub const YU_FFI_INVALID_UTF8: i32 = 2;
pub const YU_FFI_INVALID_RANGE: i32 = 3;
pub const YU_FFI_INVALID_SELECTION: i32 = 4;
pub const YU_FFI_NO_OVERLAY: i32 = 5;
pub const YU_FFI_BUFFER_TOO_SMALL: i32 = 6;
pub const YU_FFI_EDIT_FAILED: i32 = 7;

/// Opaque state owned by the Rust side of the native composition bridge.
#[repr(C)]
pub struct YuCompositionSession {
    buffer: TextBuffer,
    overlay: Option<CompositionOverlay>,
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
    let snapshot = session.buffer.snapshot();
    let source_start = snapshot
        .byte_offset_for_utf16(range.start())
        .map_err(|_| YU_FFI_INVALID_RANGE)?;
    let source_end = snapshot
        .byte_offset_for_utf16(range.end())
        .map_err(|_| YU_FFI_INVALID_RANGE)?;
    TextRange::new(source_start, source_end).ok_or(YU_FFI_INVALID_RANGE)
}

fn selection_from_utf16(start: u64, end: u64) -> Result<Utf16Range, i32> {
    Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end)).ok_or(YU_FFI_INVALID_SELECTION)
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
        buffer: TextBuffer::new(source),
        overlay: None,
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
    if session.overlay.is_some() {
        return YU_FFI_EDIT_FAILED;
    }
    let source = match read_utf8(source, source_length) {
        Ok(source) => source,
        Err(status) => return status,
    };
    session.buffer = TextBuffer::new(source);
    YU_FFI_OK
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
    let overlay =
        match CompositionOverlay::new(session.buffer.revision(), replacement, preedit, selection) {
            Ok(overlay) => overlay,
            Err(_) => return YU_FFI_INVALID_SELECTION,
        };
    session.overlay = Some(overlay);
    YU_FFI_OK
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
    let Some(overlay) = session.overlay.as_mut() else {
        return YU_FFI_NO_OVERLAY;
    };
    overlay
        .update(preedit, selection)
        .map(|()| YU_FFI_OK)
        .unwrap_or(YU_FFI_INVALID_SELECTION)
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
    let Some(overlay) = session.overlay.as_ref() else {
        return YU_FFI_NO_OVERLAY;
    };
    let transaction = overlay.clone().commit(committed_text);
    if session.buffer.apply(&transaction).is_err() {
        return YU_FFI_EDIT_FAILED;
    }
    session.overlay = None;
    YU_FFI_OK
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
    session.overlay = None;
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
    unsafe { *output = session.buffer.revision().get() };
    YU_FFI_OK
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
    unsafe { *output = session.buffer.snapshot().len_bytes().get() as usize };
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
    let snapshot = session.buffer.snapshot();
    let source = snapshot.as_str();
    write_bytes(source.as_bytes(), output, capacity)
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
            .overlay
            .as_ref()
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
    let Some(overlay) = session.overlay.as_ref() else {
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
    let Some(overlay) = session.overlay.as_ref() else {
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
}
