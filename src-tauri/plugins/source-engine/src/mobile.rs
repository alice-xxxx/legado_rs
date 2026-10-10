use serde::de::DeserializeOwned;
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{
    SourceEngineCall, SourceEngineResponse, SourceLoginWebCancelled, SourceLoginWebCapture,
    SourceLoginWebSession, SourceLoginWebStart, SourceLoginWebStarted,
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
    pub fn execute(&self, call: SourceEngineCall) -> crate::Result<SourceEngineResponse> {
        self.0
            .run_mobile_plugin("execute", call)
            .map_err(Into::into)
    }

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
