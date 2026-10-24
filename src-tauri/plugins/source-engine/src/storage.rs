//! Host persistence port. Native wire formats stay inside the plugin.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Namespace {
    SourceCache,
    Cookies,
    FileCache,
    PersistentFileCache,
    ExploreKinds,
    BookVariables,
}

impl Namespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceCache => "source-cache",
            Self::Cookies => "cookies",
            Self::FileCache => "file-cache",
            Self::PersistentFileCache => "persistent-file-cache",
            Self::ExploreKinds => "explore-kinds",
            Self::BookVariables => "book-variables",
        }
    }
}

pub enum KeyFilter {
    Key(String),
    Prefixes(Vec<String>),
    Contains(String),
    All,
}

pub trait StorageHost: Send + Sync {
    fn get(&self, namespace: Namespace, key: &str) -> Result<Option<String>, String>;
    fn put(
        &self,
        namespace: Namespace,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<(), String>;
    /// Returns deleted keys so the plugin can invalidate the affected cookie jars.
    fn remove(&self, namespace: Namespace, filter: KeyFilter) -> Result<Vec<String>, String>;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceStorageRequest {
    execution_id: String,
    namespace: Namespace,
    #[serde(flatten)]
    operation: Operation,
}

#[derive(Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Get {
        key: String,
    },
    Put {
        key: String,
        value: String,
        ttl_seconds: Option<u64>,
    },
    Delete {
        key: String,
    },
    DeletePrefixes {
        key_prefixes: Vec<String>,
    },
    DeleteContains {
        contains: String,
    },
    Clear,
    JarHasCookies {
        key: String,
        value: String,
    },
}

#[derive(Serialize)]
pub(crate) struct SourceStorageResponse {
    pub(crate) value: Option<String>,
}

pub(crate) async fn execute_source_storage_request(
    request: SourceStorageRequest,
) -> Result<SourceStorageResponse, String> {
    let storage = crate::host::execution_storage(&request.execution_id)?;
    match &request.operation {
        Operation::Get { key }
        | Operation::Put { key, .. }
        | Operation::Delete { key }
        | Operation::JarHasCookies { key, .. }
            if key.len() > 16_384 =>
        {
            return Err("Source storage key exceeds 16 KiB".into());
        }
        Operation::DeletePrefixes { key_prefixes } if key_prefixes.len() > 64 => {
            return Err("Source storage prefix list is too large".into());
        }
        _ => {}
    }
    if let Operation::JarHasCookies { key, value } = &request.operation {
        if request.namespace != Namespace::Cookies || value.len() > 16_384 {
            return Err("Invalid cookie jar status request".into());
        }
        return Ok(SourceStorageResponse {
            value: Some(crate::http::has_cookie_jar_cookies(key, value)?.to_string()),
        });
    }
    let cookies = request.namespace == Namespace::Cookies;
    let clear_all = matches!(&request.operation, Operation::Clear);
    let (value, deleted) = tokio::task::spawn_blocking(move || {
        let ns = request.namespace;
        let mut deleted = Vec::new();
        let value = match request.operation {
            Operation::Get { key } => storage.get(ns, &key)?,
            Operation::Put {
                key,
                value,
                ttl_seconds,
            } => {
                storage.put(ns, &key, &value, ttl_seconds)?;
                None
            }
            operation => {
                let filter = match operation {
                    Operation::Delete { key } => KeyFilter::Key(key),
                    Operation::DeletePrefixes { key_prefixes } => KeyFilter::Prefixes(key_prefixes),
                    Operation::DeleteContains { contains } => KeyFilter::Contains(contains),
                    Operation::Clear => KeyFilter::All,
                    _ => unreachable!(),
                };
                deleted = storage.remove(ns, filter)?;
                None
            }
        };
        Ok::<_, String>((value, deleted))
    })
    .await
    .map_err(|error| error.to_string())??;
    if cookies {
        if clear_all {
            crate::http::clear_all_cookie_jars()?;
        } else {
            for key in deleted {
                crate::http::clear_cookie_jar(&key)?;
            }
        }
    }
    Ok(SourceStorageResponse { value })
}
