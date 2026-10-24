//! 统一登录入口：Rust 执行规则、保存登录数据，Vue 只收取表单和显示结果。

use crate::support::validate_id;
use tauri_plugin_source_engine::{LoginRequest, LoginAction as EngineLoginAction};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

pub(crate) struct Login {
    pub(crate) sources: Arc<crate::source_catalog::SourceCatalog>,
    pub(crate) executor: Arc<dyn crate::source_engine::SourceExecutor>,
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(crate) app: Option<tauri::AppHandle>,
    login: tokio::sync::watch::Sender<Option<Arc<LoginSession>>>,
    backup: Arc<crate::backup::state::Operations>,
}
impl Login {
    pub(crate) fn new(
        sources: Arc<crate::source_catalog::SourceCatalog>,
        executor: Arc<dyn crate::source_engine::SourceExecutor>,
        backup: Arc<crate::backup::state::Operations>,
    ) -> Self {
        Self {
            sources,
            executor,
            backup,
            login: tokio::sync::watch::channel(None).0,
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            app: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LoginAction {
    Open,
    Submit {
        #[serde(default)]
        fields: HashMap<String, String>,
        button: Option<usize>,
    },
    Cancel,
    Clear,
}

pub(crate) struct LoginSession {
    source_id: String,
    id: String,
    cancel: CancellationToken,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn login(
    source_id: String,
    action: LoginAction,
    service: tauri::State<'_, Arc<Login>>,
) -> Result<(), String> {
    service.login(&source_id, action).await
}

impl Login {
    pub(crate) async fn cancel_login(&self) -> Result<(), String> {
        let Some(session) = self.login.send_replace(None) else {
            return Ok(());
        };
        self.close_login(session).await
    }

    async fn close_login(&self, session: Arc<LoginSession>) -> Result<(), String> {
        session.cancel.cancel();
        #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
        if let Some(app) = &self.app {
            let app = app.clone();
            tokio::task::spawn_blocking(move || close_browser(&app, &session))
                .await
                .map_err(|error| error.to_string())??;
        }
        Ok(())
    }

    async fn login(&self, source_id: &str, action: LoginAction) -> Result<(), String> {
        validate_id(source_id, "sourceId")?;
        if matches!(action, LoginAction::Cancel) {
            if self
                .login
                .borrow()
                .as_ref()
                .is_some_and(|session| session.source_id == source_id)
            {
                self.cancel_login().await?;
            }
            return Ok(());
        }
        let session = if matches!(action, LoginAction::Open) {
            let session = Arc::new(LoginSession {
                source_id: source_id.into(),
                id: uuid::Uuid::new_v4().simple().to_string(),
                cancel: CancellationToken::new(),
            });
            // 先替换再清理，旧请求不能覆盖新会话。
            if let Some(previous) = self.login.send_replace(Some(session.clone())) {
                self.close_login(previous).await?;
            }
            session
        } else {
            self.login
                .borrow()
                .clone()
                .filter(|session| session.source_id == source_id)
                .ok_or("请重新打开登录界面")?
        };
        let operation = async {
            // 只使用现有备份屏障，不持有书源锁等待规则或网络。
            let _operation = self.backup.operation_read().await;
            let revision = self.sources.revision(source_id).await;
            let epoch = self
                .backup
                .restore_epoch
                .load(std::sync::atomic::Ordering::SeqCst);
            let source = self.sources.get(source_id).await?;
            if crate::rss::is_rss_source(&source.source) {
                return Err("RSS sources do not support book-source login".into());
            }
            let kind = match &action {
                LoginAction::Open => "open",
                LoginAction::Submit { .. } => "submit",
                LoginAction::Clear => "clear",
                LoginAction::Cancel => unreachable!(),
            };
            let engine_action = match action {
                LoginAction::Open => EngineLoginAction::Open,
                LoginAction::Clear => EngineLoginAction::Clear,
                LoginAction::Submit { fields, button } => {
                    validate_source_login_credentials(&fields)?;
                    EngineLoginAction::Submit { fields, button }
                }
                LoginAction::Cancel => unreachable!(),
            };
            let mut request = LoginRequest { source: (&source.source).try_into()?, action: engine_action };
            let mut response = self.executor.login(request.clone(), session.cancel.clone()).await?;
            let web_login = response.web.is_some();
            if let Some(web) = response.web.clone() {
                if kind == "open" {
                    self.open_login_browser(session.clone(), web, revision, epoch).await?;
                } else {
                    let captured = self.complete_login_browser(session.clone()).await?;
                    if captured.source_revision != self.sources.revision(source_id).await
                        || captured.restore_epoch != self.backup.restore_epoch.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        return Err("书源或备份已变化，请重新打开登录页".into());
                    }
                    validate_login_cookies(&captured.cookie_header)?;
                    request.action = EngineLoginAction::ImportCookies {
                        url: captured.login_url, cookies: captured.cookie_header,
                    };
                    response = self.executor.login(request, session.cancel.clone()).await?;
                }
            }
            let mut result = serde_json::to_value(response).map_err(|error| error.to_string())?;
            if web_login { result["webOpen"] = json!(kind == "open"); }
            let mut update = sanitize_source_login_result(result)?;
            update["sourceId"] = json!(source_id);
            if session.cancel.is_cancelled() {
                return Err("登录已取消".into());
            }
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            if let Some(app) = &self.app {
                use tauri::Emitter;
                app.emit("login", update)
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        };
        tokio::select! {
            biased;
            _ = session.cancel.cancelled() => Err("登录已取消".into()),
            result = operation => result,
        }
    }
}


fn validate_login_cookies(cookies: &str) -> Result<(), String> {
    if cookies.len() > 256 * 1024 || cookies.contains(['\r', '\n']) {
        return Err("Private browser cookies exceed the supported size".into());
    }
    Ok(())
}

// 原生插件是同步桥接，放到阻塞线程执行，避免占用 iOS 主线程和异步执行器。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl Login {
    async fn open_login_browser(
        &self,
        session: Arc<LoginSession>,
        info: tauri_plugin_source_engine::LoginWeb,
        revision: u64,
        epoch: u64,
    ) -> Result<(), String> {
        let url = info.url;
        let parsed = reqwest::Url::parse(&url).map_err(|error| error.to_string())?;
        if url.len() > 8192
            || !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("Source login URL must be an HTTP(S) URL without credentials".into());
        }
        let cookies = info.cookie_header;
        validate_login_cookies(&cookies)?;
        let app = self.app.clone().ok_or("Application is unavailable")?;
        tokio::task::spawn_blocking(move || {
            if session.cancel.is_cancelled() {
                return Err("登录已取消".into());
            }
            use tauri_plugin_source_engine::{SourceEngineExt, SourceLoginWebStart};
            let started = app.source_engine().start_web_login(SourceLoginWebStart {
                source_id: session.source_id.clone(), session_id: session.id.clone(),
                source_revision: revision, restore_epoch: epoch,
                login_url: url, cookie_header: cookies,
            }).map_err(|error| error.to_string())?;
            if !started.started { return Err("Native login browser did not start".into()); }
            // 取消可能早于原生窗口创建完成，创建者必须补做清理。
            if session.cancel.is_cancelled() {
                close_browser(&app, &session)?;
            }
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    async fn complete_login_browser(&self, session: Arc<LoginSession>) -> Result<tauri_plugin_source_engine::SourceLoginWebCapture, String> {
        let app = self.app.clone().ok_or("Application is unavailable")?;
        tokio::task::spawn_blocking(move || {
            use tauri_plugin_source_engine::{SourceEngineExt, SourceLoginWebSession};
            app.source_engine().complete_web_login(SourceLoginWebSession {
                source_id: session.source_id.clone(), session_id: session.id.clone(),
            }).map_err(|error| error.to_string())
        }).await.map_err(|error| error.to_string())?
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
fn close_browser(app: &tauri::AppHandle, session: &LoginSession) -> Result<(), String> {
    use tauri_plugin_source_engine::{SourceEngineExt, SourceLoginWebSession};
    app.source_engine().cancel_web_login(SourceLoginWebSession {
        source_id: session.source_id.clone(), session_id: session.id.clone(),
    }).map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(not(any(feature = "desktop", feature = "mobile-runtime")))]
impl Login {
    async fn open_login_browser(
        &self,
        _: Arc<LoginSession>,
        _: tauri_plugin_source_engine::LoginWeb,
        _: u64,
        _: u64,
    ) -> Result<(), String> {
        Err("Web login is unavailable".into())
    }
    async fn complete_login_browser(&self, _: Arc<LoginSession>) -> Result<tauri_plugin_source_engine::SourceLoginWebCapture, String> {
        Err("Web login is unavailable".into())
    }
}

pub(crate) fn sanitize_source_login_form(form: &Value) -> Result<Value, String> {
    let can_login = form
        .get("canLogin")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Source engine returned an invalid login form".to_owned())?;
    let raw_fields = form["fields"]
        .as_array()
        .ok_or_else(|| "Source engine returned an invalid login form".to_owned())?;
    let raw_actions = form["actions"]
        .as_array()
        .ok_or_else(|| "Source engine returned an invalid login form".to_owned())?;
    if raw_fields.len() > 64 || raw_actions.len() > 64 {
        return Err("Source login form exceeds the supported field limit".to_owned());
    }

    let mut names = HashSet::new();
    let mut fields = Vec::with_capacity(raw_fields.len());
    for field in raw_fields {
        let name = field
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty() && name.len() <= 128)
            .ok_or_else(|| "Source engine returned an invalid login field".to_owned())?;
        if !names.insert(name.to_owned()) {
            return Err("Source login form has duplicate field names".to_owned());
        }
        let field_type = field
            .get("type")
            .and_then(Value::as_str)
            .filter(|field_type| matches!(*field_type, "text" | "password" | "select" | "toggle"))
            .ok_or_else(|| "Source engine returned an unsupported login field".to_owned())?;
        let mut safe_field = json!({
            "name": name,
            "label": name,
            "type": field_type,
            "password": field_type == "password",
        });
        if field_type == "select" {
            let choices = field
                .get("choices")
                .and_then(Value::as_array)
                .ok_or_else(|| "Source engine returned invalid login choices".to_owned())?;
            if choices.len() > 64 {
                return Err("Source login form has too many choices".to_owned());
            }
            let choices = choices
                .iter()
                .map(|choice| {
                    choice
                        .as_str()
                        .filter(|choice| choice.len() <= 256)
                        .map(str::to_owned)
                        .ok_or_else(|| "Source engine returned invalid login choices".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            safe_field["choices"] = json!(choices);
        }
        fields.push(safe_field);
    }

    let mut action_ids = HashSet::new();
    let mut actions = Vec::with_capacity(raw_actions.len());
    for action in raw_actions {
        let id = action
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id <= i32::MAX as u64)
            .ok_or_else(|| "Source engine returned an invalid login action".to_owned())?;
        if !action_ids.insert(id) {
            return Err("Source engine returned duplicate login actions".to_owned());
        }
        let label = action
            .get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty() && label.len() <= 128)
            .ok_or_else(|| "Source engine returned an invalid login action".to_owned())?;
        actions.push(json!({ "id": id, "label": label }));
    }

    let mut layout = Vec::new();
    if let Some(raw_layout) = form.get("layout") {
        let raw_layout = raw_layout
            .as_array()
            .filter(|rows| rows.len() <= 192)
            .ok_or_else(|| "Source login layout exceeds the supported row limit".to_owned())?;
        let mut placed_fields = HashSet::new();
        let mut placed_actions = HashSet::new();
        for row in raw_layout {
            let cols = row
                .get("cols")
                .and_then(Value::as_u64)
                .filter(|cols| (1..=4).contains(cols))
                .ok_or_else(|| "Source login layout has invalid columns".to_owned())?;
            let rows = row
                .get("rows")
                .and_then(Value::as_u64)
                .filter(|rows| (1..=12).contains(rows))
                .ok_or_else(|| "Source login layout has invalid rows".to_owned())?;
            let mut safe_row = match row.get("kind").and_then(Value::as_str) {
                Some("field") => {
                    let name = row
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| names.contains(*name))
                        .ok_or_else(|| {
                            "Source login layout refers to an unknown field".to_owned()
                        })?;
                    if !placed_fields.insert(name.to_owned()) {
                        return Err("Source login layout repeats a field".to_owned());
                    }
                    json!({ "kind": "field", "name": name })
                }
                Some("action") => {
                    let id = row
                        .get("id")
                        .and_then(Value::as_u64)
                        .filter(|id| action_ids.contains(id))
                        .ok_or_else(|| {
                            "Source login layout refers to an unknown action".to_owned()
                        })?;
                    if !placed_actions.insert(id) {
                        return Err("Source login layout repeats an action".to_owned());
                    }
                    json!({ "kind": "action", "id": id })
                }
                Some("title") => {
                    let label = row
                        .get("label")
                        .and_then(Value::as_str)
                        .filter(|label| label.len() <= 8192)
                        .ok_or_else(|| "Source login layout has an invalid title".to_owned())?;
                    json!({ "kind": "title", "label": label })
                }
                _ => return Err("Source login layout has an unsupported row".to_owned()),
            };
            safe_row["cols"] = json!(cols);
            safe_row["rows"] = json!(rows);
            layout.push(safe_row);
        }
        if placed_fields.len() != fields.len() || placed_actions.len() != actions.len() {
            return Err("Source login layout is missing fields or actions".to_owned());
        }
    } else {
        // Copied older engines still render a usable form without layout metadata.
        for field in &fields {
            layout.push(json!({ "kind": "field", "name": field["name"], "cols": 1, "rows": 1 }));
        }
        for action in &actions {
            layout.push(json!({ "kind": "action", "id": action["id"], "cols": 2, "rows": 1 }));
        }
    }

    Ok(json!({
        "mode": "form",
        "canLogin": can_login,
        "fields": fields,
        "actions": actions,
        "layout": layout,
    }))
}

pub(crate) fn validate_source_login_credentials(
    credentials: &HashMap<String, String>,
) -> Result<(), String> {
    if credentials.len() > 64 {
        return Err("Source login input has too many fields".to_owned());
    }
    let mut total_bytes = 0usize;
    for (name, value) in credentials {
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err("Source login input contains an invalid field".to_owned());
        }
        if value.len() > 16 * 1024 {
            return Err("A source login field exceeds the supported size".to_owned());
        }
        total_bytes = total_bytes
            .saturating_add(name.len())
            .saturating_add(value.len());
    }
    if total_bytes > 64 * 1024 {
        return Err("Source login input exceeds the supported size".to_owned());
    }
    Ok(())
}

pub(crate) fn sanitize_source_login_result(result: Value) -> Result<Value, String> {
    let mut sanitized = json!({
        "hasLoginState": result["hasLoginState"].as_bool().ok_or("Invalid login-state summary")?,
    });
    for key in ["message", "error"] {
        if let Some(text) = result[key].as_str() {
            sanitized[key] = json!(text.chars().take(4096).collect::<String>());
        }
    }
    if let Some(message) = result["messages"]
        .as_array()
        .and_then(|messages| messages.iter().rev().find_map(Value::as_str))
    {
        sanitized["message"] = json!(message.chars().take(4096).collect::<String>());
    }
    if let Some(form) = result.get("form") {
        sanitized["form"] = match form["mode"].as_str() {
            Some("form") => sanitize_source_login_form(form)?,
            Some("web") => json!({"mode": "web"}),
            _ => {
                return Err(form["message"]
                    .as_str()
                    .unwrap_or("Source login is unavailable")
                    .into())
            }
        };
    }
    if let Some(open) = result["webOpen"].as_bool() {
        sanitized["webOpen"] = json!(open);
    }
    if let Some(updates) = result.get("inputUpdates") {
        let updates: std::collections::HashMap<String, String> =
            serde_json::from_value(updates.clone())
                .map_err(|_| "Invalid source login field updates".to_owned())?;
        validate_source_login_credentials(&updates)?;
        sanitized["inputUpdates"] = json!(updates);
    }
    if let Some(targets) = result.get("refreshTargets").and_then(Value::as_array) {
        sanitized["refreshTargets"] = json!(targets
            .iter()
            .filter_map(Value::as_str)
            .filter(|target| matches!(*target, "login" | "explore" | "book"))
            .take(3)
            .collect::<Vec<_>>());
    }
    if let Some(text) = result.get("copyText").and_then(Value::as_str) {
        if text.len() > 1024 * 1024 {
            return Err("Source clipboard text exceeds its limit".into());
        }
        sanitized["copyText"] = json!(text);
    }
    Ok(sanitized)
}
