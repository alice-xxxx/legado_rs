//! 维护搜索历史的读取、去重和清理。
//! Search history is updated by Rust in normalized tables and queried by Vue.

use serde::{Deserialize, Serialize};

use crate::resources::{ResourceRef, ResourceStore};

pub const MAX_HISTORY_ENTRIES: usize = 100;
const MAX_PROCESSED_SEARCH_IDS: usize = 256;
const MAX_QUERY_CHARS: usize = 512;
const MAX_QUERY_BYTES: usize = 2048;
const MAX_SEARCH_ID_BYTES: usize = 128;

/// Browser-consumable search history. `processedSearchIds` is a bounded
/// de-duplication window for retries of the same search operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchHistoryDocument {
    pub entries: Vec<SearchHistoryEntry>,
    pub processed_search_ids: Vec<String>,
}

impl Default for SearchHistoryDocument {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            processed_search_ids: Vec::new(),
        }
    }
}

/// One normalized query with its most recent display spelling and frequency.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchHistoryEntry {
    pub query: String,
    pub normalized_query: String,
    pub usage: u64,
    pub first_use_time_ms: u64,
    pub last_use_time_ms: u64,
}

/// Search history is one section of the shared `history.json` resource.
pub fn resource_ref(store: &ResourceStore) -> Result<ResourceRef, String> {
    Ok(store.history_ref())
}

/// Read the typed document, creating the default JSON resource on first use.
pub async fn load(store: &ResourceStore) -> Result<SearchHistoryDocument, String> {
    let db=store.database().clone();
    let value=tokio::task::spawn_blocking(move ||db.read_search_history()).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    let document:SearchHistoryDocument=serde_json::from_value(value).map_err(|e|e.to_string())?;
    validate_typed_document(&document)?;
    Ok(document)
}

/// 统一搜索入口自动记录历史；每个 search_id 只计一次。
/// 同一关键词规范化后合并，由 Rust 更新次数和最近使用时间。
pub async fn record_search(
    store: &ResourceStore,
    search_id: &str,
    query: &str,
    now_ms: u64,
) -> Result<ResourceRef, String> {
    let search_id = validate_search_id(search_id)?;
    let query = validate_query(query)?;
    let normalized_query = normalize_query(&query);
    let reference = resource_ref(store)?;

    let db=store.database().clone();
    tokio::task::spawn_blocking(move ||db.record_search(search_id,query,normalized_query,now_ms))
        .await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;

    Ok(reference)
}

pub fn validate_search_history(document: &SearchHistoryDocument) -> Result<(), String> {
    validate_typed_document(document)
}

fn validate_typed_document(document: &SearchHistoryDocument) -> Result<(), String> {
    if document.entries.len() > MAX_HISTORY_ENTRIES {
        return Err("Search history contains too many entries".into());
    }
    if document.processed_search_ids.len() > MAX_PROCESSED_SEARCH_IDS {
        return Err("Search history de-duplication window is too large".into());
    }

    let mut query_keys = std::collections::HashSet::with_capacity(document.entries.len());
    for entry in &document.entries {
        let query = validate_query(&entry.query)?;
        if query != entry.query {
            return Err("Search history query has surrounding whitespace".into());
        }
        if normalize_query(&query) != entry.normalized_query {
            return Err("Search history query normalization does not match".into());
        }
        if !query_keys.insert(entry.normalized_query.as_str()) {
            return Err("Search history contains duplicate normalized queries".into());
        }
        if entry.usage == 0 {
            return Err("Search history usage must be at least one".into());
        }
        if entry.first_use_time_ms > entry.last_use_time_ms {
            return Err("Search history timestamps are out of order".into());
        }
    }

    let mut ids = std::collections::HashSet::with_capacity(document.processed_search_ids.len());
    for search_id in &document.processed_search_ids {
        let validated = validate_search_id(search_id)?;
        if validated.as_str() != search_id {
            return Err("Search history contains a noncanonical search ID".into());
        }
        if !ids.insert(validated) {
            return Err("Search history contains duplicate processed search IDs".into());
        }
    }
    Ok(())
}

fn validate_query(query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > MAX_QUERY_BYTES || query.chars().count() > MAX_QUERY_CHARS
    {
        return Err("Search query is empty or exceeds the supported size".into());
    }
    Ok(query.to_owned())
}

fn normalize_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn validate_search_id(search_id: &str) -> Result<String, String> {
    let search_id = search_id.trim();
    if search_id.is_empty()
        || search_id.len() > MAX_SEARCH_ID_BYTES
        || search_id.chars().any(char::is_control)
    {
        return Err("Search ID is empty or invalid".into());
    }
    Ok(search_id.to_owned())
}
