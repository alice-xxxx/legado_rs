//! 书源目录：结构校验、快照查询和写入后的缓存失效。
use crate::database::Database;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
};
use tokio::sync::Mutex;

pub(crate) const MAX_SOURCE_IMPORT_BODY_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct SourceCatalog {
    database: Database,
    snapshot: Mutex<Option<Arc<SourceIndex>>>,
    revisions: Mutex<HashMap<String, u64>>,
    pub(crate) publication: Arc<Mutex<()>>,
}

impl SourceCatalog {
    pub(crate) fn new(database: Database) -> Self {
        Self {
            database,
            snapshot: Mutex::new(None),
            revisions: Mutex::new(HashMap::new()),
            publication: Arc::new(Mutex::new(())),
        }
    }
    pub(crate) async fn get(&self, id: &str) -> Result<SourceRecord, String> {
        self.lookup(id)
            .await?
            .ok_or_else(|| format!("Book source '{id}' is no longer imported"))
    }
    pub(crate) async fn lookup(&self, id: &str) -> Result<Option<SourceRecord>, String> {
        let index = self.index().await?;
        Ok(index
            .by_id
            .get(id)
            .map(|position| index.records[*position].clone()))
    }
    pub(crate) async fn all(&self) -> Result<Arc<Vec<SourceRecord>>, String> {
        Ok(self.index().await?.records.clone())
    }
    async fn index(&self) -> Result<Arc<SourceIndex>, String> {
        let mut cached = self.snapshot.lock().await;
        if let Some(index) = cached.as_ref() {
            return Ok(index.clone());
        }
        let database = self.database.clone();
        let value = tokio::task::spawn_blocking(move || database.sources())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let records = value.into_iter().map(|source| {
            Ok(SourceRecord {
                id:source.id,
                name:source.definition.book_source_name.clone(),
                group:source.definition.book_source_group.clone(),
                enabled:source.definition.enabled.unwrap_or(true),
                source:serde_json::to_value(source.definition).map_err(|error|error.to_string())?,
            })
        }).collect::<Result<Vec<_>,String>>()?;
        let index = Arc::new(SourceIndex::new(records));
        *cached = Some(index.clone());
        Ok(index)
    }
    pub(crate) async fn invalidate(&self) {
        *self.snapshot.lock().await = None;
        for revision in self.revisions.lock().await.values_mut() {
            *revision = revision.saturating_add(1);
        }
    }
    pub(crate) async fn revision(&self, id: &str) -> u64 {
        *self.revisions.lock().await.entry(id.into()).or_default()
    }

    pub(crate) async fn rss_snapshot(
        &self,
        expected: &SourceRecord,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        let guard = self.publication.clone().lock_owned().await;
        let current = self
            .lookup(&expected.id)
            .await?
            .ok_or_else(|| "RSS source was removed while the request was running".to_owned())?;
        if !current.enabled
            || current.source != expected.source
            || !crate::rss::is_rss_source(&current.source)
        {
            return Err("RSS source changed while the request was running".to_owned());
        }
        Ok(guard)
    }
}

struct SourceIndex {
    records: Arc<Vec<SourceRecord>>,
    by_id: HashMap<String, usize>,
}

impl SourceIndex {
    fn new(records: Vec<SourceRecord>) -> Self {
        let by_id = records
            .iter()
            .enumerate()
            .map(|(index, source)| (source.id.clone(), index))
            .collect();
        Self {
            records: Arc::new(records),
            by_id,
        }
    }
}

/// 阅读类型只在这里映射一次，不能从书籍题材或正文内容猜测。
pub(crate) fn content_type(source: &Value) -> &'static str {
    match source["bookSourceType"].as_i64() {
        Some(1) => "audio",
        Some(2) => "image",
        Some(4) => "video",
        _ => "text",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(rename = "sourceGroup", skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) source: Value,
}

#[cfg(test)]
#[path = "tests/catalog.rs"]
mod tests;
