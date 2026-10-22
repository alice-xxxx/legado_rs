//! 阅读工具共用的 JSON 文档校验与资源初始化。

use crate::models::CURRENT_SCHEMA_VERSION;

pub(super) use crate::support::validate_id;

pub(super) fn current_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}
