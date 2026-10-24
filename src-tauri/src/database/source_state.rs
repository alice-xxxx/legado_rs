//! Application-owned source cache, cookies and script variables.
use super::{Database, WriteMode, db, params};
use std::{path::Path, sync::Arc};
use tauri_plugin_source_engine::storage::{KeyFilter, Namespace, StorageHost};

pub(crate) struct SourceState(Database);

impl SourceState {
    pub(crate) fn new(directory: &Path) -> Self {
        let root = if directory
            .file_name()
            .is_some_and(|name| name == "source-engine")
        {
            directory.parent().unwrap_or(directory)
        } else {
            directory
        };
        Self(Database {
            root: Arc::new(root.to_owned()),
        })
    }
}

impl StorageHost for SourceState {
    fn get(&self, namespace: Namespace, key: &str) -> Result<Option<String>, String> {
        if !self.0.file_path().is_file() {
            return Ok(None);
        }
        let now = crate::support::now_ms().min(i64::MAX as u64) as i64;
        db::read(&self.0, |s| {
            Ok(s.read(
                "SELECT value FROM source_kv WHERE namespace=?1 AND key=?2 AND (expires_at_ms IS NULL OR expires_at_ms>?3)",
                params![namespace.as_str(), key, now], |r| r.get(0),
            )?.pop())
        }).map_err(|error| error.to_string())
    }

    fn put(
        &self,
        namespace: Namespace,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<(), String> {
        let (key, value) = (key.to_owned(), value.to_owned());
        let now = crate::support::now_ms().min(i64::MAX as u64) as i64;
        let expiry = ttl_seconds.filter(|seconds| *seconds > 0).map(|seconds| {
            now.saturating_add(seconds.saturating_mul(1000).min(i64::MAX as u64) as i64)
        });
        db::write(&self.0, WriteMode::Transaction, move |s| {
            s.write(
                "INSERT INTO source_kv(namespace,key,value,expires_at_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,expires_at_ms=excluded.expires_at_ms WHERE source_kv.value!=excluded.value OR source_kv.expires_at_ms IS NOT excluded.expires_at_ms",
                params![namespace.as_str(), key, value, expiry],
            )?;
            Ok(())
        }).and_then(|pending| pending.wait()).map_err(|error| error.to_string())
    }

    fn remove(&self, namespace: Namespace, filter: KeyFilter) -> Result<Vec<String>, String> {
        let (mode, patterns) = match filter {
            KeyFilter::Key(key) => (1, vec![key]),
            KeyFilter::Prefixes(prefixes) => (2, prefixes),
            KeyFilter::Contains(text) => (3, vec![text]),
            KeyFilter::All => (0, vec![String::new()]),
        };
        db::write(&self.0, WriteMode::Transaction, move |s| {
            let mut deleted = Vec::new();
            for pattern in patterns {
                deleted.extend(s.read(
                    "DELETE FROM source_kv WHERE namespace=?1 AND (?2=0 OR (?2=1 AND key=?3) OR (?2=2 AND substr(key,1,length(?3))=?3) OR (?2=3 AND instr(key,?3)>0)) RETURNING key",
                    params![namespace.as_str(), mode, pattern], |r| r.get::<_, String>(0),
                )?);
            }
            Ok(deleted)
        }).and_then(|pending| pending.wait()).map_err(|error| error.to_string())
    }
}
