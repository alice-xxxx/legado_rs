//! 书源管理和搜索 Tauri 命令。

use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SourceMutation {
    Local,
    Url { url: String },
    Save {
        id: Option<String>,
        json: String,
    },
    Delete { source_ids: Vec<String> },
    StartLoginWeb { source_id: String },
    FinishLoginWeb { source_id: String, session_id: String },
    CancelLoginWeb { source_id: String, session_id: String },
    Login {
        source_id: String,
        credentials: HashMap<String, String>,
    },
    RunLoginAction {
        source_id: String,
        action_id: usize,
        credentials: HashMap<String, String>,
    },
    ClearLoginState { source_id: String },
}

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

async fn start_source_login_web(
    app: tauri::AppHandle,
    source_id: String,
    service: &ApplicationService,
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

async fn finish_source_login_web(
    _app: tauri::AppHandle,
    source_id: String,
    session_id: String,
    service: &ApplicationService,
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

async fn cancel_source_login_web(
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

async fn login_source(
    source_id: String,
    credentials: HashMap<String, String>,
    service: &ApplicationService,
) -> Result<Value, String> {
    service.login_source(&source_id, credentials).await
}

async fn run_source_login_action(
    source_id: String,
    action_id: usize,
    credentials: HashMap<String, String>,
    service: &ApplicationService,
) -> Result<Value, String> {
    service
        .run_source_login_action(&source_id, action_id, credentials)
        .await
}

async fn clear_source_login_state(
    source_id: String,
    service: &ApplicationService,
) -> Result<Value, String> {
    service.clear_source_login_state(&source_id).await
}

#[tauri::command]
pub async fn mutate_source(
    app: tauri::AppHandle,
    mutation: SourceMutation,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let updates_sources = matches!(
        &mutation,
        SourceMutation::Local
            | SourceMutation::Url { .. }
            | SourceMutation::Save { .. }
            | SourceMutation::Delete { .. }
    );
    let mut result = match mutation {
        SourceMutation::Local => import_sources_from_picker(app.clone(), &service).await,
        SourceMutation::Url { url } => import_sources_from_url(url, &service).await,
        SourceMutation::Save {
            id,
            json,
        } => match id {
            None => import_sources(json, &service).await,
            Some(source_id) => save_source_definition(source_id, json, &service).await,
        },
        SourceMutation::Delete { source_ids } => {
            remove_sources(app.clone(), source_ids, &service).await
        }
        SourceMutation::StartLoginWeb { source_id } => {
            start_source_login_web(app.clone(), source_id, &service).await
        }
        SourceMutation::FinishLoginWeb { source_id, session_id } => {
            finish_source_login_web(app.clone(), source_id, session_id, &service).await
        }
        SourceMutation::CancelLoginWeb { source_id, session_id } => {
            cancel_source_login_web(app.clone(), source_id, session_id).await
        }
        SourceMutation::Login { source_id, credentials } => {
            login_source(source_id, credentials, &service).await
        }
        SourceMutation::RunLoginAction {
            source_id,
            action_id,
            credentials,
        } => {
            run_source_login_action(source_id, action_id, credentials, &service).await
        }
        SourceMutation::ClearLoginState { source_id } => {
            clear_source_login_state(source_id, &service).await
        }
    }?;
    if updates_sources && result.get("cancelled").and_then(Value::as_bool) != Some(true) {
        let resource = service.source_definitions_resource().await?;
        let descriptor = service.resource_descriptor(&resource);
        if let Some(object) = result.as_object_mut() {
            object.remove("sources");
            object.insert("resource".to_owned(), descriptor.clone());
        }
        let _ = app.emit("sources-updated", descriptor);
    }
    Ok(result)
}

async fn import_sources(
    source_json: String,
    service: &ApplicationService,
) -> Result<Value, String> {
    service.import_sources(&source_json).await
}

async fn import_sources_from_picker(
    app: tauri::AppHandle,
    service: &ApplicationService,
) -> Result<Value, String> {
    let Some(path) = pick_file(&app, "JSON", &["json"]).await? else {
        return Ok(json!({ "cancelled": true }));
    };
    let source_json = tokio::fs::read_to_string(&path.path)
        .await
        .map_err(|error| format!("Cannot read selected source file: {error}"))?;
    service.import_sources(&source_json).await
}

async fn import_sources_from_url(
    url: String,
    service: &ApplicationService,
) -> Result<Value, String> {
    service.import_sources_from_url(&url).await
}

async fn remove_sources(
    app: tauri::AppHandle,
    source_ids: Vec<String>,
    service: &ApplicationService,
) -> Result<Value, String> {
    let result = service.remove_sources(&source_ids).await?;
    if let Some(resource) = result.get("rssState") {
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssState", "resource": resource }),
        );
    }
    Ok(result)
}

async fn save_source_definition(
    source_id: String,
    definition_json: String,
    service: &ApplicationService,
) -> Result<Value, String> {
    service.save_source_definition(&source_id, definition_json).await
}

#[tauri::command]
pub async fn search_books(
    source_ids: Vec<String>,
    keyword: String,
    page: Option<u32>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .search_books(&source_ids, &keyword, page.unwrap_or(1))
        .await
}

#[tauri::command]
pub async fn debug_search_result(
    result_id: Option<String>,
    chapter_index: Option<usize>,
    cleanup_resource_id: Option<String>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .debug_search_result(
            result_id.as_deref(),
            chapter_index,
            cleanup_resource_id.as_deref(),
        )
        .await
}

#[tauri::command]
pub async fn start_search(
    app: tauri::AppHandle,
    source_ids: Vec<String>,
    keyword: String,
    page: Option<u32>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .start_search(&source_ids, &keyword, page.unwrap_or(1))
        .await?;
    let history = service.get_search_history().await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "searchHistory", "resource": history }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn search_book_source_candidates(
    book_id: String,
    source_ids: Vec<String>,
    keyword: Option<String>,
    page: Option<u32>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .search_book_source_candidates(&book_id, &source_ids, keyword.as_deref(), page.unwrap_or(1))
        .await
}
