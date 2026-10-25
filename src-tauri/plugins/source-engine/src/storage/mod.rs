//! 原生存储适配；实际持久化由调用方提供的 StorageHost 完成。
use crate::{KeyFilter, Namespace, Result};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
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
struct Response {
    value: Option<String>,
}
pub(crate) fn native(input: &str) -> String {
    let result = serde_json::from_str(input)
        .map_err(|e| e.to_string())
        .and_then(apply);
    crate::ffi::response(result)
}
fn apply(request: Request) -> Result<Response> {
    let call = crate::engine::context::get(&request.execution_id)?;
    let ns = request.namespace;
    let key = match &request.operation {
        Operation::Get { key }
        | Operation::Put { key, .. }
        | Operation::Delete { key }
        | Operation::JarHasCookies { key, .. } => Some(key),
        _ => None,
    };
    if key.is_some_and(|key| key.len() > 16384) {
        return Err("存储键超过限制".into());
    }
    let value = match request.operation {
        Operation::Get { key } => call.storage.get(ns, &key)?,
        Operation::Put {
            key,
            value,
            ttl_seconds,
        } => {
            call.storage.put(ns, &key, &value, ttl_seconds)?;
            None
        }
        Operation::JarHasCookies { key, value } if ns == Namespace::Cookies => {
            Some(crate::http::has_cookie_jar_cookies(&key, &value)?.to_string())
        }
        operation => {
            let filter = match operation {
                Operation::Delete { key } => KeyFilter::Key(key),
                Operation::DeletePrefixes { key_prefixes } if key_prefixes.len() <= 64 => {
                    KeyFilter::Prefixes(key_prefixes)
                }
                Operation::DeleteContains { contains } => KeyFilter::Contains(contains),
                Operation::Clear => KeyFilter::All,
                _ => return Err("存储操作无效".into()),
            };
            let all = matches!(&filter, KeyFilter::All);
            let deleted = call.storage.remove(ns, filter)?;
            if ns == Namespace::Cookies {
                if all {
                    crate::http::clear_all_cookie_jars()?;
                } else {
                    for key in deleted {
                        crate::http::clear_cookie_jar(&key)?;
                    }
                }
            }
            None
        }
    };
    Ok(Response { value })
}
