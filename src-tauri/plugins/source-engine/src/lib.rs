//! 源引擎 SDK：能力模块自行实现，平台模块仅处理系统边界。
mod api;
pub use api::*;
mod engine;
#[cfg(desktop)]
pub use engine::JvmEngine;
pub use engine::{Client, SourceEngine};
mod browser;
#[cfg(desktop)]
mod desktop;
mod ffi;
pub mod http;
mod image;
#[cfg(mobile)]
mod mobile;
pub mod source;
mod storage;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
use android as platform;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(all(desktop, feature = "desktop-runtime"))]
pub use desktop::browser::install as install_browser_app;
#[cfg(target_os = "ios")]
use ios as platform;

pub trait SourceEngineExt<R: tauri::Runtime> {
    fn source_engine(&self) -> &SourceEngine<R>;
}
impl<R: tauri::Runtime, T: tauri::Manager<R>> SourceEngineExt<R> for T {
    fn source_engine(&self) -> &SourceEngine<R> {
        self.state::<SourceEngine<R>>().inner()
    }
}
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri::Manager;
    tauri::plugin::Builder::new("source-engine")
        .setup(|app, api| {
            let engine = platform::init(app, api).map_err(std::io::Error::other)?;
            app.manage(engine);
            Ok(())
        })
        .build()
}
