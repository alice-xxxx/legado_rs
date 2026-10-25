use crate::database::Database;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{watch, Mutex, OwnedMutexGuard};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceRecord {
    pub id: String,
    pub name: String,
    #[serde(rename = "sourceGroup", skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub enabled: bool,
    pub source: Value,
}
struct Index {
    records: Arc<Vec<SourceRecord>>,
    positions: HashMap<String, usize>,
}
/// 快照、版本和发布屏障只在源模块内维护。
pub(crate) struct Catalog {
    database: Database,
    snapshot: Mutex<Option<Arc<Index>>>,
    publication: Arc<Mutex<()>>,
    revision: watch::Sender<u64>,
}
impl Catalog {
    pub fn new(database: Database) -> Self {
        Self {
            database,
            snapshot: Mutex::new(None),
            publication: Arc::new(Mutex::new(())),
            revision: watch::channel(0).0,
        }
    }
    pub fn version(&self) -> u64 {
        *self.revision.borrow()
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }
    pub async fn write_guard(&self) -> OwnedMutexGuard<()> {
        self.publication.clone().lock_owned().await
    }
    async fn index(&self) -> Result<Arc<Index>, String> {
        let mut cache = self.snapshot.lock().await;
        if let Some(index) = cache.as_ref() {
            return Ok(index.clone());
        }
        let database = self.database.clone();
        let stored = tokio::task::spawn_blocking(move || database.sources())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let mut records = Vec::with_capacity(stored.len());
        for stored in stored {
            let definition = stored.definition;
            records.push(SourceRecord {
                id: stored.id,
                name: definition.book_source_name.clone(),
                group: definition.book_source_group.clone(),
                enabled: definition.enabled.unwrap_or(true),
                source: serde_json::to_value(definition).map_err(|e| e.to_string())?,
            });
        }
        let positions = records
            .iter()
            .enumerate()
            .map(|(i, row)| (row.id.clone(), i))
            .collect();
        let index = Arc::new(Index {
            records: Arc::new(records),
            positions,
        });
        *cache = Some(index.clone());
        Ok(index)
    }
    pub async fn lookup(&self, id: &str) -> Result<Option<SourceRecord>, String> {
        let index = self.index().await?;
        Ok(index.positions.get(id).map(|i| index.records[*i].clone()))
    }
    pub async fn get(&self, id: &str) -> Result<SourceRecord, String> {
        self.lookup(id).await?.ok_or_else(|| "源已不存在".into())
    }
    pub async fn all(&self) -> Result<Arc<Vec<SourceRecord>>, String> {
        Ok(self.index().await?.records.clone())
    }
    /// 数据库提交或恢复成功后才失效，加载和失效共用同一把快照锁。
    pub async fn invalidate(&self) {
        *self.snapshot.lock().await = None;
        self.revision
            .send_modify(|value| *value = value.saturating_add(1));
    }
    pub async fn current(
        &self,
        expected: &SourceRecord,
        revision: u64,
    ) -> Result<OwnedMutexGuard<()>, String> {
        let guard = self.write_guard().await;
        if self.version() != revision || self.lookup(&expected.id).await?.as_ref() != Some(expected)
        {
            return Err("源已变化，请重新操作".into());
        }
        Ok(guard)
    }
    pub async fn rss_snapshot(
        &self,
        expected: &SourceRecord,
    ) -> Result<OwnedMutexGuard<()>, String> {
        let guard = self.write_guard().await;
        if !expected.enabled
            || !tauri_plugin_source_engine::source::is_rss_source(&expected.source)
            || self.lookup(&expected.id).await?.as_ref() != Some(expected)
        {
            return Err("订阅源已变化，请重新操作".into());
        }
        Ok(guard)
    }
}
pub(crate) fn content_type(source: &Value) -> &'static str {
    match source["bookSourceType"].as_i64() {
        Some(1) => "audio",
        Some(2) => "image",
        Some(4) => "video",
        _ => "text",
    }
}
