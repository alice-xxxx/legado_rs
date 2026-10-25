//! 登录会话负责取消、版本校验和原生窗口生命周期；规则执行直接调用 SDK。
mod view;
use super::Catalog;
use crate::{backup::state::Operations, support::validate_id};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::Ordering, Arc},
};
use tauri_plugin_source_engine::{
    LoginAction as EngineAction, LoginRequest, LoginWeb, SourceExecutor, SourceLoginWebCapture,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use view::Update;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum Action {
    Open,
    Cancel,
    Clear,
    Submit {
        #[serde(default)]
        fields: HashMap<String, String>,
        button: Option<usize>,
    },
}
struct Session {
    source_id: String,
    id: String,
    revision: u64,
    epoch: u64,
    cancel: CancellationToken,
    serial: Mutex<()>,
    passwords: Mutex<HashSet<String>>,
}
impl Session {
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    fn browser(&self) -> tauri_plugin_source_engine::SourceLoginWebSession {
        tauri_plugin_source_engine::SourceLoginWebSession {
            source_id: self.source_id.clone(),
            session_id: self.id.clone(),
        }
    }
}
pub(crate) struct Login {
    sources: Arc<Catalog>,
    executor: Arc<dyn SourceExecutor>,
    backup: Arc<Operations>,
    active: Mutex<Option<Arc<Session>>>,
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(crate) app: Option<tauri::AppHandle>,
}
impl Login {
    pub(crate) fn new(
        sources: Arc<Catalog>,
        executor: Arc<dyn SourceExecutor>,
        backup: Arc<Operations>,
    ) -> Self {
        Self {
            sources,
            executor,
            backup,
            active: Mutex::new(None),
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            app: None,
        }
    }
    pub(crate) async fn cancel_login(&self) -> Result<(), String> {
        let session = self.active.lock().await.take();
        if let Some(session) = session {
            session.cancel.cancel();
            self.close_browser(session).await?
        }
        Ok(())
    }
    async fn session(&self, source_id: &str, open: bool) -> Result<Arc<Session>, String> {
        let mut active = self.active.lock().await;
        if !open {
            return active
                .as_ref()
                .filter(|s| s.source_id == source_id && !s.cancel.is_cancelled())
                .cloned()
                .ok_or("请重新打开登录界面".into());
        }
        let mut changes = self.sources.subscribe();
        let mut restoring = self.backup.restore_barrier.subscribe();
        if *restoring.borrow_and_update() {
            return Err("正在恢复备份，请稍后登录".into());
        }
        let session = Arc::new(Session {
            source_id: source_id.into(),
            id: uuid::Uuid::new_v4().simple().to_string(),
            revision: *changes.borrow_and_update(),
            epoch: self.backup.restore_epoch.load(Ordering::SeqCst),
            cancel: CancellationToken::new(),
            serial: Mutex::new(()),
            passwords: Mutex::new(HashSet::new()),
        });
        let previous = active.replace(session.clone());
        drop(active);
        if let Some(previous) = previous {
            previous.cancel.cancel();
            if let Err(error) = self.close_browser(previous).await {
                session.cancel.cancel();
                return Err(error);
            }
        }
        // 即使界面空闲，源提交和取消也会关闭窗口；迟到的窗口由创建者再次清理。
        let watched = session.clone();
        #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
        let app = self.app.clone();
        tokio::spawn(async move {
            tokio::select! {
                _=watched.cancel.cancelled()=>{},
                _=changes.changed()=>{watched.cancel.cancel();},
                _=restoring.changed()=>{watched.cancel.cancel();},
            }
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            if let Some(app) = app {
                let _ = tokio::task::spawn_blocking(move || {
                    use tauri_plugin_source_engine::SourceEngineExt;
                    app.source_engine().cancel_web_login(watched.browser())
                })
                .await;
            }
        });
        Ok(session)
    }
    pub(crate) async fn act(
        &self,
        source_id: &str,
        action: Action,
    ) -> Result<Option<Update>, String> {
        validate_id(source_id, "sourceId")?;
        if matches!(action, Action::Cancel) {
            let mut active = self.active.lock().await;
            let session = if active.as_ref().is_some_and(|s| s.source_id == source_id) {
                active.take()
            } else {
                None
            };
            drop(active);
            if let Some(session) = session {
                session.cancel.cancel();
                self.close_browser(session).await?
            }
            return Ok(None);
        }
        let opening = matches!(action, Action::Open);
        let session = self.session(source_id, opening).await?;
        let operation = async {
            let _serial = session.serial.lock().await;
            let _operation = self.backup.operation_read().await;
            if session.revision != self.sources.version()
                || session.epoch != self.backup.restore_epoch.load(Ordering::SeqCst)
            {
                return Err("源或备份已变化，请重新登录".into());
            }
            let source = self.sources.get(source_id).await?;
            if tauri_plugin_source_engine::source::is_rss_source(&source.source) {
                return Err("RSS 源不支持书源登录".into());
            }
            let clearing = matches!(action, Action::Clear);
            let action = match action {
                Action::Open => EngineAction::Open,
                Action::Clear => {
                    self.close_browser(session.clone()).await?;
                    EngineAction::Clear
                }
                Action::Submit { fields, button } => {
                    view::credentials(&fields)?;
                    EngineAction::Submit { fields, button }
                }
                Action::Cancel => unreachable!(),
            };
            let mut request = LoginRequest {
                source: (&source.source).try_into()?,
                action,
            };
            let mut response = self
                .executor
                .login(request.clone(), session.cancel.clone())
                .await?;
            let web = response.web.take();
            let web_open = web.is_some() && opening;
            if let Some(web) = web {
                if opening {
                    self.open_browser(session.clone(), web).await?;
                } else if !clearing {
                    let captured = self.capture_browser(session.clone()).await?;
                    if captured.source_revision != session.revision
                        || captured.restore_epoch != session.epoch
                    {
                        return Err("原生登录会话已失效".into());
                    }
                    validate_web(&captured.login_url, &captured.cookie_header)?;
                    request.action = EngineAction::ImportCookies {
                        url: captured.login_url,
                        cookies: captured.cookie_header,
                    };
                    response = self.executor.login(request, session.cancel.clone()).await?;
                }
                response.form = Some(tauri_plugin_source_engine::LoginForm {
                    mode: "web".into(),
                    ..Default::default()
                });
            }
            let update = view::update(
                source_id,
                response,
                web_open,
                &mut *session.passwords.lock().await,
            )?;
            let _publication = self.sources.current(&source, session.revision).await?;
            let active = self.active.lock().await;
            if session.cancel.is_cancelled()
                || !active.as_ref().is_some_and(|s| Arc::ptr_eq(s, &session))
            {
                return Err("登录已取消".into());
            }
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            if let Some(app) = &self.app {
                use tauri::Emitter;
                app.emit("login", &update).map_err(|e| e.to_string())?;
            }
            Ok(Some(update))
        };
        let result = tokio::select! {biased;_=session.cancel.cancelled()=>Err("登录已取消或源已变化".into()),result=operation=>result};
        if result.is_err() && (opening || session.cancel.is_cancelled()) {
            session.cancel.cancel();
            self.close_browser(session).await?;
        }
        result
    }
    async fn close_browser(&self, session: Arc<Session>) -> Result<(), String> {
        #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
        if let Some(app) = self.app.clone() {
            return tokio::task::spawn_blocking(move || {
                use tauri_plugin_source_engine::SourceEngineExt;
                app.source_engine()
                    .cancel_web_login(session.browser())
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        let _ = session;
        Ok(())
    }
    async fn open_browser(&self, session: Arc<Session>, web: LoginWeb) -> Result<(), String> {
        validate_web(&web.url, &web.cookie_header)?;
        #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
        {
            let app = self.app.clone().ok_or("原生登录窗口不可用")?;
            return tokio::task::spawn_blocking(move || {
                use tauri_plugin_source_engine::{SourceEngineExt, SourceLoginWebStart};
                if session.cancel.is_cancelled() {
                    return Err("登录已取消".into());
                }
                let started = app
                    .source_engine()
                    .start_web_login(SourceLoginWebStart {
                        source_id: session.source_id.clone(),
                        session_id: session.id.clone(),
                        source_revision: session.revision,
                        restore_epoch: session.epoch,
                        login_url: web.url,
                        cookie_header: web.cookie_header,
                    })
                    .map_err(|e| e.to_string())?;
                if session.cancel.is_cancelled() {
                    app.source_engine()
                        .cancel_web_login(session.browser())
                        .map_err(|e| e.to_string())?;
                    return Err("登录已取消".into());
                }
                if !started.started {
                    return Err("原生登录窗口未打开".into());
                }
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        #[cfg(not(any(feature = "desktop", feature = "mobile-runtime")))]
        {
            let _ = session;
            Err("当前运行环境不支持原生登录窗口".into())
        }
    }
    async fn capture_browser(
        &self,
        session: Arc<Session>,
    ) -> Result<SourceLoginWebCapture, String> {
        #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
        {
            let app = self.app.clone().ok_or("原生登录窗口不可用")?;
            return tokio::task::spawn_blocking(move || {
                use tauri_plugin_source_engine::SourceEngineExt;
                app.source_engine()
                    .complete_web_login(session.browser())
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        #[cfg(not(any(feature = "desktop", feature = "mobile-runtime")))]
        {
            let _ = session;
            Err("当前运行环境不支持原生登录窗口".into())
        }
    }
}
fn validate_web(url: &str, cookies: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "登录地址无效")?;
    if url.len() > 8192
        || !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("登录地址必须是无账号密码的 HTTP 或 HTTPS 地址".into());
    }
    if cookies.len() > 256 * 1024 || cookies.contains(['\r', '\n', '\0']) {
        return Err("登录 Cookie 无效或超过大小限制".into());
    }
    Ok(())
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn login(
    source_id: String,
    action: Action,
    service: tauri::State<'_, Arc<Login>>,
) -> Result<(), String> {
    service.act(&source_id, action).await.map(|_| ())
}
