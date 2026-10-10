//! 为书源页面规则提供隔离的桌面浏览器执行环境。
//! Private desktop renderer for the existing UrlOption.webJs/useWebView path.
//!
//! This module runs UrlOption.webJs in a private incognito browser. Offline mode uses a sanitized
//! about:blank DOM snapshot; dynamic mode loads a real HTTP(S) GET page. Both modes evaluate the
//! rule through Tauri's native callback and destroy the view before returning, without application IPC.

use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, WebviewUrl, WebviewWindowBuilder,
    webview::{NewWindowResponse, PageLoadEvent},
};

const MAX_BROWSER_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_PREPARE_SCRIPT_BYTES: usize = 25 * 1024 * 1024;
const MAX_EVALUATE_SCRIPT_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;
const MAX_DELAY_MS: u64 = 15_000;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const LOGIN_SESSION_TTL: Duration = Duration::from_secs(10 * 60);

static BROWSER_APP: OnceLock<AppHandle> = OnceLock::new();
static LOGIN_WINDOWS: OnceLock<Mutex<HashMap<String, LoginWindow>>> = OnceLock::new();
static LOGIN_START_LOCK: Mutex<()> = Mutex::new(());

struct LoginWindow {
    source_id: String,
    source_revision: u64,
    restore_epoch: u64,
    created_at: Instant,
    login_url: reqwest::Url,
    source_login_url: String,
    window: tauri::WebviewWindow,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserRequest {
    capability_id: String,
    #[serde(default)]
    mode: BrowserMode,
    base_url: String,
    delay_time_ms: u64,
    #[serde(default = "default_browser_method")]
    method: String,
    #[serde(default)]
    headers: Vec<BrowserHeader>,
    #[serde(default)]
    use_cookie_jar: bool,
    #[serde(default)]
    cookie_scope: Option<String>,
    prepare_script: String,
    evaluate_script: String,
}

#[derive(Debug, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BrowserMode {
    #[default]
    OfflineSnapshot,
    DynamicPage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserHeader {
    name: String,
    value: String,
}

fn default_browser_method() -> String {
    "GET".to_owned()
}

pub(crate) fn install_app(app: AppHandle) {
    let _ = BROWSER_APP.set(app);
}

/// Show a dedicated incognito webview for a source's real login page.
///
/// The source URL and initial cookies remain on the Rust side. The view has no application IPC
/// and only HTTP(S) navigation is allowed. Cookies are read only by an explicit completion call.
pub(crate) fn start_web_login(
    app: &AppHandle,
    source_id: &str,
    session_id: &str,
    source_revision: u64,
    restore_epoch: u64,
    login_url: &str,
    initial_cookie_header: &str,
) -> Result<(), String> {
    let _start = LOGIN_START_LOCK
        .lock()
        .map_err(|_| "Private login browser start lock is unavailable".to_owned())?;
    validate_login_session(session_id)?;
    if source_id.is_empty() || source_id.len() > 256 {
        return Err("Source login request has an invalid source ID".to_owned());
    }
    if initial_cookie_header.len() > 256 * 1024
        || initial_cookie_header
            .chars()
            .any(|character| matches!(character, '\r' | '\n'))
    {
        return Err("Stored source cookies exceed the private browser limit".to_owned());
    }
    let source_login_url = login_url.to_owned();
    let login_url = validate_login_url(login_url)?;

    let registry = LOGIN_WINDOWS.get_or_init(|| Mutex::new(HashMap::new()));
    let old_windows = {
        let mut windows = registry
            .lock()
            .map_err(|_| "Private login browser registry is unavailable".to_owned())?;
        let old_ids = windows
            .iter()
            .filter_map(|(id, window)| {
                (window.source_id == source_id || window.created_at.elapsed() >= LOGIN_SESSION_TTL)
                    .then_some(id.clone())
            })
            .collect::<Vec<_>>();
        old_ids
            .into_iter()
            .filter_map(|id| windows.remove(&id).map(|entry| entry.window))
            .collect::<Vec<_>>()
    };
    for window in old_windows {
        let _ = window.close();
    }

    let label = format!("source-login-{session_id}");
    let blank_url = reqwest::Url::parse("about:blank")
        .map_err(|error| format!("Cannot create private login browser blank URL: {error}"))?;
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(blank_url))
        .title("书源网页登录")
        .inner_size(1024.0, 760.0)
        .min_inner_size(480.0, 360.0)
        .incognito(true)
        .on_navigation(|url| {
            url.as_str() == "about:blank"
                || (matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none())
        })
        .build()
        .map_err(|error| format!("Cannot open private source login browser: {error}"))?;

    let registry_for_close = registry;
    let closed_session_id = session_id.to_owned();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Ok(mut windows) = registry_for_close.lock() {
                windows.remove(&closed_session_id);
            }
        }
    });

    if let Err(error) = seed_login_cookies(&window, &login_url, initial_cookie_header) {
        let _ = window.close();
        return Err(error);
    }
    if let Err(error) = window.navigate(login_url.clone()) {
        let _ = window.close();
        return Err(format!("Cannot navigate to the source login page: {error}"));
    }

    registry
        .lock()
        .map_err(|_| "Private login browser registry is unavailable".to_owned())?
        .insert(
            session_id.to_owned(),
            LoginWindow {
                source_id: source_id.to_owned(),
                source_revision,
                restore_epoch,
                created_at: Instant::now(),
                login_url,
                source_login_url,
                window,
            },
        );
    let expire_session_id = session_id.to_owned();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LOGIN_SESSION_TTL).await;
        expire_web_login(&expire_session_id);
    });
    Ok(())
}

/// Capture native cookies (including HttpOnly) and close the temporary login window.
pub(crate) fn complete_web_login(source_id: &str, session_id: &str) -> Result<Value, String> {
    validate_login_session(session_id)?;
    let registry = LOGIN_WINDOWS
        .get()
        .ok_or_else(|| "Private source login browser session has ended".to_owned())?;
    let session = registry
        .lock()
        .map_err(|_| "Private login browser registry is unavailable".to_owned())?
        .remove(session_id)
        .ok_or_else(|| "Private source login browser session has ended".to_owned())?;
    if session.source_id != source_id {
        let _ = session.window.close();
        return Err("Private source login browser session did not match this source".to_owned());
    }
    if session.created_at.elapsed() >= LOGIN_SESSION_TTL {
        let _ = session.window.close();
        return Err("Private source login browser session expired".to_owned());
    }

    let cookies_result = session.window.cookies_for_url(session.login_url.clone());
    let _ = session.window.close();
    let cookies =
        cookies_result.map_err(|error| format!("Cannot read private browser cookies: {error}"))?;
    let cookie_header = cookies
        .iter()
        .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
        .collect::<Vec<_>>()
        .join("; ");
    let cookie_count = cookies.len();
    let login_url = session.source_login_url;
    let source_revision = session.source_revision;
    let restore_epoch = session.restore_epoch;
    if cookie_header.len() > 256 * 1024 {
        return Err("Private browser cookies exceed the supported size".to_owned());
    }
    Ok(json!({
        "loginUrl": login_url,
        "sourceRevision": source_revision,
        "restoreEpoch": restore_epoch,
        "cookieHeader": cookie_header,
        "cookieCount": cookie_count,
    }))
}

fn expire_web_login(session_id: &str) {
    let Some(registry) = LOGIN_WINDOWS.get() else {
        return;
    };
    let session = registry.lock().ok().and_then(|mut windows| {
        if windows
            .get(session_id)
            .is_some_and(|entry| entry.created_at.elapsed() >= LOGIN_SESSION_TTL)
        {
            windows.remove(session_id)
        } else {
            None
        }
    });
    if let Some(session) = session {
        let _ = session.window.close();
    }
}

/// Cancellation removes the native session without reading or importing its temporary cookies.
pub(crate) fn cancel_web_login(source_id: &str, session_id: &str) -> Result<bool, String> {
    validate_login_session(session_id)?;
    let Some(registry) = LOGIN_WINDOWS.get() else {
        return Ok(false);
    };
    let Some(session) = registry
        .lock()
        .map_err(|_| "Private login browser registry is unavailable".to_owned())?
        .remove(session_id)
    else {
        return Ok(false);
    };
    if session.source_id != source_id {
        let registry = LOGIN_WINDOWS.get().expect("registry was initialized above");
        registry
            .lock()
            .map_err(|_| "Private login browser registry is unavailable".to_owned())?
            .insert(session_id.to_owned(), session);
        return Err("Private source login browser session did not match this source".to_owned());
    }
    let _ = session.window.close();
    Ok(true)
}

fn validate_login_session(session_id: &str) -> Result<(), String> {
    if session_id.len() != 32
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Private source login session ID is invalid".to_owned());
    }
    Ok(())
}

fn validate_login_url(value: &str) -> Result<reqwest::Url, String> {
    if value.len() > 8192 {
        return Err("Source login URL exceeds the 8 KiB limit".to_owned());
    }
    let url = reqwest::Url::parse(value).map_err(|_| "Source login URL is invalid".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Source login URL must be an HTTP(S) URL without credentials".to_owned());
    }
    Ok(url)
}

fn seed_login_cookies(
    window: &tauri::WebviewWindow,
    login_url: &reqwest::Url,
    cookie_header: &str,
) -> Result<(), String> {
    let Some(host) = login_url.host_str() else {
        return Err("Source login URL has no host".to_owned());
    };
    for pair in cookie_header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || name
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == b';')
        {
            continue;
        }
        let cookie = tauri::webview::Cookie::build((name.to_owned(), value.trim().to_owned()))
            .domain(host.to_owned())
            .path("/")
            .secure(login_url.scheme() == "https")
            .http_only(true)
            .build();
        window
            .set_cookie(cookie)
            .map_err(|error| format!("Cannot seed private browser cookies: {error}"))?;
    }
    Ok(())
}

pub(crate) fn browser_json(request_json: &str) -> String {
    let result = std::panic::catch_unwind(|| {
        if request_json.len() > MAX_BROWSER_REQUEST_BYTES {
            return Err("Private browser request exceeds its input-size limit".to_owned());
        }
        let request: BrowserRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("Invalid private browser request: {error}"))?;
        validate(&request)?;
        match request.mode {
            BrowserMode::OfflineSnapshot => render_snapshot(&request),
            BrowserMode::DynamicPage => render_dynamic_page(&request),
        }
    })
    .unwrap_or_else(|_| Err("Private source browser renderer panicked".to_owned()));

    match result {
        Ok(value) => json!({ "ok": true, "value": value }).to_string(),
        Err(error) => json!({ "ok": false, "error": error }).to_string(),
    }
}

fn validate(request: &BrowserRequest) -> Result<(), String> {
    if request.capability_id.len() != 32
        || !request
            .capability_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Private browser capability ID is invalid".to_owned());
    }
    if request.prepare_script.len() > MAX_PREPARE_SCRIPT_BYTES {
        return Err("Private browser document script exceeds its limit".to_owned());
    }
    if request.evaluate_script.len() > MAX_EVALUATE_SCRIPT_BYTES {
        return Err("Private browser evaluation script exceeds its limit".to_owned());
    }
    if request.base_url.len() > 8192 {
        return Err("Private browser base URL exceeds the 8 KiB limit".to_owned());
    }
    if request.delay_time_ms > MAX_DELAY_MS {
        return Err("Private browser delay exceeds the 15000 ms limit".to_owned());
    }
    let url = reqwest::Url::parse(&request.base_url)
        .map_err(|_| "Private browser base URL is invalid".to_owned())?;
    if request.mode == BrowserMode::DynamicPage {
        if !is_safe_dynamic_url(&url) {
            return Err(
                "Private browser base URL must be a public HTTP(S) URL without credentials"
                    .to_owned(),
            );
        }
        if !request.method.eq_ignore_ascii_case("GET") {
            return Err("Dynamic source browser currently supports GET only; POST pages use the offline response snapshot".to_owned());
        }
        if request.headers.iter().any(|header| {
            !header.name.eq_ignore_ascii_case("user-agent")
                && !header.name.eq_ignore_ascii_case("cookie")
        }) {
            return Err("Dynamic source browser does not support custom request headers other than User-Agent and Cookie".to_owned());
        }
        let mut names = std::collections::HashSet::new();
        let mut cookie_bytes = 0usize;
        for header in &request.headers {
            let lower_name = header.name.to_ascii_lowercase();
            if !names.insert(lower_name) {
                return Err(format!(
                    "Dynamic source browser received duplicate {} headers",
                    header.name
                ));
            }
            if header.name.eq_ignore_ascii_case("user-agent") && header.value.len() > 4096 {
                return Err("Dynamic source browser User-Agent exceeds the 4 KiB limit".to_owned());
            }
            if header.name.eq_ignore_ascii_case("cookie") {
                cookie_bytes = cookie_bytes.saturating_add(header.value.len());
            }
            if header
                .value
                .chars()
                .any(|character| matches!(character, '\r' | '\n' | '\0'))
            {
                return Err(format!(
                    "Dynamic source browser received an invalid {} header value",
                    header.name
                ));
            }
        }
        if cookie_bytes > 256 * 1024 {
            return Err(
                "Dynamic source browser Cookie header exceeds the 256 KiB limit".to_owned(),
            );
        }
        if request.use_cookie_jar && request.cookie_scope.as_deref().is_none_or(str::is_empty) {
            return Err(
                "Dynamic source browser CookieJar is enabled without a cookie scope".to_owned(),
            );
        }
        if request
            .cookie_scope
            .as_deref()
            .is_some_and(|scope| scope.len() > 255)
        {
            return Err(
                "Dynamic source browser cookie scope exceeds the 255 byte limit".to_owned(),
            );
        }
    } else if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Private browser base URL must be an HTTP(S) URL without credentials".to_owned(),
        );
    }
    Ok(())
}

fn is_safe_dynamic_url(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.ends_with(".home")
        || host.ends_with(".internal")
        || host.ends_with(".arpa")
        || !host.contains('.') && host.parse::<IpAddr>().is_err()
    {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified())
        }
        Ok(IpAddr::V6(ip)) => {
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80)
        }
        Err(_) => true,
    }
}

fn render_snapshot(request: &BrowserRequest) -> Result<Value, String> {
    let app = BROWSER_APP
        .get()
        .ok_or_else(|| "Private source browser app handle has not been installed".to_owned())?;
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let label = format!("source-browser-{}", request.capability_id);
    let navigation_attempted = Arc::new(AtomicBool::new(false));
    let navigation_seen = navigation_attempted.clone();
    let page_started = Arc::new(AtomicBool::new(false));
    let page_started_callback = page_started.clone();
    let (ready_sender, ready_receiver) = mpsc::channel::<Result<String, String>>();
    let prepare_script = request.prepare_script.clone();

    let blank_url = reqwest::Url::parse("about:blank")
        .map_err(|error| format!("Cannot create private browser blank URL: {error}"))?;
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(blank_url))
        .visible(false)
        .focused(false)
        .focusable(false)
        .inner_size(1.0, 1.0)
        .resizable(false)
        .on_navigation(move |url| {
            let allowed = url.as_str() == "about:blank";
            if !allowed {
                navigation_seen.store(true, Ordering::SeqCst);
            }
            allowed
        })
        .on_page_load(move |webview, payload| {
            if payload.event() != PageLoadEvent::Finished
                || payload.url().as_str() != "about:blank"
                || page_started_callback.swap(true, Ordering::SeqCst)
            {
                return;
            }
            let script = prepare_script.clone();
            let sender = ready_sender.clone();
            if let Err(error) = webview.eval_with_callback(script, move |result| {
                let _ = sender.send(Ok(result));
            }) {
                let _ =
                    ready_sender.send(Err(format!("Cannot prepare offline source DOM: {error}")));
            }
        })
        .build()
        .map_err(|error| format!("Cannot create hidden source browser view: {error}"))?;

    let run = (|| {
        let ready_result = recv_until(
            &ready_receiver,
            deadline,
            "private source browser page load",
        )??;
        decode_prepare_result(&ready_result, request)?;
        if navigation_attempted.load(Ordering::SeqCst) {
            return Err("Unsupported browser capability used: page navigation".to_owned());
        }
        if request.delay_time_ms > 0 {
            std::thread::sleep(Duration::from_millis(request.delay_time_ms));
        }

        let (result_sender, result_receiver) = mpsc::channel::<String>();
        window
            .eval_with_callback(request.evaluate_script.clone(), move |result| {
                let _ = result_sender.send(result);
            })
            .map_err(|error| format!("Cannot evaluate source webJs rule: {error}"))?;
        let raw = recv_until(&result_receiver, deadline, "source webJs evaluation")?;
        if navigation_attempted.load(Ordering::SeqCst) {
            return Err("Unsupported browser capability used: page navigation".to_owned());
        }
        decode_result(&raw, request)
    })();

    let _ = window.close();
    run
}

fn render_dynamic_page(request: &BrowserRequest) -> Result<Value, String> {
    let app = BROWSER_APP
        .get()
        .ok_or_else(|| "Private source browser app handle has not been installed".to_owned())?;
    let page_url = reqwest::Url::parse(&request.base_url)
        .map_err(|_| "Private dynamic browser URL is invalid".to_owned())?;
    let mut user_agent = None;
    let mut explicit_cookie = None;
    for header in &request.headers {
        if header.name.eq_ignore_ascii_case("user-agent") {
            user_agent = Some(header.value.as_str());
        } else if header.name.eq_ignore_ascii_case("cookie") {
            explicit_cookie = Some(header.value.as_str());
        }
    }
    let jar_cookie = if request.use_cookie_jar {
        let scope = request
            .cookie_scope
            .as_deref()
            .ok_or_else(|| "Dynamic source browser CookieJar has no scope".to_owned())?;
        crate::source_http::cookie_header_for_scope(&request.base_url, scope)?
    } else {
        None
    };
    let cookie_header = merge_cookie_headers(jar_cookie.as_deref(), explicit_cookie);

    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let label = format!("source-browser-{}", request.capability_id);
    let blocked_navigation = Arc::new(AtomicBool::new(false));
    let blocked_window = Arc::clone(&blocked_navigation);
    let navigation_flag = Arc::clone(&blocked_navigation);
    let (page_sender, page_receiver) = mpsc::channel::<Result<String, String>>();
    let navigation_sender = page_sender.clone();
    let new_window_sender = page_sender.clone();
    let page_started = Arc::new(AtomicBool::new(false));
    let page_started_callback = Arc::clone(&page_started);
    let prepare_script = request.prepare_script.clone();
    let blank_url = reqwest::Url::parse("about:blank")
        .map_err(|error| format!("Cannot create private browser blank URL: {error}"))?;
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(blank_url))
        .visible(false)
        .focused(false)
        .focusable(false)
        .inner_size(1.0, 1.0)
        .resizable(false)
        .incognito(true)
        .initialization_script_for_all_frames(prepare_script)
        .on_navigation(move |url| {
            if url.as_str() == "about:blank" || is_safe_dynamic_url(url) {
                return true;
            }
            navigation_flag.store(true, Ordering::SeqCst);
            let _ = navigation_sender.send(Err(
                "Dynamic source browser blocked navigation to a local or unsafe URL".to_owned(),
            ));
            false
        })
        .on_new_window(move |_, _| {
            blocked_window.store(true, Ordering::SeqCst);
            let _ = new_window_sender.send(Err(
                "Dynamic source browser blocked a new window request".to_owned(),
            ));
            NewWindowResponse::Deny
        })
        .on_page_load(move |_, payload| {
            if payload.event() != PageLoadEvent::Finished
                || payload.url().as_str() == "about:blank"
                || page_started_callback.swap(true, Ordering::SeqCst)
            {
                return;
            }
            let page_url = payload.url().to_string();
            let _ = page_sender.send(Ok(page_url));
        });
    if let Some(user_agent) = user_agent {
        builder = builder.user_agent(user_agent);
    }
    let window = builder
        .build()
        .map_err(|error| format!("Cannot create private dynamic source browser: {error}"))?;

    let run = (|| {
        if let Some(cookie_header) = cookie_header.as_deref() {
            seed_login_cookies(&window, &page_url, cookie_header)?;
        }
        window
            .navigate(page_url.clone())
            .map_err(|error| format!("Cannot load source page in private browser: {error}"))?;
        recv_until(&page_receiver, deadline, "dynamic source page load")??;
        if blocked_navigation.load(Ordering::SeqCst) {
            return Err("Dynamic source browser blocked a page navigation".to_owned());
        }
        if request.delay_time_ms > 0 {
            let delay = Duration::from_millis(request.delay_time_ms)
                .min(deadline.saturating_duration_since(Instant::now()));
            std::thread::sleep(delay);
        }

        let (start_sender, start_receiver) = mpsc::channel::<String>();
        window
            .eval_with_callback(request.evaluate_script.clone(), move |result| {
                let _ = start_sender.send(result);
            })
            .map_err(|error| format!("Cannot start dynamic source webJs rule: {error}"))?;
        let start = recv_until(&start_receiver, deadline, "dynamic source webJs start")?;
        decode_dynamic_started(&start)?;

        let result_key =
            serde_json::to_string(&format!("__legadoDynamicResult_{}", request.capability_id))
                .map_err(|error| format!("Cannot encode private browser result key: {error}"))?;
        let poll_script = format!(
            "(function(){{const state=window[{result_key}];return state&&state.json?state.json:'';}})()"
        );
        loop {
            if blocked_navigation.load(Ordering::SeqCst) {
                return Err(
                    "Dynamic source browser blocked a page navigation or new window".to_owned(),
                );
            }
            let (poll_sender, poll_receiver) = mpsc::channel::<String>();
            window
                .eval_with_callback(poll_script.clone(), move |result| {
                    let _ = poll_sender.send(result);
                })
                .map_err(|error| format!("Cannot read dynamic source browser result: {error}"))?;
            let raw = recv_until(&poll_receiver, deadline, "dynamic source webJs result")?;
            if let Some(result) = decode_dynamic_poll(&raw, request)? {
                return Ok(result);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    })();
    let _ = window.close();
    run
}

fn merge_cookie_headers(jar_header: Option<&str>, explicit_header: Option<&str>) -> Option<String> {
    let mut pairs = Vec::<(String, String)>::new();
    for header in [jar_header, explicit_header].into_iter().flatten() {
        for pair in header.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty()
                || name
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte == b';')
            {
                continue;
            }
            if let Some(existing) = pairs.iter_mut().find(|(existing, _)| existing == name) {
                existing.1 = value.trim().to_owned();
            } else {
                pairs.push((name.to_owned(), value.trim().to_owned()));
            }
        }
    }
    (!pairs.is_empty()).then(|| {
        pairs
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

fn recv_until<T>(
    receiver: &mpsc::Receiver<T>,
    deadline: Instant,
    operation: &str,
) -> Result<T, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(format!(
            "Private source browser timed out during {operation}"
        ));
    }
    receiver
        .recv_timeout(remaining)
        .map_err(|_| format!("Private source browser timed out during {operation}"))
}

fn decode_prepare_result(raw: &str, request: &BrowserRequest) -> Result<(), String> {
    if raw.len() > 8192 {
        return Err("Private browser document preparation returned an oversized result".to_owned());
    }
    let outer: Value = serde_json::from_str(raw)
        .map_err(|_| "Private browser document preparation returned invalid JSON".to_owned())?;
    let payload_json = outer.as_str().ok_or_else(|| {
        "Private browser document preparation returned no capability envelope".to_owned()
    })?;
    let payload: Value = serde_json::from_str(payload_json).map_err(|_| {
        "Private browser document preparation returned an invalid envelope".to_owned()
    })?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("Offline source document could not be prepared".to_owned());
    }
    let capability_id = payload
        .get("value")
        .and_then(|value| value.get("capabilityId"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "Private browser document preparation returned no capability ID".to_owned()
        })?;
    if capability_id != request.capability_id {
        return Err("Private browser document preparation did not match the request".to_owned());
    }
    Ok(())
}

fn decode_dynamic_started(raw: &str) -> Result<(), String> {
    if raw.len() > 8192 {
        return Err("Private dynamic browser start returned an oversized result".to_owned());
    }
    let outer: Value = serde_json::from_str(raw)
        .map_err(|_| "Private dynamic browser start returned invalid JSON".to_owned())?;
    let payload_json = outer
        .as_str()
        .ok_or_else(|| "Private dynamic browser did not return a start envelope".to_owned())?;
    let payload: Value = serde_json::from_str(payload_json)
        .map_err(|_| "Private dynamic browser returned an invalid start envelope".to_owned())?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Dynamic source webJs could not start")
            .to_owned());
    }
    Ok(())
}

fn decode_dynamic_poll(raw: &str, request: &BrowserRequest) -> Result<Option<Value>, String> {
    if raw.len() > MAX_RESULT_BYTES + 2048 {
        return Err("Private browser result exceeds the 4 MiB limit".to_owned());
    }
    let outer: Value = serde_json::from_str(raw).map_err(|_| {
        "Dynamic source browser returned invalid JSON while waiting for webJs".to_owned()
    })?;
    let Some(payload_json) = outer.as_str() else {
        if outer.is_null() {
            return Ok(None);
        }
        return Err("Dynamic source browser returned an invalid result state".to_owned());
    };
    if payload_json.is_empty() {
        return Ok(None);
    }
    if payload_json.len() > MAX_RESULT_BYTES + 1024 {
        return Err("Private browser result exceeds the 4 MiB limit".to_owned());
    }
    let payload: Value = serde_json::from_str(payload_json).map_err(|_| {
        "Dynamic source browser returned an invalid webJs result envelope".to_owned()
    })?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Source webJs rule failed")
            .to_owned());
    }
    let wrapped = serde_json::to_string(payload_json)
        .map_err(|error| format!("Cannot decode private browser result: {error}"))?;
    decode_result(&wrapped, request).map(Some)
}

fn decode_result(raw: &str, request: &BrowserRequest) -> Result<Value, String> {
    if raw.len() > MAX_RESULT_BYTES + 1024 {
        return Err("Private browser result exceeds the 4 MiB limit".to_owned());
    }
    let outer: Value = serde_json::from_str(raw)
        .map_err(|error| format!("Private browser returned invalid JSON: {error}"))?;
    let payload_json = outer
        .as_str()
        .ok_or_else(|| "Private browser returned a non-string capability envelope".to_owned())?;
    if payload_json.len() > MAX_RESULT_BYTES + 1024 {
        return Err("Private browser result exceeds the 4 MiB limit".to_owned());
    }
    let payload: Value = serde_json::from_str(payload_json)
        .map_err(|error| format!("Private browser returned an invalid result envelope: {error}"))?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Private browser rule failed")
            .to_owned());
    }
    let result = payload
        .get("value")
        .and_then(Value::as_object)
        .ok_or_else(|| "Private browser result has no value".to_owned())?;
    let capability_id = result
        .get("capabilityId")
        .and_then(Value::as_str)
        .ok_or_else(|| "Private browser result has no capability ID".to_owned())?;
    if capability_id != request.capability_id {
        return Err("Private browser result capability did not match the request".to_owned());
    }
    let final_url = result
        .get("finalUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| "Private browser result has no final URL".to_owned())?;
    if request.mode == BrowserMode::OfflineSnapshot {
        if final_url != request.base_url {
            return Err("Private browser snapshot changed its final URL".to_owned());
        }
    } else {
        let url = reqwest::Url::parse(final_url)
            .map_err(|_| "Private dynamic browser returned an invalid final URL".to_owned())?;
        if final_url.len() > 8192 || !is_safe_dynamic_url(&url) {
            return Err(
                "Private dynamic browser returned a non-public HTTP(S) final URL".to_owned(),
            );
        }
    }
    let body = result
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| "Private browser result has no body".to_owned())?;
    if body.len() > MAX_RESULT_BYTES {
        return Err("Private browser result exceeds the 4 MiB limit".to_owned());
    }
    Ok(json!({
        "capabilityId": capability_id,
        "finalUrl": final_url,
        "body": body,
    }))
}
