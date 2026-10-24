use serde::{Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{
    ExecutionContext, SourceLoginWebCancelled, SourceLoginWebCapture, SourceLoginWebSession,
    SourceLoginWebStart, SourceLoginWebStarted,
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_source_engine);

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<SourceEngine<R>> {
    #[cfg(target_os = "android")]
    let handle =
        api.register_android_plugin("io.legado.sourceengine.tauri", "SourceEnginePlugin")?;
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_source_engine)?;
    Ok(SourceEngine(handle))
}

pub struct SourceEngine<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> SourceEngine<R> {
    async fn invoke<Q, T>(
        &self,
        method: crate::api::Method,
        request: Q,
        context: ExecutionContext,
    ) -> crate::Result<T>
    where
        Q: Serialize + Send + 'static,
        T: DeserializeOwned + Send + 'static,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct NativeCall<Q> {
            #[serde(flatten)]
            request: Q,
            data_directory: String,
            execution_id: String,
        }
        let handle = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let lease = crate::host::ExecutionScope::enter(
                context.cancellation,
                context.storage,
                method.is_search(),
            )
            .map_err(crate::Error::Engine)?;
            handle
                .run_mobile_plugin(
                    method.signature().0,
                    NativeCall {
                        request,
                        data_directory: context.data_dir.to_string_lossy().into_owned(),
                        execution_id: lease.id().to_owned(),
                    },
                )
                .map_err(Into::into)
        })
        .await
        .map_err(|error| crate::Error::Engine(format!("Native source worker failed: {error}")))?
    }

    crate::api::source_methods!();

    pub fn start_web_login(
        &self,
        request: SourceLoginWebStart,
    ) -> crate::Result<SourceLoginWebStarted> {
        self.0
            .run_mobile_plugin("startWebLogin", request)
            .map_err(Into::into)
    }

    pub fn complete_web_login(
        &self,
        request: SourceLoginWebSession,
    ) -> crate::Result<SourceLoginWebCapture> {
        self.0
            .run_mobile_plugin("completeWebLogin", request)
            .map_err(Into::into)
    }

    pub fn cancel_web_login(
        &self,
        request: SourceLoginWebSession,
    ) -> crate::Result<SourceLoginWebCancelled> {
        self.0
            .run_mobile_plugin("cancelWebLogin", request)
            .map_err(Into::into)
    }
}
