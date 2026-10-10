use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{SourceEngineCall, SourceEngineResponse};

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<SourceEngine<R>> {
    Ok(SourceEngine(std::marker::PhantomData))
}

pub struct SourceEngine<R>(std::marker::PhantomData<fn() -> R>);

impl<R: Runtime> SourceEngine<R> {
    pub fn execute(&self, _call: SourceEngineCall) -> crate::Result<SourceEngineResponse> {
        Err(crate::Error::UnsupportedTarget)
    }
}
