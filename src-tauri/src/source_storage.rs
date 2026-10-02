use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

static STORAGE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStorageRequest {
    pub namespace: String,
    pub operation: String,
    pub key: Option<String>,
    pub value: Option<String>,
    pub ttl_seconds: Option<u64>,
    pub key_prefixes: Option<Vec<String>>,
    pub contains: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStorageResponse {
    pub value: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct StorageFile {
    entries: HashMap<String, StorageEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageEntry {
    value: String,
    expires_at_ms: Option<u64>,
}

/// 在 Rust 执行解析器缓存操作，避免 Kotlin 规则直接打开或写入宿主文件。
pub async fn execute_source_storage_request(
    request: SourceStorageRequest,
    data_dir: &Path,
) -> Result<SourceStorageResponse, String> {
    validate_namespace(&request.namespace)?;
    if request.key.as_ref().is_some_and(|key| key.len() > 16_384) {
        return Err("Source storage key exceeds 16 KiB".to_owned());
    }

    // 同一 Tauri 进程可能并发执行多个书源操作；读改写整份 namespace 文件必须串行，
    // 否则两个解析请求同时更新会互相覆盖对方刚写入的 key。
    let _guard = STORAGE_LOCK.lock().await;
    let path = storage_path(data_dir, &request.namespace);
    let mut store = read_storage(&path).await?;
    let now = current_time_ms();
    let old_len = store.entries.len();
    store
        .entries
        .retain(|_, entry| entry.expires_at_ms.is_none_or(|deadline| deadline > now));
    let mut changed = store.entries.len() != old_len;

    let value = match request.operation.as_str() {
        "get" => request
            .key
            .as_deref()
            .and_then(|key| store.entries.get(key))
            .map(|entry| entry.value.clone()),
        "put" => {
            let key = required_key(&request)?;
            let value = request
                .value
                .clone()
                .ok_or_else(|| "Source storage put request has no value".to_owned())?;
            let expires_at_ms = request
                .ttl_seconds
                .filter(|seconds| *seconds > 0)
                .map(|seconds| now.saturating_add(seconds.saturating_mul(1000)));
            store.entries.insert(
                key,
                StorageEntry {
                    value,
                    expires_at_ms,
                },
            );
            changed = true;
            None
        }
        "delete" => {
            changed |= store.entries.remove(&required_key(&request)?).is_some();
            None
        }
        "deletePrefixes" => {
            let prefixes = request.key_prefixes.as_deref().ok_or_else(|| {
                "Source storage deletePrefixes request has no prefixes".to_owned()
            })?;
            if prefixes.len() > 64 {
                return Err("Source storage prefix list is too large".to_owned());
            }
            let before = store.entries.len();
            store
                .entries
                .retain(|key, _| !prefixes.iter().any(|prefix| key.starts_with(prefix)));
            changed |= before != store.entries.len();
            None
        }
        "deleteContains" => {
            let needle = request.contains.as_deref().ok_or_else(|| {
                "Source storage deleteContains request has no substring".to_owned()
            })?;
            let before = store.entries.len();
            store.entries.retain(|key, _| !key.contains(needle));
            changed |= before != store.entries.len();
            None
        }
        "clear" => {
            changed |= !store.entries.is_empty();
            store.entries.clear();
            None
        }
        operation => return Err(format!("Unsupported source storage operation: {operation}")),
    };

    if changed {
        write_storage(&path, &store).await?;
    }
    Ok(SourceStorageResponse { value })
}

fn required_key(request: &SourceStorageRequest) -> Result<String, String> {
    request
        .key
        .clone()
        .ok_or_else(|| "Source storage request has no key".to_owned())
}

fn validate_namespace(namespace: &str) -> Result<(), String> {
    const NAMESPACES: &[&str] = &[
        "source-cache",
        "cookies",
        "file-cache",
        "persistent-file-cache",
        "explore-kinds",
        "book-variables",
    ];
    if NAMESPACES.contains(&namespace) {
        Ok(())
    } else {
        Err(format!("Unsupported source storage namespace: {namespace}"))
    }
}

fn storage_path(data_dir: &Path, namespace: &str) -> PathBuf {
    data_dir.join("storage").join(format!("{namespace}.json"))
}

async fn read_storage(path: &Path) -> Result<StorageFile, String> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Cannot decode source storage {}: {error}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(StorageFile::default()),
        Err(error) => Err(format!(
            "Cannot read source storage {}: {error}",
            path.display()
        )),
    }
}

async fn write_storage(path: &Path, store: &StorageFile) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Source storage path has no parent".to_owned())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("Cannot create source storage directory: {error}"))?;
    let bytes = serde_json::to_vec(store)
        .map_err(|error| format!("Cannot encode source storage: {error}"))?;
    // 先落到同目录临时文件，再替换正式文件，避免序列化/写入中断留下半份 JSON。
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(".tmp-{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|error| format!("Cannot write source storage temporary file: {error}"))?;
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        tokio::fs::remove_file(path)
            .await
            .map_err(|error| format!("Cannot replace source storage file: {error}"))?;
    }
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|error| format!("Cannot finalize source storage file: {error}"))
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::{execute_source_storage_request, SourceStorageRequest};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn test_data_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "legado-source-storage-{}-{nonce}",
            std::process::id()
        ))
    }

    fn request(operation: &str, key: Option<&str>, value: Option<&str>) -> SourceStorageRequest {
        SourceStorageRequest {
            namespace: "source-cache".to_owned(),
            operation: operation.to_owned(),
            key: key.map(str::to_owned),
            value: value.map(str::to_owned),
            ttl_seconds: None,
            key_prefixes: None,
            contains: None,
        }
    }

    #[tokio::test]
    async fn storage_persists_updates_and_deletes_selected_keys() {
        let data_dir = test_data_dir();
        execute_source_storage_request(request("put", Some("book:1"), Some("first")), &data_dir)
            .await
            .expect("first put");
        execute_source_storage_request(request("put", Some("book:1"), Some("updated")), &data_dir)
            .await
            .expect("update existing key");
        let value = execute_source_storage_request(request("get", Some("book:1"), None), &data_dir)
            .await
            .expect("get updated value")
            .value;
        assert_eq!(value.as_deref(), Some("updated"));

        let mut delete = request("deletePrefixes", None, None);
        delete.key_prefixes = Some(vec!["book:".to_owned()]);
        execute_source_storage_request(delete, &data_dir)
            .await
            .expect("delete matching keys");
        let value = execute_source_storage_request(request("get", Some("book:1"), None), &data_dir)
            .await
            .expect("get deleted value")
            .value;
        assert_eq!(value, None);

        let _ = tokio::fs::remove_dir_all(data_dir).await;
    }

    #[tokio::test]
    async fn storage_expires_values_and_rejects_unknown_namespaces() {
        let data_dir = test_data_dir();
        let mut expiring = request("put", Some("short-lived"), Some("value"));
        expiring.ttl_seconds = Some(1);
        execute_source_storage_request(expiring, &data_dir)
            .await
            .expect("put expiring value");
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let value =
            execute_source_storage_request(request("get", Some("short-lived"), None), &data_dir)
                .await
                .expect("read expired value")
                .value;
        assert_eq!(value, None);

        let mut invalid = request("get", Some("key"), None);
        invalid.namespace = "arbitrary-path".to_owned();
        assert!(execute_source_storage_request(invalid, &data_dir)
            .await
            .is_err());
        let _ = tokio::fs::remove_dir_all(data_dir).await;
    }
}
