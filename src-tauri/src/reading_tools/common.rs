//! 阅读工具共用的 JSON 文档校验与资源初始化。

use crate::models::CURRENT_SCHEMA_VERSION;

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
