//! 设置只提供 settings.json，文件读写复用公共存储。
use super::{file::Files, stream};
use crate::resources::ResourceError;
use axum::{
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde_json::Value;
use std::{io, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct Settings(Files, Arc<Mutex<()>>);

impl Settings {
    pub(crate) async fn new(root: PathBuf) -> Result<Self, ResourceError> {
        let settings_file = Self(Files::new(root).await?, Arc::new(Mutex::new(())));
        let mut defaults = serde_json::to_value(crate::settings::SettingsDocument::default())
            .map_err(|error| ResourceError::new(error.to_string()))?;
        defaults["lastBackupAtMs"] = Value::Null;
        let mut settings = settings_file.read().await.unwrap_or(defaults.clone());
        crate::settings::upgrade_settings_document(&mut settings);
        if !valid_startup_settings(&settings) {
            settings = defaults;
        }
        settings_file.save(&settings).await?;
        set_source_timeout(&settings);
        Ok(settings_file)
    }

    pub(crate) async fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let _update = self.1.lock().await;
        self.0.write_file("settings.json", bytes).await
    }

    pub(crate) async fn read(&self) -> Result<Value, ResourceError> {
        serde_json::from_slice(&self.0.read_file("settings.json").await?)
            .map_err(|error| ResourceError::new(error.to_string()))
    }

    pub(crate) async fn save(&self, value: &Value) -> Result<(), ResourceError> {
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|error| ResourceError::new(error.to_string()))?;
        Ok(self.write(&bytes).await?)
    }

    /// 设置的读改写串行执行，文件发布仍复用公共写入。
    pub(crate) async fn update<F>(&self, change: F) -> Result<Value, ResourceError>
    where
        F: FnOnce(Value) -> Result<Value, String>,
    {
        let _update = self.1.lock().await;
        let value = change(self.read().await?).map_err(ResourceError::new)?;
        let bytes = serde_json::to_vec_pretty(&value)
            .map_err(|error| ResourceError::new(error.to_string()))?;
        self.0.write_file("settings.json", &bytes).await?;
        Ok(value)
    }

    pub(super) fn routes(&self) -> Router {
        Router::new()
            .route("/settings.json", get(read))
            .with_state(self.clone())
    }
}

async fn read(
    State(settings): State<Settings>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    settings
        .0
        .read("settings.json", &method, &headers, "")
        .await
        .map(stream::no_store)
}

fn valid_startup_settings(value: &Value) -> bool {
    serde_json::from_value::<crate::settings::SettingsDocument>(value.clone()).is_ok_and(
        |settings| {
            settings.schema_version == crate::settings::SCHEMA_VERSION
                && crate::discovery::home_config::validate_home_document(&settings.home).is_ok()
        },
    )
}

fn set_source_timeout(settings: &Value) {
    let seconds = settings["sourceHttpTimeoutSeconds"]
        .as_u64()
        .filter(|seconds| [15, 30, 60, 120].contains(seconds))
        .unwrap_or(15);
    crate::source_http::set_network_timeout_seconds(seconds);
}
impl crate::resources::Resources {
    pub async fn bootstrap(&self) -> Result<Value, String> {
        let settings = self
            .settings
            .read()
            .await
            .map_err(|error| error.to_string())?;
        set_source_timeout(&settings);
        Ok(serde_json::json!({"settings":settings,"assetBaseUrl":self.base_url()}))
    }
}

/// 设置拥有的资源交给缓存清理保留，不让数据库解释设置文件。
pub(crate) fn protected_resources(
    root: &std::path::Path,
) -> Result<Vec<String>, crate::resources::ResourceError> {
    let bytes = match std::fs::read(root.join("settings.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| crate::resources::ResourceError::new(error.to_string()))?;
    let settings: crate::settings::SettingsDocument = serde_json::from_value(value)
        .map_err(|error| crate::resources::ResourceError::new(error.to_string()))?;
    Ok(settings
        .reader
        .background_image_src
        .into_iter()
        .map(|reference| reference.to_string())
        .collect())
}
