//! Owned bytes around the public macOS PDF drawing adapter. Native objects
//! are created and destroyed on this call's worker; no editor/window pointer
//! is transferred. The callback is synchronous and cannot outlive its borrow.
use std::ffi::{c_char, c_void};

pub struct RenderedPdf {
    pub bytes: Vec<u8>,
    pub pages: u32,
}
unsafe extern "C" {
    fn yu_macos_export_pdf(
        json: *const u8,
        length: usize,
        owner: *mut c_void,
        check: unsafe extern "C" fn(*mut c_void) -> bool,
        bytes: *mut *mut c_void,
        output_length: *mut usize,
        pages: *mut u32,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn yu_macos_export_pdf_free(bytes: *mut c_void);
}
pub fn export_pdf<F: FnMut() -> bool>(packet: &[u8], mut check: F) -> Result<RenderedPdf, String> {
    unsafe extern "C" fn checkpoint<F: FnMut() -> bool>(context: *mut c_void) -> bool {
        // Native code invokes this serially, only within the export call.
        let callback = unsafe { &mut *context.cast::<F>() };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)).unwrap_or(false)
    }
    if packet.is_empty() || packet.len() > 256 * 1024 * 1024 {
        return Err("PDF 中间内容超限".into());
    }
    let mut output = std::ptr::null_mut();
    let mut length = 0;
    let mut pages = 0;
    let mut error = [0u8; 2048];
    let code = unsafe {
        yu_macos_export_pdf(
            packet.as_ptr(),
            packet.len(),
            std::ptr::from_mut(&mut check).cast(),
            checkpoint::<F>,
            &mut output,
            &mut length,
            &mut pages,
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    struct NativeBytes(*mut c_void);
    impl Drop for NativeBytes {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { yu_macos_export_pdf_free(self.0) };
            }
        }
    }
    let owned = NativeBytes(output);
    if code != 1
        || owned.0.is_null()
        || length == 0
        || length > 256 * 1024 * 1024
        || !(1..=1000).contains(&pages)
    {
        let end = error.iter().position(|&b| b == 0).unwrap_or(error.len());
        return Err(if end == 0 {
            "原生 PDF 输出失败".into()
        } else {
            String::from_utf8_lossy(&error[..end]).into_owned()
        });
    }
    let bytes = unsafe { std::slice::from_raw_parts(owned.0.cast::<u8>(), length) }.to_vec();
    if !bytes.starts_with(b"%PDF-") {
        return Err("PDF 输出头无效".into());
    }
    Ok(RenderedPdf { bytes, pages })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_page_limit_and_cancellation_use_native_writer() {
        // Large test text occupies one page per block. This exercises the
        // production 1000-page guard without giant source/images or disk fills.
        let block = r#"{"kind":"paragraph","style":"body","align":"left","indent":0,"runs":[{"text":"x","scale":40}]}"#;
        let packet = |count| {
            format!(
                r#"{{"title":"Page budget","width":612,"height":792,"margin":36,"pageNumbers":false,"maxPages":1000,"minFigureScale":0.25,"images":[],"blocks":[{}]}}"#,
                vec![block; count].join(",")
            )
        };
        let exact = export_pdf(packet(1000).as_bytes(), || true).expect("1000 real pages");
        assert_eq!(exact.pages, 1000);
        let error = export_pdf(packet(1001).as_bytes(), || true)
            .err()
            .expect("1001 refused");
        assert!(error.contains("1000"), "{error}");
        let mut checks = 0;
        let cancelled = export_pdf(packet(20).as_bytes(), || {
            checks += 1;
            checks < 15
        });
        assert!(cancelled.is_err());
        assert!(checks >= 15);
        assert_eq!(
            export_pdf(packet(1).as_bytes(), || true)
                .expect("retry")
                .pages,
            1
        );
    }
}
