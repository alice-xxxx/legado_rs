//! 系统平台与 KMP 书源引擎之间的 Rust 执行器适配。

use super::*;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub struct TauriSourceExecutor {
    _app: tauri::AppHandle,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    data_dir: PathBuf,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    resource_dir: Option<PathBuf>,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl TauriSourceExecutor {
    pub fn new(app: tauri::AppHandle, _data_dir: PathBuf, _resource_dir: Option<PathBuf>) -> Self {
        Self {
            _app: app,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            data_dir: _data_dir,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            resource_dir: _resource_dir,
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl SourceExecutor for TauriSourceExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                use tauri_plugin_source_engine::{SourceEngineCall, SourceEngineExt};

                let request_json = serde_json::json!({
                    "operation": request.operation,
                    "source": request.source,
                    "keyword": request.keyword,
                    "credentials": request.credentials,
                    "actionId": request.action_id,
                    "mediaUrl": request.media_url,
                    "mediaHeaders": request.media_headers,
                    "page": request.page,
                    "book": request.book,
                    "chapter": request.chapter,
                    "nextChapterUrl": request.next_chapter_url,
                })
                .to_string();
                // The plugin bridge waits synchronously for the native call.
                // Keep it off Tokio's worker lane so search cancellation can run.
                let app = self._app.clone();
                let cancellation = request.cancellation.clone();
                let data_dir = crate::app_data_root(&app).map_err(|error| error.to_string())?.join("source-engine");
                let response = tokio::task::spawn_blocking(move || {
                    let mut wire: Value = serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
                    let _execution = crate::source_host_ffi::ExecutionScope::enter(&mut wire, data_dir, cancellation)?;
                    let request_json = wire.to_string();
                    app.source_engine().execute(SourceEngineCall { request_json })
                        .map_err(|error| error.to_string())
                }).await.map_err(|error| format!("Native source worker failed: {error}"))??;
                let result: Value =
                    serde_json::from_str(&response.result_json).map_err(|error| {
                        format!("Kotlin source engine returned invalid JSON: {error}")
                    })?;
                if let Some(error) = result.get("__sourceEngineError").and_then(Value::as_str) {
                    return Err(format!("Kotlin source engine failed:\n{error}"));
                }
                Ok(result)
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                crate::source_engine::execute_with_resource_dir(
                    request,
                    self.data_dir.clone(),
                    self.resource_dir.clone(),
                )
                .await
            }
        })
    }
}
