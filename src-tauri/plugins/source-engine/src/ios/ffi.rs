//! iOS C ABI：输入由调用方持有，返回字符串必须通过配套入口释放。
use std::ffi::{CStr, CString, c_char};
fn read(input: *const c_char) -> crate::Result<String> {
    if input.is_null() {
        return Err("原生参数为空".into());
    }
    unsafe { CStr::from_ptr(input) }
        .to_str()
        .map(str::to_owned)
        .map_err(|e| e.to_string())
}
fn output(value: String) -> *mut c_char {
    CString::new(value.replace('\0', "\\u0000"))
        .expect("JSON 无未转义的空字节")
        .into_raw()
}
macro_rules! endpoint {
    ($name:ident,$function:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(input: *const c_char) -> *mut c_char {
            output(match read(input) {
                Ok(input) => $function(&input),
                Err(error) => crate::ffi::response::<()>(Err(error)),
            })
        }
    };
}
endpoint!(foliver_source_http, crate::http::native);
endpoint!(foliver_source_storage, crate::storage::native);
endpoint!(foliver_source_image, crate::image::native);
endpoint!(foliver_source_browser, crate::browser::native);
#[unsafe(no_mangle)]
pub extern "C" fn foliver_source_cancelled(id: *const c_char) -> i32 {
    read(id).map_or(1, |id| i32::from(crate::engine::context::cancelled(&id)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn foliver_source_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) })
    }
}
