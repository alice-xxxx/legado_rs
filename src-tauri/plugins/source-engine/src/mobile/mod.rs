//! 移动端共享的原生浏览器调用；注册由 Android 和 iOS 分别完成。
use crate::{Result, browser};
use std::sync::OnceLock;
type Renderer = Box<dyn Fn(browser::Request) -> Result<browser::Response> + Send + Sync>;
static RENDER: OnceLock<Renderer> = OnceLock::new();
pub(crate) fn install<R: tauri::Runtime>(handle: tauri::plugin::PluginHandle<R>) {
    let _ = RENDER.set(Box::new(move |request| {
        handle
            .run_mobile_plugin("renderBrowser", request)
            .map_err(|e| e.to_string())
    }));
}
pub(crate) fn render(request: browser::Request) -> Result<browser::Response> {
    RENDER.get().ok_or("浏览器宿主未注册")?(request)
}
