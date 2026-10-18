//! Indexed SQLite key/value storage for source execution, cookies and variables.
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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

pub async fn execute_source_storage_request(
    request: SourceStorageRequest,
    data_dir: &Path,
) -> Result<SourceStorageResponse, String> {
    validate_namespace(&request.namespace)?;
    if request.key.as_ref().is_some_and(|key| key.len() > 16_384) {
        return Err("Source storage key exceeds 16 KiB".into());
    }
    if request.operation == "jarHasCookies" && request.namespace == "cookies" {
        let scope = required_key(&request)?;
        let url = request
            .value
            .as_deref()
            .ok_or("Cookie jar status request has no source URL")?;
        if url.len() > 16_384 {
            return Err("Cookie jar status URL exceeds 16 KiB".into());
        }
        return Ok(SourceStorageResponse {
            value: Some(crate::source_http::has_cookie_jar_cookies(&scope, url)?.to_string()),
        });
    }
    if request
        .key_prefixes
        .as_ref()
        .is_some_and(|prefixes| prefixes.len() > 64)
    {
        return Err("Source storage prefix list is too large".into());
    }
    let data_dir = data_dir.to_owned();
    tokio::task::spawn_blocking(move || execute_sql(request, &data_dir))
        .await
        .map_err(|e| e.to_string())?
}

fn execute_sql(
    request: SourceStorageRequest,
    data_dir: &Path,
) -> Result<SourceStorageResponse, String> {
    let db = crate::database::source_database(data_dir).map_err(|e| e.to_string())?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    if request.operation == "get" {
        let connection = db.connect().map_err(|e| e.to_string())?;
        let value = match request.key.as_deref() {
            None => None,
            Some(key) => connection.query_row("SELECT value FROM source_kv WHERE namespace=?1 AND key=?2 AND (expires_at_ms IS NULL OR expires_at_ms>?3)",params![request.namespace,key,now],|row|row.get(0)).optional().map_err(|e|e.to_string())?
        };
        return Ok(SourceStorageResponse { value });
    }
    let is_cookies=request.namespace == "cookies";
    let clear_all=request.operation == "clear";
    let clear_scopes = db.transaction(move |tx| {
        let mut clear_scopes = Vec::new();
        let error = crate::resources::ResourceError::new;
        match request.operation.as_str() {
            "put" => {
                let key = required_key(&request).map_err(error)?;
                let value = request.value.as_deref().ok_or_else(||error("Source storage put request has no value".to_string()))?;
                let expiry = request.ttl_seconds.filter(|seconds|*seconds>0).map(|seconds|now.saturating_add(seconds.saturating_mul(1000).min(i64::MAX as u64) as i64));
                tx.execute("INSERT INTO source_kv(namespace,key,value,expires_at_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,expires_at_ms=excluded.expires_at_ms WHERE source_kv.value!=excluded.value OR source_kv.expires_at_ms IS NOT excluded.expires_at_ms",params![request.namespace,key,value,expiry])?;
            }
            "delete" => {
                let key = required_key(&request).map_err(error)?;
                tx.execute("DELETE FROM source_kv WHERE namespace=?1 AND key=?2",params![request.namespace,key])?;
                clear_scopes.push(key);
            }
            "deletePrefixes" | "deleteContains" | "clear" => {
                let (predicate,argument) = match request.operation.as_str() {
                    "deletePrefixes" => ("EXISTS(SELECT 1 FROM json_each(?2) WHERE substr(source_kv.key,1,length(value))=value)",serde_json::to_string(request.key_prefixes.as_ref().ok_or_else(||error("Source storage deletePrefixes request has no prefixes".into()))?).unwrap()),
                    "deleteContains" => ("instr(key,?2)>0",request.contains.clone().ok_or_else(||error("Source storage deleteContains request has no substring".into()))?),
                    _ => ("?2 IS NOT NULL",String::new())
                };
                if request.namespace == "cookies" {
                    let mut stmt = tx.prepare(&format!("SELECT key FROM source_kv WHERE namespace=?1 AND ({predicate})"))?;
                    clear_scopes = stmt.query_map(params![request.namespace,argument],|row|row.get(0))?.collect::<Result<Vec<String>,_>>()?;
                }
                tx.execute(&format!("DELETE FROM source_kv WHERE namespace=?1 AND ({predicate})"),params![request.namespace,argument])?;
            }
            operation => return Err(error(format!("Unsupported source storage operation: {operation}")))
        }
        Ok(clear_scopes)
    }).map_err(|e|e.to_string())?;
    if is_cookies {
        if clear_all {
            crate::source_http::clear_all_cookie_jars()?;
        } else {
            for scope in clear_scopes {
                crate::source_http::clear_cookie_jar(&scope)?;
            }
        }
    }
    Ok(SourceStorageResponse { value: None })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(operation: &str, key: &str, value: Option<&str>) -> SourceStorageRequest {
        serde_json::from_value(
            json!({"namespace":"source-cache","operation":operation,"key":key,"value":value}),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn indexed_storage_preserves_concurrent_writes_and_observes_expiry() {
        let directory = tempfile::tempdir().unwrap();
        crate::database::Database::open(directory.path()).unwrap();
        let mut jobs = Vec::new();
        for index in 0..12 {
            let root = directory.path().to_owned();
            jobs.push(tokio::spawn(async move {
                execute_source_storage_request(
                    request("put", &format!("key-{index}"), Some("value")),
                    &root,
                )
                .await
                .unwrap();
            }));
        }
        for job in jobs {
            job.await.unwrap();
        }
        let db = crate::database::Database::at(directory.path());
        let connection = db.connect().unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM source_kv", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            12
        );
        db.transaction(|tx|{tx.execute("UPDATE source_kv SET expires_at_ms=1 WHERE key='key-0'", [])?;Ok(())}).unwrap();
        assert!(
            execute_source_storage_request(request("get", "key-0", None), directory.path())
                .await
                .unwrap()
                .value
                .is_none()
        );
        assert_eq!(
            execute_source_storage_request(request("get", "key-1", None), directory.path())
                .await
                .unwrap()
                .value
                .as_deref(),
            Some("value")
        );
        assert!(!directory.path().join("storage").exists());
    }

    #[tokio::test]
    async fn storage_deletion_treats_prefixes_literally_and_failed_commits_do_not_leak() {
        let directory = tempfile::tempdir().unwrap();
        for key in ["a%_one", "aXXone", "keep"] {
            execute_source_storage_request(request("put", key, Some("value")), directory.path())
                .await
                .unwrap();
        }
        let deletion = serde_json::from_value(
            json!({"namespace":"source-cache","operation":"deletePrefixes","keyPrefixes":["a%_"]}),
        )
        .unwrap();
        execute_source_storage_request(deletion, directory.path())
            .await
            .unwrap();
        assert!(
            execute_source_storage_request(request("get", "a%_one", None), directory.path())
                .await
                .unwrap()
                .value
                .is_none()
        );
        assert!(
            execute_source_storage_request(request("get", "aXXone", None), directory.path())
                .await
                .unwrap()
                .value
                .is_some()
        );
        let db = crate::database::Database::at(directory.path());
        db.transaction(|tx|{tx.execute_batch("CREATE TRIGGER fail_put BEFORE INSERT ON source_kv WHEN new.key='uncommitted' BEGIN SELECT RAISE(ABORT,'failure'); END;")?;Ok(())}).unwrap();
        assert!(execute_source_storage_request(
            request("put", "uncommitted", Some("value")),
            directory.path()
        )
        .await
        .is_err());
        assert!(execute_source_storage_request(
            request("get", "uncommitted", None),
            directory.path()
        )
        .await
        .unwrap()
        .value
        .is_none());
        let mut deletion = request("deleteContains", "", None);
        deletion.contains = Some("XX".into());
        execute_source_storage_request(deletion, directory.path())
            .await
            .unwrap();
        assert!(
            execute_source_storage_request(request("get", "aXXone", None), directory.path())
                .await
                .unwrap()
                .value
                .is_none()
        );
        execute_source_storage_request(request("clear", "", None), directory.path())
            .await
            .unwrap();
        assert!(
            execute_source_storage_request(request("get", "keep", None), directory.path())
                .await
                .unwrap()
                .value
                .is_none()
        );
    }
}
