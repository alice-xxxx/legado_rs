//! 阅读工具、发现页与 RSS 内容 Tauri 命令。

use super::*;
use serde::Deserialize;

#[tauri::command]
pub async fn get_home_config(service: State<'_, ApplicationService>) -> Result<Value, String> {
    service.get_home_config().await
}

#[tauri::command]
pub async fn save_home_config(
    app: tauri::AppHandle,
    config: crate::discovery::home_config::HomeConfigDocument,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.save_home_config(config).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "homeConfig", "resource": resource }),
    );
    Ok(resource)
}

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
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TxtTocRulesMutation {
    Local,
    Url {
        url: String,
    },
    Save { id: Option<String>, json: String },
    Reorder {
        ordered_ids: Vec<String>,
    },
    Delete {
        rule_ids: Vec<String>,
    },
}

#[tauri::command]
pub async fn mutate_txt_toc_rules(
    app: tauri::AppHandle,
    mutation: TxtTocRulesMutation,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let response = match mutation {
        TxtTocRulesMutation::Local => {
            let Some(file) = pick_file_with_limit(
                &app,
                "TXT TOC rule JSON",
                &["json"],
                crate::local_books::txt_toc_rules::MAX_IMPORT_BODY_BYTES as u64,
            )
            .await?
            else {
                return Ok(json!({ "cancelled": true }));
            };
            let source_json = tokio::fs::read_to_string(&file.path)
                .await
                .map_err(|error| format!("Cannot read selected TXT TOC rule file: {error}"))?;
            service.import_txt_toc_rules_json(&source_json).await?
        }
        TxtTocRulesMutation::Url { url } => {
            service.import_txt_toc_rules_from_url(&url).await?
        }
        TxtTocRulesMutation::Save { id, json } => {
            if json.len() > crate::local_books::txt_toc_rules::MAX_IMPORT_BODY_BYTES {
                return Err("TXT TOC rule JSON exceeds the 1 MiB limit".to_owned());
            }
            let mut rule_json: Value = serde_json::from_str(&json)
                .map_err(|error| format!("Invalid TXT TOC rule JSON: {error}"))?;
            let object = rule_json
                .as_object_mut()
                .ok_or_else(|| "TXT TOC rule JSON must be an object".to_owned())?;
            object.insert("id".to_owned(), Value::String(id.clone().unwrap_or_default()));
            let rule = serde_json::from_value(rule_json)
                .map_err(|error| format!("Invalid TXT TOC rule JSON: {error}"))?;
            service
                .mutate_txt_toc_rules(crate::local_books::txt_toc_rules::TxtTocRulesEdit::Save {
                    id,
                    rule,
                })
                .await?
        }
        TxtTocRulesMutation::Reorder { ordered_ids } => {
            service
                .mutate_txt_toc_rules(
                    crate::local_books::txt_toc_rules::TxtTocRulesEdit::Reorder { ordered_ids },
                )
                .await?
        }
        TxtTocRulesMutation::Delete { rule_ids } => {
            service
                .mutate_txt_toc_rules(crate::local_books::txt_toc_rules::TxtTocRulesEdit::Delete {
                    rule_ids,
                })
                .await?
        }
    };
    if let Some(resource) = response.get("resource") {
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "txtTocRules", "resource": resource }),
        );
    }
    Ok(response)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ReplacementRulesMutation {
    Local,
    Url { url: String },
    Save { id: Option<String>, json: String },
    Delete { rule_ids: Vec<String> },
}

#[tauri::command]
pub async fn mutate_replacement_rules(
    app: tauri::AppHandle,
    mutation: ReplacementRulesMutation,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let response = match mutation {
        ReplacementRulesMutation::Local => {
            let Some(file) = pick_file_with_limit(
                &app,
                "显示替换 JSON",
                &["json"],
                MAX_REPLACEMENT_RULE_IMPORT_BODY_BYTES as u64,
            )
            .await?
            else {
                return Ok(json!({ "cancelled": true }));
            };
            let rules_json = tokio::fs::read_to_string(&file.path)
                .await
                .map_err(|error| format!("无法读取所选显示替换规则文件：{error}"))?;
            let (resource, imported_count) = {
                let _operation = service.operation_read().await;
                crate::reading_tools::import_replacement_rules_json(
                    service.resource_store(),
                    &rules_json,
                )
                .await?
            };
            json!({
                "resource": service.resource_descriptor(&resource),
                "importedCount": imported_count,
            })
        }
        ReplacementRulesMutation::Url { url } => {
            service.import_replacement_rules_from_url(&url).await?
        }
        ReplacementRulesMutation::Save { id, json: rule_json } => {
            if rule_json.len() > MAX_REPLACEMENT_RULE_IMPORT_BODY_BYTES {
                return Err("Replacement rule JSON exceeds the import size limit".to_owned());
            }
            let mut input: crate::reading_tools::DisplayReplacementRuleInput =
                serde_json::from_str(&rule_json)
                    .map_err(|error| format!("Invalid replacement rule JSON: {error}"))?;
            input.id = id;
            let _operation = service.operation_read().await;
            let (resource, saved_id) =
                crate::reading_tools::upsert_replacement_rule(service.resource_store(), input)
                    .await?;
            json!({ "resource": service.resource_descriptor(&resource), "savedId": saved_id })
        }
        ReplacementRulesMutation::Delete { rule_ids } => {
            let _operation = service.operation_read().await;
            let resource = crate::reading_tools::delete_replacement_rules(
                service.resource_store(),
                rule_ids,
            )
            .await?;
            json!({ "resource": service.resource_descriptor(&resource) })
        }
    };
    if let Some(resource) = response.get("resource") {
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "replacementRules", "resource": resource }),
        );
    }
    Ok(response)
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
