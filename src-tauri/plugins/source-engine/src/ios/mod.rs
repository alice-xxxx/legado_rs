//! iOS 插件注册；C ABI 独立存放。
mod ffi;
use crate::*;
tauri::ios_plugin_binding!(init_plugin_source_engine);
pub(crate) fn init<R: tauri::Runtime, C: serde::de::DeserializeOwned>(
    _: &tauri::AppHandle<R>,
    api: tauri::plugin::PluginApi<R, C>,
) -> Result<SourceEngine<R>> {
    let handle = api
        .register_ios_plugin(init_plugin_source_engine)
        .map_err(|e| e.to_string())?;
    crate::mobile::install(handle.clone());
    Ok(SourceEngine {
        handle,
        runtime: std::marker::PhantomData,
    })
}
