//! HTTP 能力：请求、重定向、超时、取消和独立 Cookie Jar 均在此实现。
use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, ClientBuilder, Url,
    cookie::{CookieStore, Jar},
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

static TIMEOUT: AtomicU64 = AtomicU64::new(30);
static JARS: LazyLock<Mutex<HashMap<String, Arc<Jar>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static IO: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
pub fn set_network_timeout_seconds(seconds: u64) {
    TIMEOUT.store(seconds.clamp(1, 300), Ordering::Relaxed);
}
pub fn network_timeout_seconds() -> u64 {
    TIMEOUT.load(Ordering::Relaxed)
}
pub fn request_error(error: reqwest::Error) -> String {
    let mut cause: Option<&dyn std::error::Error> = Some(&error);
    let mut messages = Vec::new();
    while let Some(error) = cause {
        messages.push(error.to_string());
        cause = error.source();
    }
    messages.join(": ")
}
pub fn builder() -> ClientBuilder {
    defaults(true)
}
pub fn builder_with_user_agent(enabled: bool) -> ClientBuilder {
    defaults(enabled)
}
pub fn raw_builder() -> ClientBuilder {
    defaults(false)
}
fn defaults(ua: bool) -> ClientBuilder {
    let builder = Client::builder().timeout(Duration::from_secs(TIMEOUT.load(Ordering::Relaxed)));
    if ua {
        builder.user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
    } else {
        builder
    }
}
fn method() -> String {
    "GET".into()
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpRequest {
    pub execution_id: Option<String>,
    pub url: String,
    #[serde(default = "method")]
    pub method: String,
    #[serde(default)]
    pub headers: Vec<SourceHttpHeader>,
    pub body_base64: Option<String>,
    #[serde(default)]
    pub multipart: Vec<SourceHttpMultipartPart>,
    pub proxy: Option<String>,
    pub connect_timeout_ms: Option<u64>,
    pub read_timeout_ms: Option<u64>,
    pub call_timeout_ms: Option<u64>,
    pub max_redirects: Option<usize>,
    #[serde(default)]
    pub use_cookie_jar: bool,
    #[serde(default)]
    pub cookie_scope: Option<String>,
    #[serde(default)]
    pub accept_invalid_certs: bool,
}
#[derive(Deserialize, Serialize)]
pub struct SourceHttpHeader {
    pub name: String,
    pub value: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpMultipartPart {
    pub name: String,
    pub text: Option<String>,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub bytes_base64: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpResponse {
    pub requested_url: String,
    pub final_url: String,
    pub status: u16,
    pub reason: String,
    pub redirects: Vec<SourceHttpRedirect>,
    pub headers: Vec<SourceHttpHeader>,
    pub body_base64: String,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpRedirect {
    pub from_url: String,
    pub to_url: String,
    pub status: u16,
}

fn url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("HTTP 地址无效".into());
    }
    Ok(url)
}
fn validate_scope(scope: &str, url: &Url) -> Result<()> {
    let scope = scope.trim_start_matches('.');
    let host = url.host_str().unwrap_or_default();
    if scope.is_empty()
        || scope.len() > 255
        || !(host == scope || host.ends_with(&format!(".{scope}")))
    {
        return Err("Cookie 范围与地址不符".into());
    }
    Ok(())
}
pub fn cookie_header_for_scope(source_url: &str, scope: &str) -> Result<Option<String>> {
    let url = url(source_url)?;
    validate_scope(scope, &url)?;
    JARS.lock()
        .map_err(|_| "Cookie 状态不可用")?
        .get(scope)
        .and_then(|jar| jar.cookies(&url))
        .map(|v| v.to_str().map(str::to_owned).map_err(|e| e.to_string()))
        .transpose()
}
pub fn clear_cookie_jar(scope: &str) -> Result<()> {
    JARS.lock().map_err(|_| "Cookie 状态不可用")?.remove(scope);
    Ok(())
}
pub fn clear_all_cookie_jars() -> Result<()> {
    JARS.lock().map_err(|_| "Cookie 状态不可用")?.clear();
    Ok(())
}
pub fn has_cookie_jar_cookies(scope: &str, source_url: &str) -> Result<bool> {
    Ok(cookie_header_for_scope(source_url, scope)?.is_some_and(|v| !v.is_empty()))
}

pub(crate) fn native(input: &str) -> String {
    let result = serde_json::from_str(input)
        .map_err(|e| e.to_string())
        .and_then(|request| blocking(fetch(request)));
    crate::ffi::response(result)
}
// 只有 HTTP 需要异步运行时；原生回调通过通道等待，避免嵌套 Tokio。
pub(crate) fn blocking<T: Send + 'static>(
    future: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    let runtime = IO.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("source-http")
            .build()
            .expect("创建 HTTP 运行时")
    });
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv().map_err(|e| e.to_string())?
}
pub(crate) async fn fetch(request: SourceHttpRequest) -> Result<SourceHttpResponse> {
    let call = request
        .execution_id
        .as_deref()
        .map(crate::engine::context::get)
        .transpose()?;
    let cancel = call.as_ref().map(|c| c.cancel.clone()).unwrap_or_default();
    let future = send(request, call.as_ref().and_then(|c| c.deadline));
    tokio::select! {biased;_=cancel.cancelled()=>Err("HTTP 请求已取消".into()),result=future=>result}
}
async fn send(
    request: SourceHttpRequest,
    deadline: Option<std::time::Instant>,
) -> Result<SourceHttpResponse> {
    let requested_url = request.url.clone();
    let target = url(&request.url)?;
    let redirects = Arc::new(Mutex::new(Vec::new()));
    let history = redirects.clone();
    let limit = request.max_redirects.unwrap_or(20).min(20);
    let mut client = builder()
        .danger_accept_invalid_certs(request.accept_invalid_certs)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > limit {
                return attempt.error("重定向次数超限");
            }
            if let Some(from) = attempt.previous().last() {
                if let Ok(mut h) = history.lock() {
                    h.push(SourceHttpRedirect {
                        from_url: from.to_string(),
                        to_url: attempt.url().to_string(),
                        status: attempt.status().as_u16(),
                    });
                }
            }
            attempt.follow()
        }));
    if let Some(proxy) = request.proxy.filter(|p| !p.is_empty()) {
        client = client.proxy(reqwest::Proxy::all(proxy).map_err(|e| e.to_string())?);
    }
    if let Some(ms) = request.connect_timeout_ms.filter(|v| *v > 0) {
        client = client.connect_timeout(Duration::from_millis(ms));
    }
    if let Some(ms) = request.read_timeout_ms.filter(|v| *v > 0) {
        client = client.read_timeout(Duration::from_millis(ms));
    }
    let mut timeout = Duration::from_millis(
        request
            .call_timeout_ms
            .filter(|v| *v > 0)
            .unwrap_or(TIMEOUT.load(Ordering::Relaxed) * 1000),
    );
    if let Some(deadline) = deadline {
        timeout = timeout.min(deadline.saturating_duration_since(std::time::Instant::now()));
    }
    client = client.timeout(timeout);
    if request.use_cookie_jar {
        let scope = request.cookie_scope.as_deref().ok_or("缺少 Cookie 范围")?;
        validate_scope(scope, &target)?;
        let jar = JARS
            .lock()
            .map_err(|_| "Cookie 状态不可用")?
            .entry(scope.into())
            .or_default()
            .clone();
        client = client.cookie_provider(jar);
    }
    let client = client.build().map_err(request_error)?;
    let method =
        reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
    let mut headers = HeaderMap::new();
    for h in request.headers {
        if [
            "host",
            "connection",
            "content-length",
            "transfer-encoding",
            "proxy-authorization",
        ]
        .contains(&h.name.to_ascii_lowercase().as_str())
        {
            continue;
        }
        headers.append(
            HeaderName::from_bytes(h.name.as_bytes()).map_err(|e| e.to_string())?,
            HeaderValue::from_str(&h.value).map_err(|e| e.to_string())?,
        );
    }
    let mut outgoing = client.request(method, target).headers(headers);
    if !request.multipart.is_empty() {
        let mut form = reqwest::multipart::Form::new();
        for p in request.multipart {
            let mut part = if let Some(text) = p.text {
                reqwest::multipart::Part::text(text)
            } else {
                reqwest::multipart::Part::bytes(
                    STANDARD
                        .decode(p.bytes_base64.unwrap_or_default())
                        .map_err(|e| e.to_string())?,
                )
            };
            if let Some(name) = p.file_name {
                part = part.file_name(name);
            }
            if let Some(mime) = p.content_type {
                part = part.mime_str(&mime).map_err(|e| e.to_string())?;
            }
            form = form.part(p.name, part);
        }
        outgoing = outgoing.multipart(form);
    } else if let Some(body) = request.body_base64 {
        outgoing = outgoing.body(STANDARD.decode(body).map_err(|e| e.to_string())?);
    }
    let mut response = outgoing.send().await.map_err(request_error)?;
    let status = response.status();
    let final_url = response.url().to_string();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| SourceHttpHeader {
            name: name.to_string(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len() + chunk.len() > 64 * 1024 * 1024 {
            return Err("HTTP 响应超过 64 MiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    let redirects = redirects.lock().map_err(|_| "重定向状态不可用")?.clone();
    Ok(SourceHttpResponse {
        requested_url,
        final_url,
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or_default().into(),
        redirects,
        headers,
        body_base64: STANDARD.encode(body),
    })
}
