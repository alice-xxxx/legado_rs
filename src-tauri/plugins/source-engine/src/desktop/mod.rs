//! Windows、macOS、Linux 共用 JVM 嵌入；系统路径由目标平台负责。
#[cfg(feature = "desktop-runtime")]
pub(crate) mod browser;
pub(crate) mod jvm;
pub(crate) fn init<R: tauri::Runtime, C: serde::de::DeserializeOwned>(
    _: &tauri::AppHandle<R>,
    _: tauri::plugin::PluginApi<R, C>,
) -> crate::Result<crate::SourceEngine<R>> {
    Ok(crate::SourceEngine {
        runtime: std::marker::PhantomData,
    })
}
