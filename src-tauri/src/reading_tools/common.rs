//! 阅读工具共用的 JSON 文档校验与资源初始化。

use serde::Serialize;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    models::CURRENT_SCHEMA_VERSION,
    resources::{ResourceRef, ResourceStore},
};
pub(super) async fn ensure_document<T: Serialize>(
    store: &ResourceStore,
    reference: ResourceRef,
    default: T,
) -> Result<ResourceRef, String> {
    let default = serde_json::to_value(default).map_err(|error| error.to_string())?;
    store
        .update_json_ref(&reference, move |value| match value {
            Value::Null => Ok(default),
            value => Ok(value),
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

pub(super) fn ensure_object(value: &mut Value, document_name: &str) -> Result<(), String> {
    if value.is_null() {
        *value = json!({});
    }
    if !value.is_object() {
        return Err(format!("{document_name} JSON must be an object"));
    }
    Ok(())
}

pub(super) fn validate_id(id: &str, field: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("Invalid {field}"));
    }
    Ok(())
}

pub(super) fn current_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
