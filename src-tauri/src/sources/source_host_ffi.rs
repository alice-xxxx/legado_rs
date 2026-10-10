//! 提供 KMP 引擎调用 Rust 宿主能力的 FFI 边界。
//! 移动端 KMP 使用的稳定 C ABI。Android 通过 JNI 调用 Rust Host，iOS 通过
//! Kotlin/Native cinterop 调用同一组 Rust HTTP 与存储实现。

use crate::source_http::{SourceHttpRequest, execute_source_http_request};
use crate::source_storage::{SourceStorageRequest, execute_source_storage_request};
use serde::Serialize;
use serde_json::json;
use std::ffi::{CStr, CString};
use std::future::Future;
use std::os::raw::c_char;
use std::path::Path;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

pub(crate) fn http_json(input: &str) -> String {
    let result = (|| {
        let request: SourceHttpRequest = serde_json::from_str(input)
            .map_err(|error| format!("Invalid source HTTP request: {error}"))?;
        run_on_runtime(async move { execute_source_http_request(request).await })
    })();
    envelope(result)
}

pub(crate) fn storage_json(input: &str, app_data_dir: &Path) -> String {
    let result = (|| {
        let request: SourceStorageRequest = serde_json::from_str(input)
            .map_err(|error| format!("Invalid source storage request: {error}"))?;
        let data_dir = app_data_dir.to_path_buf();
        run_on_runtime(async move { execute_source_storage_request(request, &data_dir).await })
    })();
    envelope(result)
}

pub(crate) fn image_json(input: &str) -> String {
    crate::image_ops_host::execute_json(input)
}

fn envelope<T: Serialize>(result: Result<T, String>) -> String {
    match result {
        Ok(value) => json!({ "ok": true, "value": value }).to_string(),
        Err(error) => json!({ "ok": false, "error": error }).to_string(),
    }
}

fn runtime() -> Result<&'static Runtime, String> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("legado-source-host")
                .build()
                .map_err(|error| format!("Cannot create Rust source host runtime: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// JNI and C callers are synchronous, while reqwest and storage use Tokio. A dedicated runtime
/// avoids nesting `block_on` inside callers that are already running on a coroutine dispatcher.
pub(crate) fn run_on_runtime<T, F>(future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    runtime()?.spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver
        .recv()
        .map_err(|error| format!("Rust source host runtime stopped: {error}"))?
}

fn string_from_c(value: *const c_char, label: &str) -> Result<String, String> {
    if value.is_null() {
        return Err(format!("{label} pointer is null"));
    }
    // SAFETY: callers promise NUL-terminated UTF-8 strings as declared by the public header.
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map(str::to_owned)
        .map_err(|error| format!("{label} is not valid UTF-8: {error}"))
}

fn into_c_string(value: String) -> *mut c_char {
    // JSON strings cannot contain an unescaped NUL byte. Keep an explicit fallback so malformed
    // internal data can never cross the ABI as a truncated response.
    CString::new(value.replace('\0', "\\u0000"))
        .unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"invalid response\"}").unwrap())
        .into_raw()
}

/// Execute a Rust-owned HTTP request. Input is the serialized `SourceHttpRequest` value.
#[no_mangle]
pub unsafe extern "C" fn legado_source_host_http(request_json: *const c_char) -> *mut c_char {
    let response = string_from_c(request_json, "request_json")
        .map(|request| http_json(&request))
        .unwrap_or_else(|error| envelope::<serde_json::Value>(Err(error)));
    into_c_string(response)
}

/// Execute a Rust-owned storage operation under the caller-provided app data directory.
#[no_mangle]
pub unsafe extern "C" fn legado_source_host_storage(
    request_json: *const c_char,
    app_data_dir: *const c_char,
) -> *mut c_char {
    let response = string_from_c(request_json, "request_json")
        .and_then(|request| string_from_c(app_data_dir, "app_data_dir").map(|dir| (request, dir)))
        .map(|(request, dir)| storage_json(&request, Path::new(&dir)))
        .unwrap_or_else(|error| envelope::<serde_json::Value>(Err(error)));
    into_c_string(response)
}

/// Execute one bounded image pixel operation requested by the KMP ImageOps provider.
#[no_mangle]
pub unsafe extern "C" fn legado_source_host_image(request_json: *const c_char) -> *mut c_char {
    let response = string_from_c(request_json, "request_json")
        .map(|request| image_json(&request))
        .unwrap_or_else(|error| envelope::<serde_json::Value>(Err(error)));
    into_c_string(response)
}

/// Release a response returned by either C ABI operation.
#[no_mangle]
pub unsafe extern "C" fn legado_source_host_string_free(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: `value` was created by `CString::into_raw` in this library.
        drop(CString::from_raw(value));
    }
}
