//! 书源登录与私有浏览器操作；书源定义写入统一走 fe_db。

use super::*;
#[tauri::command]
pub async fn get_source_login_state(
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.get_source_login_state(&source_id).await
}

#[tauri::command]
pub async fn get_source_login_form(
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.get_source_login_form(&source_id).await
}

#[tauri::command]
pub async fn start_source_login_web(
    app: tauri::AppHandle,
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let info = service.source_login_web_info(&source_id).await?;
    let login_url = info
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "Source engine returned no web login URL".to_owned())?;
    let cookie_header = info
        .get("cookieHeader")
        .and_then(Value::as_str)
        .ok_or_else(|| "Source engine returned invalid login cookies".to_owned())?;
    let source_revision = info
        .get("sourceRevision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Source engine returned an invalid source revision".to_owned())?;
    let restore_epoch = info
        .get("restoreEpoch")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Source engine returned an invalid restore epoch".to_owned())?;
    let session_id = uuid::Uuid::new_v4().simple().to_string();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        use tauri_plugin_source_engine::{SourceEngineExt, SourceLoginWebStart};

        let started = app
            .source_engine()
            .start_web_login(SourceLoginWebStart {
                source_id,
                session_id: session_id.clone(),
                source_revision,
                restore_epoch,
                login_url: login_url.to_owned(),
                cookie_header: cookie_header.to_owned(),
            })
            .map_err(|error| error.to_string())?;
        if !started.started {
            return Err("Native source login browser did not start".to_owned());
        }
        Ok(json!({ "sessionId": session_id }))
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        crate::source_engine::start_source_login_web(
            &app,
            &source_id,
            &session_id,
            source_revision,
            restore_epoch,
            login_url,
            cookie_header,
        )?;
        Ok(json!({ "sessionId": session_id }))
    }
}

#[tauri::command]
pub async fn finish_source_login_web(
    _app: tauri::AppHandle,
    source_id: String,
    session_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let captured = {
        use tauri_plugin_source_engine::{SourceEngineExt, SourceLoginWebSession};

        let captured = _app
            .source_engine()
            .complete_web_login(SourceLoginWebSession {
                source_id: source_id.clone(),
                session_id,
            })
            .map_err(|error| error.to_string())?;
        json!({
            "loginUrl": captured.login_url,
            "sourceRevision": captured.source_revision,
            "restoreEpoch": captured.restore_epoch,
            "cookieHeader": captured.cookie_header,
        })
    };
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let captured = crate::source_engine::complete_source_login_web(&source_id, &session_id)?;
    let login_url = captured
        .get("loginUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| "Private browser returned no login URL".to_owned())?;
    let cookie_header = captured
        .get("cookieHeader")
        .and_then(Value::as_str)
        .ok_or_else(|| "Private browser returned invalid cookies".to_owned())?;
    let source_revision = captured
        .get("sourceRevision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Private browser returned an invalid source revision".to_owned())?;
    let restore_epoch = captured
        .get("restoreEpoch")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Private browser returned an invalid restore epoch".to_owned())?;
    service
        .import_source_login_cookies(
            &source_id,
            source_revision,
            restore_epoch,
            login_url,
            cookie_header,
        )
        .await
}

#[tauri::command]
pub async fn cancel_source_login_web(
    _app: tauri::AppHandle,
    source_id: String,
    session_id: String,
) -> Result<Value, String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        use tauri_plugin_source_engine::{SourceEngineExt, SourceLoginWebSession};

        let result = _app
            .source_engine()
            .cancel_web_login(SourceLoginWebSession {
                source_id,
                session_id,
            })
            .map_err(|error| error.to_string())?;
        return Ok(json!({ "cancelled": result.cancelled }));
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let cancelled = crate::source_engine::cancel_source_login_web(&source_id, &session_id)?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    Ok(json!({ "cancelled": cancelled }))
}

#[tauri::command]
pub async fn login_source(
    source_id: String,
    credentials: HashMap<String, String>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.login_source(&source_id, credentials).await
}

#[tauri::command]
pub async fn run_source_login_action(
    source_id: String,
    action_id: usize,
    credentials: HashMap<String, String>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .run_source_login_action(&source_id, action_id, credentials)
        .await
}

#[tauri::command]
pub async fn clear_source_login_state(
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.clear_source_login_state(&source_id).await
}
