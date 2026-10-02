use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::multipart::{Form, Part};
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Proxy, Url};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

static COOKIE_JAR: LazyLock<Arc<Jar>> = LazyLock::new(|| Arc::new(Jar::default()));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpRequest {
    pub url: String,
    #[serde(default = "default_method")]
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
    pub accept_invalid_certs: bool,
}

fn default_method() -> String {
    "GET".to_owned()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpMultipartPart {
    pub name: String,
    pub text: Option<String>,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub bytes_base64: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpResponse {
    pub requested_url: String,
    pub final_url: String,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<SourceHttpHeader>,
    pub body_base64: String,
}

pub(super) async fn execute_source_http_request(
    request: SourceHttpRequest,
) -> Result<SourceHttpResponse, String> {
    // 网络边界固定在 Rust：书源只能通过 HTTP(S) 请求，不允许 Kotlin 侧把 file:// 等
    // 非网络 scheme 借 reqwest 路径扩展成任意系统资源读取。
    let url = Url::parse(&request.url).map_err(|error| format!("Invalid request URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("Unsupported request URL scheme: {}", url.scheme()));
    }

    let method = Method::from_bytes(request.method.trim().as_bytes())
        .map_err(|error| format!("Invalid HTTP method: {error}"))?;

    let mut headers = HeaderMap::new();
    for header in &request.headers {
        let name = HeaderName::from_bytes(header.name.trim().as_bytes())
            .map_err(|error| format!("Invalid HTTP header name: {error}"))?;
        let value = HeaderValue::from_str(&header.value)
            .map_err(|error| format!("Invalid value for HTTP header {}: {error}", header.name))?;
        headers.append(name, value);
    }

    if !request.multipart.is_empty() && request.body_base64.is_some() {
        return Err("Use either bodyBase64 or multipart, not both".to_owned());
    }

    // 每个书源请求可能带独立代理、超时和证书策略；按请求建 client 以免配置串到其他书源。
    // Cookie Jar 是显式共享的进程级对象，满足开启 cookieJar 的书源跨请求保留 Cookie。
    let redirect_policy = Policy::limited(request.max_redirects.unwrap_or(10).min(100));
    let mut client_builder = Client::builder().redirect(redirect_policy);
    if let Some(timeout_ms) = request.connect_timeout_ms {
        client_builder = client_builder.connect_timeout(Duration::from_millis(timeout_ms));
    }
    if let Some(timeout_ms) = request.read_timeout_ms {
        client_builder = client_builder.read_timeout(Duration::from_millis(timeout_ms));
    }
    if let Some(timeout_ms) = request.call_timeout_ms {
        client_builder = client_builder.timeout(Duration::from_millis(timeout_ms));
    }
    if request.use_cookie_jar {
        client_builder = client_builder.cookie_provider(Arc::clone(&COOKIE_JAR));
    }
    if request.accept_invalid_certs {
        client_builder = client_builder.danger_accept_invalid_certs(true);
    }
    if let Some(proxy) = request
        .proxy
        .as_deref()
        .filter(|proxy| !proxy.trim().is_empty())
    {
        let proxy = Proxy::all(proxy).map_err(|error| format!("Invalid proxy URL: {error}"))?;
        client_builder = client_builder.proxy(proxy);
    }

    let client = client_builder
        .build()
        .map_err(|error| format!("Failed to create HTTP client: {error}"))?;
    let mut request_builder = client.request(method, url.clone()).headers(headers);

    if !request.multipart.is_empty() {
        let mut form = Form::new();
        for part in request.multipart {
            if let Some(bytes_base64) = part.bytes_base64 {
                let bytes = BASE64.decode(bytes_base64).map_err(|error| {
                    format!("Invalid base64 in multipart field {}: {error}", part.name)
                })?;
                let mut file_part = Part::bytes(bytes);
                if let Some(file_name) = part.file_name {
                    file_part = file_part.file_name(file_name);
                }
                if let Some(content_type) = part.content_type {
                    file_part = file_part
                        .mime_str(&content_type)
                        .map_err(|error| format!("Invalid multipart content type: {error}"))?;
                }
                form = form.part(part.name, file_part);
            } else {
                form = form.text(part.name, part.text.unwrap_or_default());
            }
        }
        request_builder = request_builder.multipart(form);
    } else if let Some(body_base64) = request.body_base64 {
        let body = BASE64
            .decode(body_base64)
            .map_err(|error| format!("Invalid base64 request body: {error}"))?;
        request_builder = request_builder.body(body);
    }

    let response = request_builder
        .send()
        .await
        .map_err(|error| format!("HTTP request failed: {error}"))?;
    let final_url = response.url().to_string();
    let status = response.status();
    let reason = status.canonical_reason().unwrap_or_default().to_owned();
    let response_headers = response
        .headers()
        .iter()
        .map(|(name, value)| SourceHttpHeader {
            name: name.as_str().to_owned(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("Failed to read HTTP response body: {error}"))?;

    Ok(SourceHttpResponse {
        requested_url: request.url,
        final_url,
        status: status.as_u16(),
        reason,
        headers: response_headers,
        body_base64: BASE64.encode(body),
    })
}
