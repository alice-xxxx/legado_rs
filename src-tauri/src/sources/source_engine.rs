//! App integration: paths and storage are injected through the plugin's host interface.
use std::{path::PathBuf, sync::Arc};
use serde_json::Value;
use tauri_plugin_source_engine as engine;
pub use engine::{EngineFuture, SourceExecutor};

pub fn execution_context(data_dir: PathBuf, resource_dir: Option<PathBuf>) -> engine::ExecutionContext {
    engine::ExecutionContext {
        storage: Arc::new(crate::database::SourceState::new(&data_dir)),
        data_dir, resource_dir, cancellation: None,
    }
}

pub(crate) fn book_request(source: &Value, book: Value) -> Result<engine::BookRequest, String> {
    Ok(engine::BookRequest::new(source.try_into()?,
        serde_json::from_value(book).map_err(|error| error.to_string())?))
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub struct TauriSourceExecutor {
    app: tauri::AppHandle,
    context: engine::ExecutionContext,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl TauriSourceExecutor {
    pub fn new(app: tauri::AppHandle, data_dir: PathBuf, resource_dir: Option<PathBuf>) -> Self {
        Self { app, context: execution_context(data_dir, resource_dir) }
    }

    fn context(&self, cancellation: tokio_util::sync::CancellationToken) -> Result<engine::ExecutionContext, String> {
        let mut context = self.context.clone();
        context.cancellation = Some(cancellation);
        #[cfg(any(target_os = "android", target_os = "ios"))]
        { context.data_dir = crate::app_data_root(&self.app).map_err(|e| e.to_string())?.join("source-engine"); }
        Ok(context)
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
macro_rules! delegate {
    ($method:ident, $request:ty, $response:ty) => {
        fn $method<'a>(&'a self, request: $request, cancellation: tokio_util::sync::CancellationToken) -> EngineFuture<'a, $response> {
            use engine::SourceEngineExt;
            Box::pin(async move {
                self.app.source_engine().$method(request, self.context(cancellation)?).await.map_err(|e| e.to_string())
            })
        }
    };
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl SourceExecutor for TauriSourceExecutor {
    delegate!(search, engine::SearchRequest, engine::SearchPage);
    delegate!(explore_kinds, engine::SourceRequest, engine::ExploreKinds);
    delegate!(explore, engine::ExploreRequest, engine::SearchPage);
    delegate!(detail, engine::BookRequest, engine::Book);
    delegate!(chapters, engine::BookRequest, engine::ChapterList);
    delegate!(content, engine::ContentRequest, engine::ChapterContent);
    delegate!(resolve_media, engine::MediaRequest, engine::MediaLocation);
    delegate!(http_tts_audio, engine::TtsRequest, engine::AudioData);
    delegate!(login, engine::LoginRequest, engine::LoginState);
}
