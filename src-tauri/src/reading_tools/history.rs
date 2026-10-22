//! 阅读会话与书签的固定输入类型；持久化和统计由 database 负责。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub chapter_id: String,
    pub chapter_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_title: Option<String>,
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub orphaned: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingSession {
    pub book_id: String,
    /// 一次阅读会话的稳定 ID；重试提交不会重复计时。
    pub session_id: String,
    pub duration_ms: u64,
    pub ended_at_ms: u64,
    /// 会话结束时的时区偏移，单位为分钟，UTC 以东为正。
    #[serde(default)]
    pub utc_offset_minutes: i32,
}
