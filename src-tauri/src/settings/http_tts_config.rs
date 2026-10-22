//! 校验并保存 HTTP 朗读配置及其安全元数据。

use serde_json::{json, Value};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_NAME_CHARS: usize = 256;
const MAX_URL_BYTES: usize = 64 * 1024;

/// 解析本地或远程导入的单个 HttpTTS 对象或对象数组，调用方在全部校验通过后再写入。
pub fn import_configs(config_json: &str) -> Result<Vec<(Value, Value)>, String> {
    if config_json.len() > MAX_CONFIG_BYTES {
        return Err("HttpTTS configuration exceeds the 1 MiB limit".to_owned());
    }
    let parsed: Value = serde_json::from_str(config_json)
        .map_err(|error| format!("Invalid HttpTTS configuration: {error}"))?;
    let configs = match parsed {
        Value::Array(configs) => configs,
        config => vec![config],
    };
    if configs.is_empty() {
        return Err("HttpTTS import must contain at least one configuration".to_owned());
    }
    configs.into_iter().map(imported_entry).collect()
}

fn validate_raw_config(config: &Value) -> Result<String, String> {
    let object = config
        .as_object()
        .ok_or_else(|| "HttpTTS configuration must be one JSON object".to_owned())?;
    let encoded = serde_json::to_vec(config)
        .map_err(|error| format!("Cannot encode HttpTTS configuration: {error}"))?;
    if encoded.len() > MAX_CONFIG_BYTES {
        return Err("HttpTTS configuration exceeds the 1 MiB limit".to_owned());
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty() && name.chars().count() <= MAX_NAME_CHARS)
        .ok_or_else(|| "HttpTTS name is required and must be at most 256 characters".to_owned())?;
    if object
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty() && url.len() <= MAX_URL_BYTES)
        .is_none()
    {
        return Err("HttpTTS URL is required and must be at most 64 KiB".to_owned());
    }
    if object.get("id").is_some_and(|id| id.as_i64().is_none()) {
        return Err("HttpTTS ID must be a signed 64-bit integer when present".to_owned());
    }
    Ok(name.to_owned())
}

fn imported_entry(config: Value) -> Result<(Value, Value), String> {
    let name = validate_raw_config(&config)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let metadata = json!({ "id": id, "name": name, "enabled": true });
    let entry = json!({
        "id": id,
        "name": name,
        "enabled": true,
        "config": config,
    });
    Ok((metadata, entry))
}
