//! 一次引擎调用的私有上下文；仅用于关联原生回调，不提供任务总线。
use crate::{EngineContext, Result, StorageHost};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub(crate) struct Call {
    pub cancel: CancellationToken,
    pub deadline: Option<Instant>,
    pub storage: Arc<dyn StorageHost>,
}
static CALLS: LazyLock<Mutex<HashMap<String, Arc<Call>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub(crate) struct Lease(pub String);
impl Lease {
    pub fn enter(context: &EngineContext, search: bool) -> Result<Self> {
        let cancel = context
            .cancellation
            .clone()
            .unwrap_or_default()
            .child_token();
        if cancel.is_cancelled() {
            return Err("调用已取消".into());
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let call = Call {
            cancel,
            deadline: search.then(|| {
                Instant::now() + Duration::from_secs(crate::http::network_timeout_seconds())
            }),
            storage: context.storage.clone(),
        };
        CALLS
            .lock()
            .map_err(|_| "调用上下文不可用")?
            .insert(id.clone(), Arc::new(call));
        Ok(Self(id))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut calls) = CALLS.lock() {
            if let Some(call) = calls.remove(&self.0) {
                call.cancel.cancel();
            }
        }
    }
}
pub(crate) fn get(id: &str) -> Result<Arc<Call>> {
    let call = CALLS
        .lock()
        .map_err(|_| "调用上下文不可用")?
        .get(id)
        .cloned()
        .ok_or("调用已结束")?;
    if call.cancel.is_cancelled() {
        return Err("调用已取消".into());
    }
    if call
        .deadline
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        return Err("书源搜索超时".into());
    }
    Ok(call)
}
pub(crate) fn cancelled(id: &str) -> bool {
    get(id).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyFilter, Namespace};

    struct Storage;
    impl StorageHost for Storage {
        fn get(&self, _: Namespace, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn put(&self, _: Namespace, _: &str, _: &str, _: Option<u64>) -> Result<()> {
            Ok(())
        }
        fn remove(&self, _: Namespace, _: KeyFilter) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }

    #[test]
    fn configured_search_budget_preserves_immediate_cancellation() {
        let previous = crate::http::network_timeout_seconds();
        crate::http::set_network_timeout_seconds(60);
        let cancellation = CancellationToken::new();
        let context = EngineContext {
            data_dir: Default::default(),
            resource_dir: None,
            storage: Arc::new(Storage),
            cancellation: Some(cancellation.clone()),
        };
        let lease = Lease::enter(&context, true).unwrap();
        let remaining = get(&lease.0)
            .unwrap()
            .deadline
            .unwrap()
            .duration_since(Instant::now());
        cancellation.cancel();
        let stopped = cancelled(&lease.0);
        drop(lease);
        crate::http::set_network_timeout_seconds(previous);
        assert!(remaining > Duration::from_secs(30));
        assert!(stopped);
    }
}
