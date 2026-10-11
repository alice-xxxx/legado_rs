//! 阅读工具、发现页与 RSS 内容 Tauri 命令。

use super::*;
use serde::Deserialize;

#[tauri::command]
pub async fn get_rss_state(service: State<'_, ApplicationService>) -> Result<Value, String> {
    service.get_rss_state().await
}

#[tauri::command]
pub async fn set_rss_filter(
    app: tauri::AppHandle,
    source_id: String,
    filter: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.set_rss_filter(&source_id, &filter).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssState", "resource": resource }),
    );
    Ok(resource)
}

#[tauri::command]
pub async fn set_rss_article_state(
    app: tauri::AppHandle,
    source_id: String,
    article_id: String,
    is_read: Option<bool>,
    is_favorite: Option<bool>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service
        .set_rss_article_state(&source_id, &article_id, is_read, is_favorite)
        .await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssState", "resource": resource }),
    );
    Ok(resource)
}

#[tauri::command]
pub async fn unsubscribe_rss(
    app: tauri::AppHandle,
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let mut result = service.unsubscribe_rss(&source_id).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssState", "resource": result["resource"] }),
    );
    let sources = service.source_definitions_resource().await?;
    let _ = app.emit(
        "sources-updated",
        service.resource_descriptor(&sources),
    );
    if let Some(object) = result.as_object_mut() {
        object.remove("sources");
    }
    Ok(result)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConfigurationImportSource {
    Local,
    Url { url: String },
}

#[tauri::command]
pub async fn prepare_configuration_import(
    app: tauri::AppHandle,
    kind: String,
    source: ConfigurationImportSource,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let (limit, label) = match kind.as_str() {
        "replacementRules" => (MAX_REPLACEMENT_RULE_IMPORT_BODY_BYTES, "显示替换规则 JSON"),
        "txtTocRules" => (crate::local_books::txt_toc_rules::MAX_IMPORT_BODY_BYTES, "TXT 目录规则 JSON"),
        "httpTtsConfigs" => (crate::http_tts_config::MAX_CONFIG_BYTES, "HTTP TTS JSON"),
        _ => return Err("Unknown configuration kind".into()),
    };
    let source_json = match source {
        ConfigurationImportSource::Local => {
            let Some(file) = pick_file_with_limit(&app, label, &["json"], limit as u64).await? else {
                return Ok(json!({ "cancelled": true }));
            };
            tokio::fs::read_to_string(&file.path).await.map_err(|error| error.to_string())?
        }
        ConfigurationImportSource::Url { url } => {
            download_json_url(&url, limit, Duration::from_secs(30), label).await?
        }
    };
    service.prepare_configuration_import(&kind, &source_json).await
}

#[tauri::command]
pub async fn list_discovery_categories(
    app: tauri::AppHandle,
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let result = crate::discovery::list_categories(&service, &source_id, Some(false)).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "discoveryCategories", "resource": result["resource"] }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn list_discovery_books(
    app: tauri::AppHandle,
    source_id: String,
    category_id: String,
    page: Option<u32>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let result = crate::discovery::list_books(
        &service,
        &source_id,
        &category_id,
        page.unwrap_or(1),
        Some(false),
    )
    .await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "discoveryResults", "resource": result["resource"] }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn list_discovery_favorites(
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource = crate::discovery::favorites_resource(service.resource_store()).await?;
    Ok(json!({ "resource": service.resource_descriptor(&resource) }))
}

#[tauri::command]
pub async fn set_discovery_favorite(
    app: tauri::AppHandle,
    source_id: String,
    category_id: String,
    favorite: bool,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::discovery::set_favorite(&service, &source_id, &category_id, favorite).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "discoveryFavorites", "resource": descriptor }),
    );
    Ok(descriptor)
}

#[tauri::command]
pub async fn list_rss_categories(
    app: tauri::AppHandle,
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let result = crate::rss::list_rss_categories(&service, &source_id).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssCategories", "resource": result["resource"] }),
    );
    if let Some(resource) = result.get("rssState") {
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssState", "resource": resource }),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn list_rss_articles(
    app: tauri::AppHandle,
    source_id: String,
    category_id: String,
    page: Option<u32>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let result =
        crate::rss::list_rss_articles(&service, &source_id, &category_id, page.unwrap_or(1))
            .await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssArticles", "resource": result["resource"] }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn open_rss_article(
    app: tauri::AppHandle,
    source_id: String,
    article_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let result = crate::rss::read_standard_feed_article(&service, &source_id, &article_id).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssArticle", "resource": result["resource"] }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn lookup_dictionary(
    word: String,
    language: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.lookup_dictionary(&word, &language).await
}
