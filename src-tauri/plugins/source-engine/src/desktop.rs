use crate::{ExecutionContext, api::Method};
use serde::{Serialize, de::DeserializeOwned};
use tauri::{AppHandle, Runtime, plugin::PluginApi};

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<SourceEngine<R>> {
    Ok(SourceEngine(std::marker::PhantomData))
}

/// The same typed engine can run without a Tauri window.
pub struct JvmEngine;
impl JvmEngine {
    async fn invoke<Q: Serialize + Send, T: DeserializeOwned>(
        &self,
        method: Method,
        request: Q,
        context: ExecutionContext,
    ) -> crate::Result<T> {
        crate::jvm::invoke(method, request, context)
            .await
            .map_err(crate::Error::Engine)
    }
    crate::api::source_methods!();
}

pub struct SourceEngine<R>(std::marker::PhantomData<fn() -> R>);
impl<R: Runtime> SourceEngine<R> {
    async fn invoke<Q: Serialize + Send, T: DeserializeOwned>(
        &self,
        method: Method,
        request: Q,
        context: ExecutionContext,
    ) -> crate::Result<T> {
        JvmEngine.invoke(method, request, context).await
    }
    crate::api::source_methods!();
}

#[cfg(feature = "desktop-runtime")]
impl<R: Runtime> SourceEngine<R> {
    pub fn start_web_login(
        &self,
        request: crate::SourceLoginWebStart,
    ) -> crate::Result<crate::SourceLoginWebStarted> {
        crate::browser::start_web_login(request).map_err(crate::Error::Engine)
    }
    pub fn complete_web_login(
        &self,
        request: crate::SourceLoginWebSession,
    ) -> crate::Result<crate::SourceLoginWebCapture> {
        crate::browser::complete_web_login(request).map_err(crate::Error::Engine)
    }
    pub fn cancel_web_login(
        &self,
        request: crate::SourceLoginWebSession,
    ) -> crate::Result<crate::SourceLoginWebCancelled> {
        crate::browser::cancel_web_login(request).map_err(crate::Error::Engine)
    }
}
