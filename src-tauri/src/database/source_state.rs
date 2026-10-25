//! 引擎存储端口：键值和过期规则归数据库管理，不接收原生协议。
use super::*;
use tauri_plugin_source_engine::{KeyFilter, Namespace, StorageHost};
struct SourceState(Database);
impl Database {
    pub(crate) fn source_storage(&self) -> Arc<dyn StorageHost> {
        Arc::new(SourceState(self.clone()))
    }
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
impl StorageHost for SourceState {
    fn get(&self, namespace: Namespace, key: &str) -> std::result::Result<Option<String>, String> {
        db::read(&self.0,|session|{
            let values=session.read(
                "SELECT value FROM source_kv WHERE namespace=?1 AND key=?2 AND (expires_at_ms IS NULL OR expires_at_ms>?3)",
                params![namespace.as_str(),key,now()],|row|row.get(0))?;
            Ok(values.into_iter().next())
        }).map_err(|e|e.to_string())
    }
    fn put(
        &self,
        namespace: Namespace,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> std::result::Result<(), String> {
        let key = key.to_owned();
        let value = value.to_owned();
        let expiry = ttl_seconds
            .filter(|v| *v > 0)
            .map(|v| now().saturating_add(v.saturating_mul(1000).min(i64::MAX as u64) as i64));
        db::write(&self.0,WriteMode::Transaction,move|session|{
            session.write(
                "INSERT INTO source_kv(namespace,key,value,expires_at_ms) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,expires_at_ms=excluded.expires_at_ms",
                params![namespace.as_str(),key,value,expiry])?;
            Ok(())
        }).and_then(Pending::wait).map_err(|e|e.to_string())
    }
    fn remove(
        &self,
        namespace: Namespace,
        filter: KeyFilter,
    ) -> std::result::Result<Vec<String>, String> {
        let condition = match filter {
            KeyFilter::Key(value) => Some(("key=?2", value)),
            KeyFilter::Contains(value) => {
                Some(("instr(CAST(key AS BLOB),CAST(?2 AS BLOB))>0", value))
            }
            // 按字节比较，保留百分号、下划线和空字节的字面含义。
            KeyFilter::Prefixes(values) => Some((
                "EXISTS(SELECT 1 FROM json_each(?2) WHERE substr(CAST(source_kv.key AS BLOB),1,length(CAST(value AS BLOB)))=CAST(value AS BLOB))",
                serde_json::to_string(&values).map_err(|e| e.to_string())?,
            )),
            KeyFilter::All => None,
        };
        db::write(
            &self.0,
            WriteMode::Transaction,
            move |session| match condition {
                Some((predicate, value)) => session.read(
                    &format!(
                        "DELETE FROM source_kv WHERE namespace=?1 AND {predicate} RETURNING key"
                    ),
                    params![namespace.as_str(), value],
                    |row| row.get(0),
                ),
                None => session.read(
                    "DELETE FROM source_kv WHERE namespace=?1 RETURNING key",
                    params![namespace.as_str()],
                    |row| row.get(0),
                ),
            },
        )
        .and_then(Pending::wait)
        .map_err(|e| e.to_string())
    }
}
