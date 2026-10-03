//! JSON-backed search history for user initiated book searches.
//!
//! The WebView consumes this processed document as a normal resource. It holds
//! user query text and usage metadata only; source definitions and search rules
//! stay in Rust's private source store and are never copied here.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::resources::{ResourceRef, ResourceStore};

const CURRENT_SCHEMA_VERSION: u32 = 1;
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
    pub schema_version: u32,
    pub entries: Vec<SearchHistoryEntry>,
    pub processed_search_ids: Vec<String>,
}

impl Default for SearchHistoryDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
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

/// Stable browser resource location: `resource://reading/search-history.json`.
pub fn resource_ref(store: &ResourceStore) -> Result<ResourceRef, String> {
    store
        .reading_ref("search-history")
        .map_err(|error| error.to_string())
}

/// Read the typed document, creating the default JSON resource on first use.
pub async fn load(store: &ResourceStore) -> Result<SearchHistoryDocument, String> {
    let reference = resource_ref(store)?;
    let value = store
        .update_json_ref(&reference, |current| {
            let document = decode_or_default(current)?;
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    decode_document(value)
}

/// Record a user initiated search exactly once per `search_id` within the
/// bounded retry window. Distinct searches for the same normalized keyword
/// update its usage count and last-use time. Internal source-switch searches
/// must not call this helper.
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

    store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_or_default(current)?;
            if document
                .processed_search_ids
                .iter()
                .any(|processed| processed == &search_id)
            {
                return serde_json::to_value(document).map_err(|error| error.to_string());
            }

            document.processed_search_ids.push(search_id);
            if document.processed_search_ids.len() > MAX_PROCESSED_SEARCH_IDS {
                let discard = document.processed_search_ids.len() - MAX_PROCESSED_SEARCH_IDS;
                document.processed_search_ids.drain(0..discard);
            }

            if let Some(entry) = document
                .entries
                .iter_mut()
                .find(|entry| entry.normalized_query == normalized_query)
            {
                entry.query = query;
                entry.usage = entry.usage.saturating_add(1);
                entry.last_use_time_ms = entry.last_use_time_ms.max(now_ms);
                entry.first_use_time_ms = entry.first_use_time_ms.min(now_ms);
            } else {
                document.entries.push(SearchHistoryEntry {
                    query,
                    normalized_query,
                    usage: 1,
                    first_use_time_ms: now_ms,
                    last_use_time_ms: now_ms,
                });
            }

            sort_by_recency(&mut document.entries);
            document.entries.truncate(MAX_HISTORY_ENTRIES);
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;

    Ok(reference)
}

/// Delete the history entry matching a query after normalization. Repeating
/// this operation is safe and leaves the de-duplication window intact.
pub async fn delete_query(store: &ResourceStore, query: &str) -> Result<ResourceRef, String> {
    let query = validate_query(query)?;
    let normalized_query = normalize_query(&query);
    let reference = resource_ref(store)?;

    store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_or_default(current)?;
            document
                .entries
                .retain(|entry| entry.normalized_query != normalized_query);
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;

    Ok(reference)
}

/// Clear all visible history entries. Recent operation IDs remain in the
/// document so a delayed IPC retry cannot recreate a just-cleared entry.
pub async fn clear(store: &ResourceStore) -> Result<ResourceRef, String> {
    let reference = resource_ref(store)?;
    store
        .update_json_ref(&reference, |current| {
            let mut document = decode_or_default(current)?;
            document.entries.clear();
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Rank common searches by usage, breaking ties by recency. The Vue layer can
/// also derive this ordering directly from `entries` when it needs a different
/// presentation.
pub fn most_used(document: &SearchHistoryDocument, limit: usize) -> Vec<SearchHistoryEntry> {
    let mut entries = document.entries.clone();
    entries.sort_by(|left, right| {
        right
            .usage
            .cmp(&left.usage)
            .then_with(|| right.last_use_time_ms.cmp(&left.last_use_time_ms))
            .then_with(|| left.normalized_query.cmp(&right.normalized_query))
    });
    entries.truncate(limit);
    entries
}

/// Strictly decode and validate the supported schema. Unknown/future versions
/// are rejected instead of being rewritten as this version.
pub fn decode_document(value: Value) -> Result<SearchHistoryDocument, String> {
    let document: SearchHistoryDocument = serde_json::from_value(value)
        .map_err(|error| format!("Invalid search history: {error}"))?;
    validate_typed_document(&document)?;
    Ok(document)
}

/// Validate a JSON document for backup/restore callers without consulting live
/// sources or interpreting any source rules.
pub fn validate_document(value: &Value) -> Result<(), String> {
    decode_document(value.clone()).map(|_| ())
}

fn decode_or_default(value: Value) -> Result<SearchHistoryDocument, String> {
    if value.is_null() {
        Ok(SearchHistoryDocument::default())
    } else {
        decode_document(value)
    }
}

fn validate_typed_document(document: &SearchHistoryDocument) -> Result<(), String> {
    if document.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported search history schema version: {}",
            document.schema_version
        ));
    }
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

fn sort_by_recency(entries: &mut [SearchHistoryEntry]) {
    entries.sort_by(|left, right| {
        right
            .last_use_time_ms
            .cmp(&left.last_use_time_ms)
            .then_with(|| right.usage.cmp(&left.usage))
            .then_with(|| left.normalized_query.cmp(&right.normalized_query))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        clear, decode_document, delete_query, load, most_used, record_search, validate_document,
        MAX_HISTORY_ENTRIES,
    };
    use crate::resources::ResourceStore;

    async fn open_store() -> (tempfile::TempDir, ResourceStore) {
        let temp = tempdir().expect("temporary app data root");
        let store = ResourceStore::open(temp.path()).expect("open resource store");
        (temp, store)
    }

    #[tokio::test]
    async fn history_is_utf8_deduplicated_counted_and_persistent_after_reopen() {
        let (temp, store) = open_store().await;
        let reference = record_search(&store, "search-a", "  没钱  修什么仙  ", 100)
            .await
            .expect("record first search");
        assert_eq!(reference.as_str(), "resource://reading/search-history.json");

        record_search(&store, "search-a", "没钱 修什么仙", 101)
            .await
            .expect("retry is idempotent");
        record_search(&store, "search-b", " 没钱 修什么仙 ", 120)
            .await
            .expect("normalized query increments usage");
        record_search(&store, "search-c", "三体", 130)
            .await
            .expect("record a second keyword");

        let document = load(&store).await.expect("load history");
        assert_eq!(document.entries.len(), 2);
        let common = most_used(&document, 1);
        assert_eq!(common[0].query, "没钱 修什么仙");
        assert_eq!(common[0].usage, 2);
        assert_eq!(common[0].first_use_time_ms, 100);
        assert_eq!(common[0].last_use_time_ms, 120);

        let reopened = ResourceStore::open(temp.path()).expect("reopen resource store");
        let persisted = load(&reopened).await.expect("read after restart");
        assert_eq!(persisted, document);
        assert_eq!(persisted.entries[0].query, "三体");
    }

    #[tokio::test]
    async fn delete_and_clear_are_idempotent_and_do_not_restore_duplicate_retries() {
        let (_temp, store) = open_store().await;
        record_search(&store, "search-a", "Alpha", 10)
            .await
            .expect("record alpha");
        record_search(&store, "search-b", "Beta", 20)
            .await
            .expect("record beta");

        delete_query(&store, " alpha ").await.expect("delete alpha");
        delete_query(&store, "ALPHA").await.expect("repeat delete");
        record_search(&store, "search-a", "Alpha", 30)
            .await
            .expect("delayed retry is deduplicated");
        assert_eq!(load(&store).await.expect("load").entries.len(), 1);

        clear(&store).await.expect("clear entries");
        clear(&store).await.expect("repeat clear");
        record_search(&store, "search-b", "Beta", 40)
            .await
            .expect("delayed retry after clear is deduplicated");
        let document = load(&store).await.expect("load cleared history");
        assert!(document.entries.is_empty());
        assert_eq!(document.processed_search_ids.len(), 2);
    }

    #[tokio::test]
    async fn concurrent_retries_and_same_query_updates_are_serialized() {
        let (_temp, store) = open_store().await;
        let store = Arc::new(store);
        let mut workers = Vec::new();
        for index in 0..24u64 {
            let store = Arc::clone(&store);
            workers.push(tokio::spawn(async move {
                record_search(&store, &format!("search-{index}"), "并发查询", index + 1).await
            }));
        }
        for worker in workers {
            worker.await.expect("worker join").expect("record query");
        }

        // Every operation ID is unique, so all actual searches count once.
        let document = load(&store).await.expect("load concurrent history");
        assert_eq!(document.entries.len(), 1);
        assert_eq!(document.entries[0].usage, 24);
        assert_eq!(document.processed_search_ids.len(), 24);

        let mut retries = Vec::new();
        for _ in 0..20 {
            let store = Arc::clone(&store);
            retries.push(tokio::spawn(async move {
                record_search(&store, "same-retried-search", "并发查询", 30).await
            }));
        }
        for retry in retries {
            retry
                .await
                .expect("retry worker join")
                .expect("deduplicate retry");
        }
        let after_retries = load(&store).await.expect("load retry result");
        assert_eq!(after_retries.entries[0].usage, 25);
        assert_eq!(after_retries.processed_search_ids.len(), 25);
    }

    #[tokio::test]
    async fn caps_history_and_rejects_invalid_or_future_documents() {
        let (_temp, store) = open_store().await;
        for index in 0..125u64 {
            record_search(
                &store,
                &format!("search-{index}"),
                &format!("query-{index}"),
                index + 1,
            )
            .await
            .expect("record capped query");
        }
        let document = load(&store).await.expect("load capped history");
        assert_eq!(document.entries.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(document.entries[0].query, "query-124");
        assert_eq!(document.entries.last().unwrap().query, "query-25");

        assert!(decode_document(json!({
            "schemaVersion": 2,
            "entries": [],
            "processedSearchIds": []
        }))
        .is_err());
        assert!(decode_document(json!({
            "schemaVersion": 1,
            "entries": [{
                "query": "ＡＢＣ",
                "normalizedQuery": "abc",
                "usage": 1,
                "firstUseTimeMs": 1,
                "lastUseTimeMs": 1
            }],
            "processedSearchIds": []
        }))
        .is_err());
        assert!(decode_document(json!({
            "schemaVersion": 1,
            "entries": [],
            "processedSearchIds": [],
            "bookSource": {"rules": "must not be stored here"}
        }))
        .is_err());
        assert!(validate_document(&json!({"schemaVersion": 999})).is_err());
    }

    #[tokio::test]
    async fn bounds_the_idempotency_window_without_growing_the_resource_forever() {
        let (_temp, store) = open_store().await;
        for index in 0..300u64 {
            record_search(&store, &format!("window-{index}"), "same query", index + 1)
                .await
                .expect("record bounded operation ID");
        }

        let document = load(&store).await.expect("load bounded history");
        assert_eq!(document.entries[0].usage, 300);
        assert_eq!(document.processed_search_ids.len(), 256);
        assert_eq!(document.processed_search_ids.first().unwrap(), "window-44");
        assert_eq!(document.processed_search_ids.last().unwrap(), "window-299");
    }
}
