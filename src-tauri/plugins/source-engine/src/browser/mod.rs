//! 浏览器能力入口；原生渲染与窗口控制由所属平台实现。
use crate::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub(crate) enum Mode {
    #[serde(rename = "OFFLINE_SNAPSHOT")]
    Offline,
    #[serde(rename = "DYNAMIC_PAGE")]
    Dynamic,
    #[serde(rename = "INTERACTIVE_PAGE")]
    Interactive,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Request {
    pub capability_id: String,
    pub html: String,
    pub base_url: String,
    pub java_script: String,
    pub delay_time_ms: u64,
    pub mode: Mode,
    pub method: String,
    pub headers: Vec<Header>,
    pub use_cookie_jar: bool,
    pub cookie_scope: Option<String>,
    pub title: String,
    pub wait_for_result: bool,
    pub execution_id: Option<String>,
    pub override_url_regex: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Header {
    pub name: String,
    pub value: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Response {
    pub capability_id: String,
    pub final_url: String,
    pub body: String,
    pub cookie_header: String,
}
pub(crate) fn native(input: &str) -> String {
    let result: Result<Response> = serde_json::from_str::<Request>(input)
        .map_err(|e| e.to_string())
        .and_then(|mut request| {
            if let Some(id) = &request.execution_id {
                crate::engine::context::get(id)?;
            }
            if uuid::Uuid::parse_str(&request.capability_id).is_err()
                || request.delay_time_ms > 15000
                || request.html.len() > 16 * 1024 * 1024
                || request.java_script.len() > 2 * 1024 * 1024
                || request.override_url_regex.as_ref().is_some_and(|r| r.len() > 4096)
            {
                return Err("浏览器请求超限".into());
            }
            let target = reqwest::Url::parse(&request.base_url).map_err(|e| e.to_string())?;
            if !matches!(target.scheme(), "http" | "https")
                || !target.username().is_empty()
                || target.password().is_some()
                || request.method != "GET"
            {
                return Err("浏览器地址或请求方式无效".into());
            }
            if request.use_cookie_jar
                && !request
                    .headers
                    .iter()
                    .any(|h| h.name.eq_ignore_ascii_case("cookie"))
            {
                if let Some(value) = crate::http::cookie_header_for_scope(
                    &request.base_url,
                    request.cookie_scope.as_deref().unwrap_or_default(),
                )? {
                    request.headers.push(Header {
                        name: "Cookie".into(),
                        value,
                    });
                }
            }
            let id = request.capability_id.clone();
            let base = request.base_url.clone();
            let offline = request.mode == Mode::Offline;
            let response = render(request)?;
            if response.capability_id != id
                || response.body.len() > 16 * 1024 * 1024
                || response.cookie_header.len() > 256 * 1024
                || offline && response.final_url != base
            {
                return Err("浏览器结果无效".into());
            }
            Ok(response)
        });
    crate::ffi::response(result)
}
fn render(request: Request) -> Result<Response> {
    #[cfg(all(desktop, feature = "desktop-runtime"))]
    return crate::desktop::browser::render(request);
    #[cfg(mobile)]
    return crate::mobile::render(request);
    #[cfg(all(desktop, not(feature = "desktop-runtime")))]
    {
        let _ = request;
        Err("当前运行时没有浏览器宿主".into())
    }
}
impl<R: tauri::Runtime> SourceEngine<R> {
    pub fn start_web_login(&self, _request: SourceLoginWebStart) -> Result<SourceLoginWebStarted> {
        #[cfg(all(desktop, feature = "desktop-runtime"))]
        return crate::desktop::browser::start(_request);
        #[cfg(mobile)]
        return self
            .handle
            .run_mobile_plugin("startWebLogin", _request)
            .map_err(|e| e.to_string());
        #[cfg(all(desktop, not(feature = "desktop-runtime")))]
        return Err("当前运行时没有浏览器宿主".into());
    }
    pub fn complete_web_login(
        &self,
        _request: SourceLoginWebSession,
    ) -> Result<SourceLoginWebCapture> {
        #[cfg(all(desktop, feature = "desktop-runtime"))]
        return crate::desktop::browser::complete(_request);
        #[cfg(mobile)]
        return self
            .handle
            .run_mobile_plugin("completeWebLogin", _request)
            .map_err(|e| e.to_string());
        #[cfg(all(desktop, not(feature = "desktop-runtime")))]
        return Err("当前运行时没有浏览器宿主".into());
    }
    pub fn cancel_web_login(
        &self,
        _request: SourceLoginWebSession,
    ) -> Result<SourceLoginWebCancelled> {
        #[cfg(all(desktop, feature = "desktop-runtime"))]
        return crate::desktop::browser::cancel(_request);
        #[cfg(mobile)]
        return self
            .handle
            .run_mobile_plugin("cancelWebLogin", _request)
            .map_err(|e| e.to_string());
        #[cfg(all(desktop, not(feature = "desktop-runtime")))]
        return Err("当前运行时没有浏览器宿主".into());
    }
}
