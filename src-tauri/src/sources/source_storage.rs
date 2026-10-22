//! Indexed SQLite key/value storage for source execution, cookies and variables.
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    let cookies = request.namespace == "cookies";
    let clear_all = request.operation == "clear";
    let data_dir = data_dir.to_owned();
    let (response, clear_scopes) = tokio::task::spawn_blocking(move || {
        crate::database::storage(request, &data_dir).map_err(|error|error.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    if cookies {
        if clear_all {
            crate::source_http::clear_all_cookie_jars()?;
        } else {
            for scope in clear_scopes {
                crate::source_http::clear_cookie_jar(&scope)?;
            }
        }
    }
    Ok(response)
}

pub(crate) fn required_key(request: &SourceStorageRequest) -> Result<String, String> {
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


