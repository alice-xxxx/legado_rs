//! 应用只注入存储和目录；不实现规则调度或平台桥接。
pub use engine::{EngineFuture, SourceExecutor};
use std::{path::PathBuf, sync::Arc};
use tauri_plugin_source_engine as engine;
pub fn context(app_dir: PathBuf, resource_dir: Option<PathBuf>) -> engine::EngineContext {
    engine::EngineContext {
        storage: Arc::new(crate::database::SourceState::new(&app_dir)),
        data_dir: app_dir.join("source-engine"),
        resource_dir,
        cancellation: None,
    }
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub fn client(
    app: tauri::AppHandle,
    data_dir: PathBuf,
    resource_dir: Option<PathBuf>,
) -> engine::Client<tauri::Wry> {
    engine::Client::new(app, context(data_dir, resource_dir))
}
pub(crate) fn book_request(
    source: &serde_json::Value,
    book: serde_json::Value,
) -> Result<engine::BookRequest, String> {
    Ok(engine::BookRequest::new(
        source.clone().try_into()?,
        serde_json::from_value(book).map_err(|e| e.to_string())?,
    ))
}
