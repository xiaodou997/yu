//! Owned, immutable raster drawing plan. No bitmap survives an encode call.
use std::ffi::{CString, c_char, c_void};
use std::path::Path;

#[cfg(test)]
#[path = "png_tests.rs"]
mod tests;

pub struct PngPlan {
    native: *mut c_void,
    pub sizes: Vec<[u32; 2]>,
}
// Ownership is transferred, not shared; the task serializes all native access.
unsafe impl Send for PngPlan {}
impl Drop for PngPlan {
    fn drop(&mut self) {
        unsafe { yu_macos_png_free(self.native) };
    }
}
unsafe extern "C" {
    fn yu_macos_png_prepare(
        json: *const u8,
        length: usize,
        owner: *mut c_void,
        check: unsafe extern "C" fn(*mut c_void) -> bool,
        plan: *mut *mut c_void,
        count: *mut u32,
        error: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn yu_macos_png_size(plan: *mut c_void, index: u32, width: *mut u32, height: *mut u32) -> i32;
    fn yu_macos_png_encode(
        plan: *mut c_void,
        index: u32,
        owner: *mut c_void,
        check: unsafe extern "C" fn(*mut c_void) -> bool,
        limit: usize,
        bytes: *mut *mut c_void,
        length: *mut usize,
        error: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn yu_macos_png_free(plan: *mut c_void);
    fn yu_macos_export_pdf_free(bytes: *mut c_void);
    fn yu_macos_move_directory_exclusive(source: *const c_char, target: *const c_char) -> i32;
}
unsafe extern "C" fn checkpoint<F: FnMut() -> bool>(owner: *mut c_void) -> bool {
    let f = unsafe { &mut *owner.cast::<F>() };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(false)
}
fn error_message(error: &[u8], default: &str) -> String {
    let end = error.iter().position(|&b| b == 0).unwrap_or(error.len());
    if end == 0 {
        default.to_owned()
    } else {
        String::from_utf8_lossy(&error[..end]).into_owned()
    }
}
pub fn prepare_png<F: FnMut() -> bool>(packet: &[u8], mut check: F) -> Result<PngPlan, String> {
    if packet.is_empty() || packet.len() > 256 * 1024 * 1024 {
        return Err("PNG中间内容超限".into());
    }
    let mut native = std::ptr::null_mut();
    let mut count = 0;
    let mut error = [0u8; 2048];
    let code = unsafe {
        yu_macos_png_prepare(
            packet.as_ptr(),
            packet.len(),
            std::ptr::from_mut(&mut check).cast(),
            checkpoint::<F>,
            &mut native,
            &mut count,
            error.as_mut_ptr().cast(),
            error.len(),
        )
    };
    let mut plan = PngPlan {
        native,
        sizes: Vec::new(),
    };
    if code != 1 || plan.native.is_null() || !(1..=64).contains(&count) {
        return Err(error_message(&error, "PNG布局失败"));
    }
    for i in 0..count {
        let (mut w, mut h) = (0, 0);
        if unsafe { yu_macos_png_size(native, i, &mut w, &mut h) } != 1 {
            return Err("PNG分段度量无效".into());
        }
        plan.sizes.push([w, h]);
    }
    Ok(plan)
}
impl PngPlan {
    pub fn encode<F: FnMut() -> bool>(
        &mut self,
        index: usize,
        limit: usize,
        mut check: F,
    ) -> Result<Vec<u8>, String> {
        if index >= self.sizes.len() {
            return Err("PNG分段索引无效".into());
        }
        let mut bytes = std::ptr::null_mut();
        let mut length = 0;
        let mut error = [0u8; 2048];
        let code = unsafe {
            yu_macos_png_encode(
                self.native,
                index as u32,
                std::ptr::from_mut(&mut check).cast(),
                checkpoint::<F>,
                limit,
                &mut bytes,
                &mut length,
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        struct Bytes(*mut c_void);
        impl Drop for Bytes {
            fn drop(&mut self) {
                unsafe { yu_macos_export_pdf_free(self.0) };
            }
        }
        let owned = Bytes(bytes);
        if code != 1 || owned.0.is_null() || length == 0 || length > limit {
            return Err(error_message(&error, "PNG编码失败"));
        }
        let result = unsafe { std::slice::from_raw_parts(owned.0.cast::<u8>(), length) }.to_vec();
        if !result.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("PNG签名无效".into());
        }
        Ok(result)
    }
}
pub fn move_directory_exclusive(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| "PNG临时目录无效")?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(|_| "PNG目标目录无效")?;
    if unsafe { yu_macos_move_directory_exclusive(source.as_ptr(), target.as_ptr()) } != 0 {
        return Err("PNG目录已存在或不能提交；未合并或覆盖".into());
    }
    Ok(())
}
