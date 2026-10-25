//! 桌面私有 WebView：页面结果经一次性导航回收，不授予应用命令权限。
use crate::{
    browser::{Mode, Request, Response},
    *,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder, webview::PageLoadEvent};
static APP: OnceLock<AppHandle> = OnceLock::new();
static LOGINS: LazyLock<Mutex<HashMap<String, Session>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
struct Session {
    window: WebviewWindow,
    request: SourceLoginWebStart,
    url: reqwest::Url,
}
pub fn install(app: AppHandle) {
    let _ = APP.set(app);
}
fn app() -> Result<&'static AppHandle> {
    APP.get().ok_or_else(|| "浏览器宿主未注册".into())
}
fn url(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("浏览器地址无效".into());
    }
    Ok(url)
}
fn cookies(window: &WebviewWindow, url: &reqwest::Url) -> Result<String> {
    let value = window
        .cookies_for_url(url.clone())
        .map_err(|e| e.to_string())?
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ");
    if value.len() > 256 * 1024 {
        return Err("浏览器 Cookie 超限".into());
    }
    Ok(value)
}
fn seed(window: &WebviewWindow, url: &reqwest::Url, value: &str) -> Result<()> {
    if value.len() > 256 * 1024 {
        return Err("浏览器 Cookie 超限".into());
    }
    for pair in value.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        if name.trim().is_empty() {
            continue;
        }
        window
            .set_cookie(
                tauri::webview::Cookie::build((name.trim().to_owned(), value.trim().to_owned()))
                    .domain(url.host_str().unwrap_or_default().to_owned())
                    .path("/")
                    .secure(url.scheme() == "https")
                    .http_only(true)
                    .build(),
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub(crate) fn start(request: SourceLoginWebStart) -> Result<SourceLoginWebStarted> {
    let target = url(&request.login_url)?;
    if request.session_id.is_empty()
        || request.session_id.len() > 200
        || request.source_id.len() > 2048
    {
        return Err("登录会话无效".into());
    }
    if LOGINS
        .lock()
        .map_err(|_| "登录会话锁不可用")?
        .contains_key(&request.session_id)
    {
        return Err("登录会话已存在".into());
    }
    let window = WebviewWindowBuilder::new(
        app()?,
        format!("source-login-{}", uuid::Uuid::new_v4().simple()),
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .title("书源登录")
    .incognito(true)
    .inner_size(900., 700.)
    .build()
    .map_err(|e| e.to_string())?;
    let result = seed(&window, &target, &request.cookie_header)
        .and_then(|_| window.navigate(target.clone()).map_err(|e| e.to_string()));
    if let Err(error) = result {
        let _ = window.close();
        return Err(error);
    }
    let id = request.session_id.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Ok(mut sessions) = LOGINS.lock() {
                sessions.remove(&id);
            }
        }
    });
    LOGINS.lock().map_err(|_| "登录会话锁不可用")?.insert(
        request.session_id.clone(),
        Session {
            window,
            request,
            url: target,
        },
    );
    Ok(SourceLoginWebStarted { started: true })
}
fn take(request: SourceLoginWebSession) -> Result<Option<Session>> {
    let mut sessions = LOGINS.lock().map_err(|_| "登录会话锁不可用")?;
    if sessions
        .get(&request.session_id)
        .is_some_and(|s| s.request.source_id != request.source_id)
    {
        return Err("登录会话不属于此书源".into());
    }
    Ok(sessions.remove(&request.session_id))
}
pub(crate) fn complete(request: SourceLoginWebSession) -> Result<SourceLoginWebCapture> {
    let session = take(request)?.ok_or("登录会话已结束")?;
    let result = cookies(&session.window, &session.url);
    let _ = session.window.close();
    Ok(SourceLoginWebCapture {
        login_url: session.request.login_url,
        source_revision: session.request.source_revision,
        restore_epoch: session.request.restore_epoch,
        cookie_header: result?,
    })
}
pub(crate) fn cancel(request: SourceLoginWebSession) -> Result<SourceLoginWebCancelled> {
    let session = take(request)?;
    let cancelled = session.is_some();
    if let Some(s) = session {
        let _ = s.window.close();
    }
    Ok(SourceLoginWebCancelled { cancelled })
}

pub(crate) fn render(request: Request) -> Result<Response> {
    let target = url(&request.base_url)?;
    if request.method != "GET" {
        return Err("浏览器仅接受 GET 页面".into());
    }
    if request.headers.iter().any(|h| {
        !h.name.eq_ignore_ascii_case("cookie") && !h.name.eq_ignore_ascii_case("user-agent")
    }) {
        return Err("桌面浏览器仅支持 User-Agent 和 Cookie 请求头".into());
    }
    let callback = format!("https://foliver.invalid/{}/", uuid::Uuid::new_v4().simple());
    let (send, receive) = mpsc::channel::<Result<(bool, String)>>();
    let callback_copy = callback.clone();
    let send_close = send.clone();
    let ready = Arc::new(AtomicBool::new(false));
    let ready_page = ready.clone();
    let html=serde_json::to_string(&format!("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; img-src data:\"><base href=\"{}\">{}",target.as_str().replace('&',"&amp;").replace('"',"&quot;"),request.html)).map_err(|e|e.to_string())?;
    let script = serde_json::to_string(&request.java_script).map_err(|e| e.to_string())?;
    let callback_js = serde_json::to_string(&callback).unwrap();
    let offline = request.mode == Mode::Offline;
    let delay = request.delay_time_ms;
    // 分块导航避免把整页内容塞进受长度限制的单个 URL。
    let evaluate = format!(
        r#"(async()=>{{try{{{prepare}await new Promise(r=>setTimeout(r,{delay}));let value={script} ? await (0,eval)({script}) : document.documentElement.outerHTML;let result=JSON.stringify({{body:String(value??''),finalUrl:location.href}});send(result);}}catch(e){{send(JSON.stringify({{error:String(e)}}));}}function send(value){{const data=btoa(unescape(encodeURIComponent(value)));let index=0;window.__foliverNext=()=>{{const chunk=data.slice(index,index+60000);index+=60000;location.replace({callback_js}+'?done='+(index>=data.length?'1':'0')+'&chunk='+encodeURIComponent(chunk));}};window.__foliverNext();}}}})()"#,
        prepare = if offline {
            format!("document.open();document.write({html});document.close();")
        } else {
            String::new()
        }
    );
    let mut builder = WebviewWindowBuilder::new(
        app()?,
        format!("source-browser-{}", uuid::Uuid::new_v4().simple()),
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .title(if request.title.is_empty() {
        "书源页面"
    } else {
        &request.title
    })
    .visible(request.mode == Mode::Interactive)
    .incognito(true)
    .on_navigation(move |url| {
        if url.as_str().starts_with(&callback_copy) {
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            let _ = send.send(Ok((
                query.get("done").is_some_and(|v| v == "1"),
                query.get("chunk").cloned().unwrap_or_default(),
            )));
            return false;
        }
        matches!(url.scheme(), "http" | "https") || url.as_str() == "about:blank"
    })
    .on_page_load(move |window, payload| {
        if payload.event() != PageLoadEvent::Finished {
            return;
        }
        if !offline && payload.url().as_str() == "about:blank" {
            return;
        }
        if ready_page.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = window.eval(&evaluate);
    })
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
    if let Some(header) = request
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("user-agent"))
    {
        builder = builder.user_agent(&header.value);
    }
    let window = builder.build().map_err(|e| e.to_string())?;
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = send_close.send(Err("浏览器已关闭".into()));
        }
    });
    let result = (|| {
        if !offline {
            for header in &request.headers {
                if header.name.eq_ignore_ascii_case("cookie") {
                    seed(&window, &target, &header.value)?;
                }
            }
            if request.use_cookie_jar {
                if let Some(header) = crate::http::cookie_header_for_scope(
                    target.as_str(),
                    request.cookie_scope.as_deref().unwrap_or_default(),
                )? {
                    seed(&window, &target, &header)?;
                }
            }
            window.navigate(target.clone()).map_err(|e| e.to_string())?;
        }
        if request.mode == Mode::Interactive && !request.wait_for_result {
            return Ok(Response {
                capability_id: request.capability_id.clone(),
                final_url: request.base_url.clone(),
                body: String::new(),
                cookie_header: String::new(),
            });
        }
        let until = Instant::now() + Duration::from_secs(45);
        let mut data = String::new();
        loop {
            if request
                .execution_id
                .as_deref()
                .is_some_and(crate::engine::context::cancelled)
            {
                return Err("浏览器请求已取消".into());
            }
            if Instant::now() > until {
                return Err("浏览器请求超时".into());
            }
            match receive.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => {
                    let (done, chunk) = message?;
                    data.push_str(&chunk);
                    if data.len() > 24 * 1024 * 1024 {
                        return Err("浏览器结果超限".into());
                    }
                    if done {
                        break;
                    }
                    window
                        .eval("window.__foliverNext()")
                        .map_err(|e| e.to_string())?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err("浏览器通道已结束".into()),
            }
        }
        let value: serde_json::Value =
            serde_json::from_slice(&STANDARD.decode(data).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if let Some(error) = value["error"].as_str() {
            return Err(error.into());
        }
        let final_url = if offline {
            request.base_url.clone()
        } else {
            value["finalUrl"].as_str().ok_or("缺少页面地址")?.to_owned()
        };
        Ok(Response {
            capability_id: request.capability_id.clone(),
            final_url,
            body: value["body"].as_str().ok_or("缺少页面内容")?.to_owned(),
            cookie_header: if offline {
                String::new()
            } else {
                cookies(&window, &target)?
            },
        })
    })();
    if request.mode != Mode::Interactive || request.wait_for_result || result.is_err() {
        let _ = window.close();
    }
    result
}
