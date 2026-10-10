//! 校验并保存 HTTP 朗读配置及其安全元数据。
use std::collections::HashSet;

use serde::Deserialize;
use serde_json::{json, Value};

const DOCUMENT_SCHEMA_VERSION: u64 = 1;
pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_NAME_CHARS: usize = 256;
const MAX_URL_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HttpTtsConfigImportSource {
    Local,
    Url { url: String },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum HttpTtsConfigOperation {
    Import(HttpTtsConfigImportSource),
    Mutation(HttpTtsConfigMutation),
}

/// 解析单条 HttpTTS 配置并分配应用内部 ID，同时保留未识别的原始字段。
pub fn import_config(config_json: &str) -> Result<(Value, Value), String> {
    if config_json.len() > MAX_CONFIG_BYTES {
        return Err("HttpTTS configuration exceeds the 1 MiB limit".to_owned());
    }
    let config: Value = serde_json::from_str(config_json)
        .map_err(|error| format!("Invalid HttpTTS configuration: {error}"))?;
    imported_entry(config)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HttpTtsConfigMutation {
    Save {
        id: Option<String>,
        json: String,
    },
    Delete {
        #[serde(rename = "configIds")]
        config_ids: Vec<String>,
    },
}

/// Apply the only supported writes to the public HTTP TTS document.
/// Saving without an ID accepts one object or an imported object array; saving
/// with an ID replaces exactly that object's complete JSON payload.
pub fn apply_mutation(
    document: &Value,
    mutation: HttpTtsConfigMutation,
) -> Result<(Value, Value), String> {
    match mutation {
        HttpTtsConfigMutation::Save {
            id: Some(id),
            json,
        } => {
            let (mut metadata, mut entry) = import_config(&json)?;
            if !document_configs(document)?
                .iter()
                .any(|existing| existing["id"].as_str() == Some(id.as_str()))
            {
                return Err("HttpTTS configuration was not found".to_owned());
            }
            entry["id"] = Value::String(id.clone());
            metadata["id"] = Value::String(id);
            let updated = upsert_config(document, entry)?;
            Ok((updated, json!({ "items": [metadata] })))
        }
        HttpTtsConfigMutation::Save {
            id: None,
            json,
        } => {
            let entries = import_configs(&json)?;
            let mut updated = normalized_document(document);
            let mut items = Vec::with_capacity(entries.len());
            for (metadata, entry) in entries {
                updated = upsert_config(&updated, entry)?;
                items.push(metadata);
            }
            Ok((updated, json!({ "items": items })))
        }
        HttpTtsConfigMutation::Delete { config_ids } => {
            let (updated, deleted) = delete_configs(document, &config_ids)?;
            Ok((updated, json!({ "items": [], "deleted": deleted })))
        }
    }
}

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

pub fn list_configs(document: &Value) -> Result<Value, String> {
    let configs = document_configs(document)?;
    let items = configs
        .iter()
        .map(|entry| {
            json!({
                "id": entry["id"],
                "name": entry["name"],
                "enabled": true,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "items": items }))
}

pub fn validate_document(document: &Value) -> Result<(), String> {
    if document.get("schemaVersion").and_then(Value::as_u64) != Some(DOCUMENT_SCHEMA_VERSION) {
        return Err("HttpTTS configuration document has an unsupported schema".to_owned());
    }
    document_configs(document).map(|_| ())
}

pub fn find_config(document: &Value, id: &str) -> Result<Value, String> {
    let configs = document_configs(document)?;
    configs
        .iter()
        .find(|entry| entry["id"].as_str() == Some(id))
        .map(|entry| entry["config"].clone())
        .ok_or_else(|| "HttpTTS configuration was not found".to_owned())
}

pub fn delete_configs(document: &Value, ids: &[String]) -> Result<(Value, bool), String> {
    if ids.is_empty() {
        return Err("HttpTTS delete selection is empty".to_owned());
    }
    document_configs(document)?;
    let mut updated = normalized_document(document);
    let configs = updated
        .get_mut("configs")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "HttpTTS configuration document is invalid".to_owned())?;
    let old_len = configs.len();
    let ids = ids.iter().map(String::as_str).collect::<HashSet<_>>();
    configs.retain(|entry| {
        !entry["id"]
            .as_str()
            .is_some_and(|id| ids.contains(id))
    });
    let removed = configs.len() != old_len;
    Ok((updated, removed))
}

pub fn upsert_config(document: &Value, entry: Value) -> Result<Value, String> {
    document_configs(document)?;
    validate_entry(&entry)?;

    let id = entry["id"]
        .as_str()
        .ok_or_else(|| "HttpTTS configuration ID is invalid".to_owned())?
        .to_owned();
    let mut updated = normalized_document(document);
    let configs = updated
        .get_mut("configs")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "HttpTTS configuration document is invalid".to_owned())?;
    if let Some(index) = configs
        .iter()
        .position(|existing| existing["id"].as_str() == Some(id.as_str()))
    {
        configs[index] = entry;
    } else {
        configs.push(entry);
    }
    Ok(updated)
}

pub fn empty_document() -> Value {
    json!({
        "schemaVersion": DOCUMENT_SCHEMA_VERSION,
        "configs": [],
    })
}

fn document_configs(document: &Value) -> Result<&[Value], String> {
    if document.is_null() {
        return Ok(&[]);
    }
    if document.get("schemaVersion").and_then(Value::as_u64) != Some(DOCUMENT_SCHEMA_VERSION) {
        return Err("HttpTTS configuration document has an unsupported schema".to_owned());
    }
    let configs = document
        .get("configs")
        .and_then(Value::as_array)
        .ok_or_else(|| "HttpTTS configuration document is invalid".to_owned())?;
    let mut ids = HashSet::with_capacity(configs.len());
    for entry in configs {
        validate_entry(entry)?;
        let id = entry["id"]
            .as_str()
            .ok_or_else(|| "HttpTTS configuration ID is invalid".to_owned())?;
        if !ids.insert(id) {
            return Err("HttpTTS configuration document contains a duplicate ID".to_owned());
        }
    }
    Ok(configs)
}

fn validate_entry(entry: &Value) -> Result<(), String> {
    let id = entry
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "HttpTTS configuration ID is invalid".to_owned())?;
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("HttpTTS configuration ID is invalid".to_owned());
    }
    if entry.get("enabled") != Some(&Value::Bool(true)) {
        return Err("HttpTTS configuration enabled state is invalid".to_owned());
    }
    let config = entry
        .get("config")
        .ok_or_else(|| "HttpTTS configuration payload is missing".to_owned())?;
    let config_name = validate_raw_config(config)?;
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "HttpTTS configuration name is invalid".to_owned())?;
    if name != config_name {
        return Err("HttpTTS configuration name does not match its payload".to_owned());
    }
    Ok(())
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

fn normalized_document(document: &Value) -> Value {
    if document.is_null() {
        empty_document()
    } else {
        document.clone()
    }
}
