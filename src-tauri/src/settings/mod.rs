//! 设置文件的读取、校验与保存；设置不属于业务数据库。

mod reader;
mod types;
mod upgrade;
pub(crate) use reader::ReaderDefaults;
pub(crate) use types::SettingsDocument;
pub(crate) use upgrade::upgrade_settings_document;
pub(crate) const SCHEMA_VERSION: u32 = 1;

pub(crate) mod appearance;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) mod import;
#[cfg(test)]
mod tests;

use crate::resources::Resources;
use serde_json::{json, Value};

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::{Emitter, State};

/// 设置保存与备份元数据更新共用，避免互相覆盖。
pub(crate) static WRITE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn save(
    resources: &Resources,
    database: &crate::database::Database,
    mut document: Value,
) -> Result<Value, String> {
    if !document.is_object() {
        return Err("设置必须是对象".into());
    }
    crate::settings::upgrade_settings_document(&mut document);
    if let Some(src) = document["reader"]["backgroundImageSrc"].as_str() {
        let stable = src
            .strip_prefix(resources.base_url())
            .map(|path| format!("resource://{path}"))
            .unwrap_or_else(|| src.to_owned());
        let reference =
            appearance::user_reader_background_reference(&stable).ok_or("无效阅读背景")?;
        if !resources
            .cache
            .contains(&reference)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("阅读背景文件不存在".into());
        }
        document["reader"]["backgroundImageSrc"] = json!(reference);
    }
    let mut config: crate::settings::SettingsDocument =
        serde_json::from_value(document.clone()).map_err(|error| error.to_string())?;
    let reader = &config.reader;
    if ![15, 30, 60, 120].contains(&config.source_http_timeout_seconds)
        || !(12.0..=36.0).contains(&reader.font_size_px)
        || !(1.2..=2.8).contains(&reader.line_height)
        || !(1..=20).contains(&reader.preload_count)
        || ![10, 15, 30].contains(&reader.audio_skip_seconds)
        || !(0.5..=3.0).contains(&reader.video_playback_rate)
        || !matches!(reader.tts_engine.as_str(), "system" | "http")
    {
        return Err("设置取值超出允许范围".into());
    }
    if reader.http_tts_config_id.as_ref().is_some_and(|id| {
        id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    }) {
        return Err("无效 HTTP TTS 配置标识".into());
    }
    crate::discovery::home_config::validate_home_document(&config.home)?;
    let current = resources
        .settings
        .read()
        .await
        .map_err(|error| error.to_string())?;
    if current.get("home") != document.get("home") {
        crate::discovery::home_config::validate_home_sources(database, &config.home).await?;
    }
    crate::discovery::home_config::normalize_home_document(&mut config.home);
    document["home"] = serde_json::to_value(&config.home).map_err(|error| error.to_string())?;
    if document["reader"]["httpTtsConfigId"].is_null() {
        document["reader"]
            .as_object_mut()
            .unwrap()
            .remove("httpTtsConfigId");
    }
    document["schemaVersion"] = json!(SCHEMA_VERSION);
    let background = config
        .reader
        .background_image_src
        .map(|reference| reference.to_string());
    let _write = WRITE_GATE.lock().await;
    let mut previous = None;
    let saved = resources
        .settings
        .update(|current| {
            previous = current["reader"]["backgroundImageSrc"]
                .as_str()
                .map(str::to_owned);
            // 备份时间由备份模块维护，前端提交不能覆盖。
            if let Some(timestamp) = current.get("lastBackupAtMs") {
                document["lastBackupAtMs"] = timestamp.clone();
            } else {
                document.as_object_mut().unwrap().remove("lastBackupAtMs");
            }
            crate::backup::service::normalize_webdav_settings(&current, &mut document)?;
            Ok(document)
        })
        .await
        .map_err(|error| error.to_string())?;
    appearance::cleanup_unreferenced_reader_background_asset(
        resources,
        previous,
        background.as_deref(),
    )
    .await;
    crate::source_http::set_network_timeout_seconds(config.source_http_timeout_seconds);
    Ok(saved)
}

pub(crate) fn preload_count(settings: Option<Value>) -> usize {
    settings
        .and_then(|value| value["reader"]["preloadCount"].as_u64())
        .filter(|count| (1..=20).contains(count))
        .map(|count| count as usize)
        .unwrap_or_else(|| ReaderDefaults::default().preload_count)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    settings: Value,
    request_id: u64,
    resources: State<'_, std::sync::Arc<Resources>>,
    database: State<'_, crate::database::Database>,
) -> Result<Value, String> {
    let resource = save(&resources, &database, settings).await?;
    let result = json!({ "requestId": request_id, "resource": resource });
    let _ = app.emit("settings-updated", result.clone());
    Ok(result)
}
