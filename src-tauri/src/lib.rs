pub mod source_engine;
mod source_host_ffi;
mod source_http;
mod source_jni;
mod source_storage;

// 桌面测试界面将书源操作交给 Rust source_engine；Rust 通过 JNI 调用嵌入式 KMP 解析器。
// HTTP 与存储由 Rust 宿主处理，此 command 只负责接收界面请求和返回结果。
#[cfg(feature = "desktop")]
#[tauri::command]
async fn execute_source_engine(
    app: tauri::AppHandle,
    request: source_engine::SourceEngineRequest,
) -> Result<serde_json::Value, String> {
    use tauri::Manager;

    // 将 cookie jar 等宿主数据放在应用数据目录，使同一桌面应用的多次测试可复用。
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Cannot locate application data directory: {error}"))?
        .join("source-engine");
    source_engine::execute(request, data_dir).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(feature = "desktop")]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![execute_source_engine])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
