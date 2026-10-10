//! 为 KMP 书源引擎提供受控 HTTP 请求能力。
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::multipart::{Form, Part};
use reqwest::redirect::{Attempt, Policy};
use reqwest::{Client, Method, Proxy, Request, Url};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error as StdError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_SAFE_RETRIES: usize = 2;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(5);
const MAX_SOURCE_HTTP_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_NETWORK_TIMEOUT_SECONDS: u64 = 15;

static NETWORK_TIMEOUT_SECONDS: AtomicU64 = AtomicU64::new(DEFAULT_NETWORK_TIMEOUT_SECONDS);

static COOKIE_JARS: LazyLock<Mutex<HashMap<String, Arc<Jar>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn set_network_timeout_seconds(seconds: u64) {
    NETWORK_TIMEOUT_SECONDS.store(seconds, Ordering::Relaxed);
}

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
    pub cookie_scope: Option<String>,
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
    pub redirects: Vec<SourceHttpRedirect>,
    pub headers: Vec<SourceHttpHeader>,
    pub body_base64: String,
}

/// A redirect actually followed by reqwest for this one request. This stays inside the
/// Rust-to-KMP host protocol and is never projected into public book/source resources.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHttpRedirect {
    pub from_url: String,
    pub to_url: String,
    pub status: u16,
}

pub(super) async fn execute_source_http_request(
    request: SourceHttpRequest,
) -> Result<SourceHttpResponse, String> {
    // 网络边界固定在 Rust：书源只能通过 HTTP(S) 请求，不允许 Kotlin 侧把 file:// 等
    // 非网络 scheme 借 reqwest 路径扩展成任意系统资源读取。
    let url = Url::parse(&request.url).map_err(|error| format!("Invalid request URL: {error}"))?;
    let requested_url = request.url.clone();
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("Unsupported request URL scheme: {}", url.scheme()));
    }

    let method = Method::from_bytes(request.method.trim().as_bytes())
        .map_err(|error| format!("Invalid HTTP method: {error}"))?;
    let retryable_request = matches!(method, Method::GET | Method::HEAD)
        && request.body_base64.is_none()
        && request.multipart.is_empty();

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

    let cookie_partition = if request.use_cookie_jar {
        let scope = cookie_scope(&request, &url)?;
        let partition = cookie_jar(&scope)?;
        Some(partition)
    } else {
        None
    };

    // 每个书源请求可能带独立代理、超时和证书策略；按请求建 client 以免配置串到其他书源。
    // Cookie Jar 按 KMP CookieStore 的规范化域分区，同域书源沿用共享语义，
    // 但清除登录状态时不影响其他域。
    // Mirror reqwest's Policy::limited semantics exactly, while retaining the real response
    // status and next URL for each followed hop. The trace is request-local and bounded by the
    // same maximum (which is itself capped at 100).
    let max_redirects = request.max_redirects.unwrap_or(10).min(100);
    let redirects = Arc::new(Mutex::new(Vec::<SourceHttpRedirect>::new()));
    let redirect_trace = Arc::clone(&redirects);
    let redirect_policy = Policy::custom(move |attempt: Attempt<'_>| {
        if attempt.previous().len() > max_redirects {
            return attempt.error("too many redirects");
        }
        if let Some(from_url) = attempt.previous().last() {
            if let Ok(mut redirects) = redirect_trace.lock() {
                redirects.push(SourceHttpRedirect {
                    from_url: from_url.to_string(),
                    to_url: attempt.url().to_string(),
                    status: attempt.status().as_u16(),
                });
            }
        }
        attempt.follow()
    });
    let mut client_builder = Client::builder().redirect(redirect_policy);
    if let Some(timeout_ms) = request.connect_timeout_ms {
        client_builder = client_builder.connect_timeout(Duration::from_millis(timeout_ms));
    }
    if let Some(timeout_ms) = request.read_timeout_ms {
        client_builder = client_builder.read_timeout(Duration::from_millis(timeout_ms));
    }
    // The app-level timeout is an upper bound. KMP sends its ordinary 15-second
    // client default as though it were explicit, so treat that exact default as
    // unset; any other shorter source deadline stays in effect.
    let network_timeout_ms = NETWORK_TIMEOUT_SECONDS
        .load(Ordering::Relaxed)
        .saturating_mul(1000);
    let call_timeout_ms = match request.call_timeout_ms.filter(|timeout_ms| *timeout_ms > 0) {
        Some(timeout_ms) if timeout_ms != DEFAULT_NETWORK_TIMEOUT_SECONDS * 1000 => {
            timeout_ms.min(network_timeout_ms)
        }
        _ => network_timeout_ms,
    };
    client_builder = client_builder.timeout(Duration::from_millis(call_timeout_ms));
    if let Some(partition) = cookie_partition.as_ref() {
        client_builder = client_builder.cookie_provider(Arc::clone(partition));
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

    let request = request_builder
        .build()
        .map_err(|error| format!("Failed to build HTTP request: {error}"))?;
    tokio::time::timeout(
        Duration::from_millis(call_timeout_ms),
        send_with_safe_retries(
            &client,
            request,
            retryable_request,
            requested_url,
            redirects,
        ),
    )
    .await
    .map_err(|_| format!("HTTP request timed out after {call_timeout_ms} ms"))?
}

/// Cookie state is shared by the KMP CookieStore at the same normalized domain scope.
/// reqwest's Jar cannot remove one domain, so each scope owns a separate Jar instance.
fn cookie_jar(scope: &str) -> Result<Arc<Jar>, String> {
    let mut jars = COOKIE_JARS
        .lock()
        .map_err(|_| "Source cookie jar registry is unavailable".to_owned())?;
    Ok(Arc::clone(
        jars.entry(scope.to_owned())
            .or_insert_with(|| Arc::new(Jar::default())),
    ))
}

fn cookie_scope(request: &SourceHttpRequest, url: &Url) -> Result<String, String> {
    let scope = request
        .cookie_scope
        .as_deref()
        .filter(|scope| !scope.trim().is_empty())
        .or_else(|| url.host_str())
        .ok_or_else(|| "Cookie-enabled request has no cookie scope".to_owned())?;
    validate_cookie_scope(scope, url)?;
    Ok(scope.to_owned())
}

fn validate_cookie_scope(scope: &str, url: &Url) -> Result<(), String> {
    if scope.len() > 255 || scope.contains('/') || scope.contains('\\') {
        return Err("Cookie-enabled request has an invalid cookie scope".to_owned());
    }
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Cookie-enabled request requires an HTTP(S) URL with a host".to_owned());
    }
    Ok(())
}

/// Read the private source cookie jar only for this HTTP(S) URL. `Jar::cookies` applies the
/// cookie domain, path, secure and expiry rules, so this never returns cookies for other hosts.
#[cfg(all(
    feature = "desktop",
    not(any(target_os = "android", target_os = "ios"))
))]
pub(crate) fn cookie_header_for_scope(
    source_url: &str,
    scope: &str,
) -> Result<Option<String>, String> {
    let url =
        Url::parse(source_url).map_err(|error| format!("Invalid source cookie URL: {error}"))?;
    validate_cookie_scope(scope, &url)?;
    let jar = COOKIE_JARS
        .lock()
        .map_err(|_| "Source cookie jar registry is unavailable".to_owned())?
        .get(scope)
        .cloned();
    let Some(cookie) = jar.and_then(|jar| jar.cookies(&url)) else {
        return Ok(None);
    };
    let value = cookie
        .to_str()
        .map_err(|_| "Source cookie jar returned an invalid Cookie header".to_owned())?;
    Ok((!value.trim().is_empty()).then(|| value.to_owned()))
}

/// Remove only the jar associated with one KMP-normalized cookie domain.
/// Requests holding the old Arc can finish, but cannot repopulate the registry entry.
pub(crate) fn clear_cookie_jar(scope: &str) -> Result<(), String> {
    let mut jars = COOKIE_JARS
        .lock()
        .map_err(|_| "Source cookie jar registry is unavailable".to_owned())?;
    jars.remove(scope);
    Ok(())
}

pub(crate) fn clear_all_cookie_jars() -> Result<(), String> {
    let mut jars = COOKIE_JARS
        .lock()
        .map_err(|_| "Source cookie jar registry is unavailable".to_owned())?;
    jars.clear();
    Ok(())
}

/// Report whether reqwest has any cookie applicable to this source URL without exposing values.
pub(crate) fn has_cookie_jar_cookies(scope: &str, source_url: &str) -> Result<bool, String> {
    let url =
        Url::parse(source_url).map_err(|error| format!("Invalid source cookie URL: {error}"))?;
    let jar = COOKIE_JARS
        .lock()
        .map_err(|_| "Source cookie jar registry is unavailable".to_owned())?
        .get(scope)
        .cloned();
    Ok(jar.is_some_and(|jar| {
        jar.cookies(&url)
            .is_some_and(|cookies| !cookies.as_bytes().is_empty())
    }))
}

async fn send_with_safe_retries(
    client: &Client,
    request: Request,
    retryable: bool,
    requested_url: String,
    redirects: Arc<Mutex<Vec<SourceHttpRedirect>>>,
) -> Result<SourceHttpResponse, String> {
    let attempts = if retryable { MAX_SAFE_RETRIES + 1 } else { 1 };
    let mut request = Some(request);
    'attempts: for attempt in 0..attempts {
        let attempt_request = if retryable {
            request
                .as_ref()
                .and_then(Request::try_clone)
                .ok_or_else(|| "Cannot replay a bodyless HTTP request".to_owned())?
        } else {
            request
                .take()
                .ok_or_else(|| "HTTP request was already consumed".to_owned())?
        };
        let mut response = match client.execute(attempt_request).await {
            Ok(response) => response,
            Err(error)
                if retryable && attempt + 1 < attempts && is_retryable_transport_error(&error) =>
            {
                tokio::time::sleep(retry_delay(None, attempt)).await;
                continue;
            }
            Err(error) => return Err(format!("HTTP request failed: {error}")),
        };
        if retryable && attempt + 1 < attempts && is_retryable_status(response.status().as_u16()) {
            let delay = retry_delay(response.headers().get("retry-after"), attempt);
            drop(response);
            tokio::time::sleep(delay).await;
            continue;
        }

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
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SOURCE_HTTP_RESPONSE_BYTES as u64)
        {
            return Err(format!(
                "HTTP response exceeds the {} MiB host limit",
                MAX_SOURCE_HTTP_RESPONSE_BYTES / (1024 * 1024)
            ));
        }
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let Some(next_len) = body.len().checked_add(chunk.len()) else {
                        return Err("HTTP response body size overflow".to_owned());
                    };
                    if next_len > MAX_SOURCE_HTTP_RESPONSE_BYTES {
                        return Err(format!(
                            "HTTP response exceeds the {} MiB host limit",
                            MAX_SOURCE_HTTP_RESPONSE_BYTES / (1024 * 1024)
                        ));
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error)
                    if retryable
                        && attempt + 1 < attempts
                        && is_retryable_transport_error(&error) =>
                {
                    tokio::time::sleep(retry_delay(None, attempt)).await;
                    continue 'attempts;
                }
                Err(error) => {
                    return Err(format!("Failed to read HTTP response body: {error}"));
                }
            }
        }
        let redirect_trace = redirects
            .lock()
            .map_err(|_| "HTTP redirect trace lock is poisoned".to_owned())?
            .drain(..)
            .collect();
        return Ok(SourceHttpResponse {
            requested_url,
            final_url,
            status: status.as_u16(),
            reason,
            redirects: redirect_trace,
            headers: response_headers,
            body_base64: BASE64.encode(body),
        });
    }
    Err("HTTP retry loop ended without a response".to_owned())
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    if error.is_decode() {
        return false;
    }
    if error.is_timeout() || error.is_body() {
        return true;
    }
    if !error.is_connect() {
        return false;
    }

    // Retry connect failures only when reqwest exposes a typed transient socket
    // error. TLS/certificate and other opaque connect failures are deliberately
    // left to the source rule caller unchanged.
    let mut cause = StdError::source(error);
    while let Some(current) = cause {
        if let Some(io_error) = current.downcast_ref::<std::io::Error>() {
            return matches!(
                io_error.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::Interrupted
            );
        }
        cause = current.source();
    }
    false
}

fn retry_delay(retry_after: Option<&reqwest::header::HeaderValue>, attempt: usize) -> Duration {
    if let Some(delay) = retry_after
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
    {
        return delay.min(MAX_RETRY_AFTER);
    }
    Duration::from_millis(200u64.saturating_mul(1u64 << attempt.min(8))).min(MAX_RETRY_AFTER)
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    parse_http_date_delay(value)
}

fn parse_http_date_delay(value: &str) -> Option<Duration> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 6 || !parts[0].ends_with(',') || parts[5] != "GMT" {
        return None;
    }
    let day = parts[1].parse::<u32>().ok()?;
    let month = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = parts[3].parse::<i64>().ok()?;
    let time = parts[4].split(':').collect::<Vec<_>>();
    if time.len() != 3 {
        return None;
    }
    let hour = time[0].parse::<u32>().ok()?;
    let minute = time[1].parse::<u32>().ok()?;
    let second = time[2].parse::<u32>().ok()?;
    if !(1970..=9999).contains(&year) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > days_in_month {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let timestamp = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3_600)?
        .checked_add(i64::from(minute) * 60)?
        .checked_add(i64::from(second))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let target = u64::try_from(timestamp).ok()?;
    Some(Duration::from_secs(target.saturating_sub(now)))
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let adjusted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
