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
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        use tauri_plugin_source_engine::{SourceEngineCall, SourceEngineExt};

        // Mobile platforms enter their KMP-native parser through the Tauri plugin. Keep the
        // existing request shape intact; the platform adapter decodes it with the shared codec.
        let request_json = serde_json::json!({
            "operation": request.operation,
            "source": request.source,
            "keyword": request.keyword,
            "page": request.page,
            "book": request.book,
            "chapter": request.chapter,
            "nextChapterUrl": request.next_chapter_url,
        })
        .to_string();
        let response = app
            .source_engine()
            .execute(SourceEngineCall { request_json })
            .map_err(|error| error.to_string())?;
        let result: serde_json::Value = serde_json::from_str(&response.result_json)
            .map_err(|error| format!("Kotlin source engine returned invalid JSON: {error}"))?;
        if let Some(error) = result
            .get("__sourceEngineError")
            .and_then(serde_json::Value::as_str)
        {
            return Err(format!("Kotlin source engine failed:\n{error}"));
        }
        Ok(result)
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use tauri::Manager;

        // Keep mobile data access native; desktop state remains under Tauri's app-data directory.
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| format!("Cannot locate application data directory: {error}"))?
            .join("source-engine");
        let resource_dir = app.path().resource_dir().ok();
        source_engine::execute_with_resource_dir(request, data_dir, resource_dir).await
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(feature = "desktop")]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_source_engine::init())
        .invoke_handler(tauri::generate_handler![execute_source_engine])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
