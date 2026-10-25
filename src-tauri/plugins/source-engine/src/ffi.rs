//! 原生边界共用的结果编码；不选择平台、不转发业务操作。
use serde::Serialize;
pub(crate) fn response<T: Serialize>(result: crate::Result<T>) -> String {
    match result {
        Ok(value) => serde_json::json!({"ok":true,"value":value}).to_string(),
        Err(error) => serde_json::json!({"ok":false,"error":error}).to_string(),
    }
}
