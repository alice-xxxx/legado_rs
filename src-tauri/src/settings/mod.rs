//! 设置文件的读取、校验与保存；设置不属于业务数据库。

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) mod import;

use crate::{
    application::ApplicationService,
    models::{ReaderDefaults, ReaderTheme, CURRENT_SCHEMA_VERSION},
    support::{validate_integer_range, validate_number_range},
};
use serde_json::{json, Value};

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::{Emitter, State};

impl ApplicationService {
    pub async fn save_settings(&self, settings: Value) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let mut document = settings;
        if !document.is_object() {
            return Err("Settings must be a JSON object".into());
        }
        crate::models::upgrade_settings_document(&mut document);
        serde_json::from_value::<crate::models::SettingsDocument>(document.clone())
            .map_err(|error| format!("Invalid settings: {error}"))?;
        let home_value = document
            .get("home")
            .cloned()
            .ok_or_else(|| "Settings home is required".to_owned())?;
        let mut home: crate::models::HomeConfigDocument =
            serde_json::from_value(home_value.clone())
                .map_err(|error| format!("Invalid settings home: {error}"))?;
        crate::discovery::home_config::validate_home_document(&home)?;
        let settings_ref = self.store.settings_ref();
        let current = self
            .store
            .read_json_ref(&settings_ref)
            .await
            .map_err(|error| error.to_string())?;
        if current.get("home") != Some(&home_value) {
            crate::discovery::home_config::validate_home_sources(self, &home).await?;
        }
        crate::discovery::home_config::normalize_home_document(&mut home);
        document["home"] = serde_json::to_value(home)
            .map_err(|error| format!("Cannot encode settings home: {error}"))?;

        let source_http_timeout_seconds = match document.get("sourceHttpTimeoutSeconds") {
            None => 15,
            Some(value) => value
                .as_u64()
                .filter(|seconds| [15, 30, 60, 120].contains(seconds))
                .ok_or_else(|| {
                    "Source HTTP timeout must be 15, 30, 60, or 120 seconds".to_owned()
                })?,
        };
        document["sourceHttpTimeoutSeconds"] = json!(source_http_timeout_seconds);
        let reader = document
            .get("reader")
            .and_then(Value::as_object)
            .ok_or_else(|| "Settings reader must be an object".to_owned())?;
        if let Some(config_id) = reader.get("httpTtsConfigId") {
            match config_id {
                Value::Null => {}
                Value::String(value)
                    if !value.is_empty()
                        && value.len() <= 128
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte)) => {}
                _ => return Err("HTTP TTS configuration ID is invalid".to_owned()),
            }
        }
        if let Some(engine) = reader.get("ttsEngine") {
            match engine.as_str() {
                Some("system" | "http") => {}
                _ => return Err("TTS engine must be system or http".to_owned()),
            }
        }
        validate_number_range(reader, "fontSizePx", 12.0, 36.0)?;
        validate_number_range(reader, "lineHeight", 1.2, 2.8)?;
        validate_integer_range(reader, "preloadCount", 1, 20)?;
        if let Some(skip_seconds) = reader.get("audioSkipSeconds") {
            if skip_seconds
                .as_u64()
                .is_none_or(|seconds| ![10, 15, 30].contains(&seconds))
            {
                return Err("Audio skip seconds must be 10, 15, or 30".to_owned());
            }
        }
        if reader.contains_key("videoPlaybackRate") {
            validate_number_range(reader, "videoPlaybackRate", 0.5, 3.0)?;
        }
        let reader = document
            .get_mut("reader")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "Settings reader must be an object".to_owned())?;
        if reader.get("httpTtsConfigId").is_some_and(Value::is_null) {
            reader.remove("httpTtsConfigId");
        }
        if let Some(src) = reader.get("backgroundImageSrc").and_then(Value::as_str) {
            let raw = if let Some(path) = src.strip_prefix(self.server.base_url()) {
                format!("resource://{path}")
            } else {
                src.to_owned()
            };
            let reference = crate::resources::appearance::user_reader_background_reference(&raw)
                .ok_or_else(|| "Invalid reader background resource".to_owned())?;
            if !self
                .store
                .regular_file_exists(&reference)
                .await
                .map_err(|error| error.to_string())?
            {
                return Err("Reader background image is missing".into());
            }
            reader.insert("backgroundImageSrc".into(), json!(reference.as_str()));
        }
        let background = document["reader"]
            .get("backgroundImageSrc")
            .and_then(Value::as_str)
            .map(str::to_owned);
        document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
        let _config = self.webdav_backup_config_gate.lock().await;
        let previous_background = self
            .store
            .read_json_ref(&settings_ref)
            .await
            .map_err(|e| e.to_string())?["reader"]
            .get("backgroundImageSrc")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let saved = self
            .store
            .update_json_ref(&settings_ref, move |current| {
                // This value is maintained by successful picker exports. Never
                // accept a stale or client-provided copy from reader settings.
                let last_backup_at_ms = current.get("lastBackupAtMs").cloned();
                if let Some(last_backup_at_ms) = last_backup_at_ms {
                    document["lastBackupAtMs"] = last_backup_at_ms;
                } else if let Some(object) = document.as_object_mut() {
                    object.remove("lastBackupAtMs");
                }
                crate::backup::service::normalize_webdav_settings(&current, &mut document)?;
                Ok(document)
            })
            .await
            .map_err(|error| error.to_string())?;
        crate::resources::appearance::cleanup_unreferenced_reader_background_asset(
            &self.store,
            previous_background,
            background.as_deref(),
        )
        .await;
        crate::source_http::set_network_timeout_seconds(source_http_timeout_seconds);
        Ok(saved)
    }
}
pub(crate) fn configured_source_http_timeout_seconds(settings: Option<&Value>) -> u64 {
    settings
        .and_then(|settings| settings.get("sourceHttpTimeoutSeconds"))
        .and_then(Value::as_u64)
        .filter(|seconds| [15, 30, 60, 120].contains(seconds))
        .unwrap_or(15)
}

pub(crate) fn reader_defaults(settings: Option<Value>) -> ReaderDefaults {
    let Some(mut value) = settings else {
        return ReaderDefaults::default();
    };
    let mode = if value.get("theme").and_then(Value::as_str) == Some("dark") {
        "dark"
    } else {
        "light"
    };
    if value.get("reader").is_some() {
        value = value["reader"].clone();
    }
    let mut defaults = ReaderDefaults::default();
    if let Some(size) = value
        .get("fontSizePx")
        .or_else(|| value.get("fontSize"))
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
    {
        defaults.font_size_px = (size as f32).clamp(12.0, 36.0);
    }
    if let Some(line_height) = value
        .get("lineHeight")
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
    {
        defaults.line_height = (line_height as f32).clamp(1.2, 2.8);
    }
    if let Some(family) = value.get("fontFamily").and_then(Value::as_str) {
        defaults.font_family = match family {
            "serif" => "serif",
            "sans" => "sans-serif",
            "system" => "system-ui, sans-serif",
            "mono" => "monospace",
            _ => defaults.font_family.as_str(),
        }
        .to_owned();
    }
    if let Some(align) = value.get("textAlign").and_then(Value::as_str) {
        if matches!(align, "left" | "right" | "center" | "justify") {
            defaults.text_align = align.to_owned();
        }
    }
    if let Some(preload_count) = value.get("preloadCount").and_then(Value::as_u64) {
        if (1..=20).contains(&preload_count) {
            defaults.preload_count = preload_count as usize;
        }
    }
    if let Some(theme) = value["themes"].get(mode).and_then(Value::as_str) {
        defaults.theme = match theme {
            "sepia" => ReaderTheme::Sepia,
            "dark" => ReaderTheme::Dark,
            "white" => ReaderTheme::White,
            _ => ReaderTheme::Paper,
        };
    }
    let (text, background) = match defaults.theme {
        ReaderTheme::Paper => ("#3f3b34", "#f7f3e9"),
        ReaderTheme::Sepia => ("#4e4131", "#eee1c8"),
        ReaderTheme::Dark => ("#d1d0cb", "#17191c"),
        ReaderTheme::White => ("#252525", "#ffffff"),
    };
    defaults.text_color = text.to_owned();
    defaults.background_color = background.to_owned();
    defaults
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    settings: Value,
    request_id: u64,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.save_settings(settings).await?;
    let result = json!({ "requestId": request_id, "resource": resource });
    let _ = app.emit("settings-updated", result.clone());
    Ok(result)
}

/// 设置拥有的资源交给缓存清理保留，不让数据库解释设置文件。
pub(crate) fn protected_resources(root:&std::path::Path) -> Result<Vec<String>,crate::resources::ResourceError> {
    let Some(value)=crate::configuration::read(root,"settings.json")? else {return Ok(Vec::new());};
    let settings:crate::models::SettingsDocument=serde_json::from_value(value)
        .map_err(|error|crate::resources::ResourceError::new(error.to_string()))?;
    Ok(settings.reader.background_image_src.into_iter().map(|reference|reference.to_string()).collect())
}
