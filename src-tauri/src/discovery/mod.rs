//! 处理发现页分类、书目和收藏的资源读写。
//! Source discovery orchestration over the existing KMP WebBook engine.
//!
//! 分类规则地址保存在数据库中；Vue 使用分类 ID 请求，只读取展示信息。

#[path = "home_config.rs"]
pub mod home_config;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    application::{project_search_result, source_book_id, ApplicationService},
    rss::{is_legacy_rss_source, is_rss_source},
    source_engine::SourceEngineRequest,
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCategory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) style: Option<crate::database::CategoryStyle>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrivateCategory {
    pub(crate) category_id: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    pub(crate) kind: String,
}

/// 通过引擎取得分类，展示字段和请求目标一起写入关系表。
pub async fn list_categories(
    service: &ApplicationService,
    source_id: &str,
    require_rss: Option<bool>,
) -> Result<Value, String> {
    let (public_categories, private_categories) =
        prepare_categories(service, source_id, require_rss).await?;
    publish_categories(service, source_id, public_categories, private_categories).await
}

/// Evaluate category rules and produce the display/private projections without
/// writing either resource. RSS uses this split so it can revalidate the
/// source and publish the result while holding the source mutation guard.
pub(crate) async fn prepare_categories(
    service: &ApplicationService,
    source_id: &str,
    require_rss: Option<bool>,
) -> Result<(Vec<DiscoveryCategory>, Vec<PrivateCategory>), String> {
    let source = enabled_source(service, source_id).await?;
    if let Some(require_rss) = require_rss {
        if is_rss_source(&source.source) != require_rss {
            return Err(if require_rss {
                "Selected source is not an RSS subscription".to_owned()
            } else {
                "RSS sources must use the subscription view".to_owned()
            });
        }
    }
    let operation = if is_legacy_rss_source(&source.source) {
        "rssExploreKinds"
    } else {
        "exploreKinds"
    };
    let raw = service
        .executor
        .execute(SourceEngineRequest::new(operation, &source.source))
        .await?;
    let raw_categories = raw
        .as_array()
        .ok_or_else(|| "Source engine returned invalid discovery categories".to_owned())?;
    if raw_categories.len() > 512 {
        return Err("Source engine returned too many discovery categories".to_owned());
    }

    let mut public_categories = Vec::with_capacity(raw_categories.len());
    let mut private_categories = Vec::with_capacity(raw_categories.len());
    let mut seen_category_ids = std::collections::HashSet::new();
    for value in raw_categories {
        // The engine reports category evaluation failures as an ExploreKind
        // whose URL contains diagnostic text. Do not project that value.
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(512)
            .collect::<String>();
        if title.starts_with("ERROR:") {
            return Err("Source engine could not load discovery categories".to_owned());
        }
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| matches!(*kind, "text" | "button" | "title" | "select" | "toggle"))
            .map(str::to_owned);
        let style = value.get("style").and_then(project_category_style);
        let category_url = value
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.trim().is_empty());
        if let Some(url) = category_url {
            if url.len() > 16_384 {
                return Err("Discovery category URL is too long".to_owned());
            }
            let category_id = stable_category_id(source_id, url);
            // Duplicate ExploreKind URLs lead to the same engine request
            // target; keep one public and private category for that target.
            if !seen_category_ids.insert(category_id.clone()) {
                continue;
            }
            private_categories.push(PrivateCategory {
                category_id: category_id.clone(),
                title: title.clone(),
                url: Some(url.to_owned()),
                kind: "engine".to_owned(),
            });
            public_categories.push(DiscoveryCategory {
                category_id: Some(category_id),
                title,
                kind,
                style,
            });
        } else {
            public_categories.push(DiscoveryCategory {
                category_id: None,
                title,
                kind,
                style,
            });
        }
    }

    Ok((public_categories, private_categories))
}

/// Stable opaque identity for a source category. The URL and source identity
/// determine the engine request target; mutable display titles do not
/// invalidate a favorite after a category rename.
pub(crate) fn stable_category_id(source_id: &str, category_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_id.as_bytes());
    hasher.update([0]);
    hasher.update(category_url.as_bytes());
    let digest = hasher.finalize();
    let token = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("category-{token}")
}

pub(crate) async fn publish_categories(
    service: &ApplicationService,
    source_id: &str,
    categories: Vec<DiscoveryCategory>,
    private_categories: Vec<PrivateCategory>,
) -> Result<Value, String> {
    validate_source_id(source_id)?;
    let category_count = categories.iter().filter(|c| c.category_id.is_some()).count();
    service.resource_store().database().save_discovery_categories(
        source_id.to_owned(), categories, private_categories,
    ).map_err(|e|e.to_string())?.await.map_err(|e|e.to_string())?;
    Ok(json!({
        "sourceId": source_id,
        "categoryCount": category_count,
    }))
}

pub(crate) async fn private_category(
    service: &ApplicationService,
    source_id: &str,
    category_id: &str,
) -> Result<PrivateCategory, String> {
    validate_source_id(source_id)?;
    validate_category_id(category_id)?;
    let database = service.resource_store().database().clone();
    let source = source_id.to_owned();
    let category = category_id.to_owned();
    tokio::task::spawn_blocking(move || database.discovery_category(&source, &category))
        .await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?
        .ok_or_else(|| "Unknown or expired discovery category ID".to_owned())
}

/// Fetch one page for a previously returned opaque category ID. The source
/// engine receives the private URL and returns parsed SearchBook values; only
/// Rust-projected cards are placed in the browser resource.
pub async fn list_books(
    service: &ApplicationService,
    source_id: &str,
    category_id: &str,
    page: u32,
    require_rss: Option<bool>,
) -> Result<Value, String> {
    let source = enabled_source(service, source_id).await?;
    if let Some(require_rss) = require_rss {
        if is_rss_source(&source.source) != require_rss {
            return Err(if require_rss {
                "Selected source is not an RSS subscription".to_owned()
            } else {
                "RSS sources must use the subscription view".to_owned()
            });
        }
    }
    let category = private_category(service, source_id, category_id).await?;
    if category.kind == "feed" {
        return crate::rss::list_standard_feed_articles(service, &source, category, page.max(1))
            .await;
    }
    let category_url = category
        .url
        .clone()
        .ok_or_else(|| "Discovery category has no engine URL".to_owned())?;
    let operation = if is_legacy_rss_source(&source.source) {
        "rssExplore"
    } else {
        "explore"
    };
    let raw = service
        .executor
        .execute(SourceEngineRequest {
            keyword: Some(category_url),
            page: Some(page.max(1) as i32),
            ..SourceEngineRequest::new(operation, &source.source)
        })
        .await?;
    let raw_books = raw
        .get("books")
        .and_then(Value::as_array)
        .ok_or_else(|| "Source engine returned an invalid discovery page".to_owned())?;
    if raw_books.len() > 2_000 {
        return Err("Source engine returned too many discovery results".to_owned());
    }
    // Engine execution may take arbitrarily long. Only an RSS caller requests
    // this guard: revalidate the source after the network/engine work, then
    // keep the mutation lock across the public result write so an unsubscribe
    // cannot race a delayed RSS response into publishing a new result resource.
    let _source_guard = if require_rss == Some(true) {
        Some(service.sources.rss_snapshot(&source).await?)
    } else {
        None
    };
    let mut result = service
        .store_processed_results(
            &category.title,
            page.max(1),
            raw_books
                .iter()
                .cloned()
                .map(|book| (source.clone(), book))
                .collect(),
            Vec::new(),
        )
        .await?;
    result["sourceId"] = json!(source_id);
    result["categoryId"] = json!(category_id);
    result["page"] = json!(page.max(1));
    result["hasNextPage"] = raw
        .get("hasNextPage")
        .cloned()
        .unwrap_or(Value::Bool(false));
    Ok(result)
}

async fn enabled_source(
    service: &ApplicationService,
    source_id: &str,
) -> Result<SourceRecord, String> {
    validate_source_id(source_id)?;
    let source = service.sources.get(source_id).await?;
    if !source.enabled {
        return Err("Selected source is disabled".to_owned());
    }
    Ok(source)
}

pub(crate) fn validate_source_id(source_id: &str) -> Result<(), String> {
    if source_id.is_empty()
        || source_id.len() > 128
        || !source_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Invalid source ID".to_owned());
    }
    Ok(())
}

pub(crate) fn validate_category_id(category_id: &str) -> Result<(), String> {
    if category_id.is_empty()
        || category_id.len() > 128
        || !category_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Invalid category ID".to_owned());
    }
    Ok(())
}

/// ExploreKind.style is presentation metadata, but only its known layout
/// scalars cross into the public resource. Do not copy future/raw fields from
/// an engine object into a browser resource by default.
fn project_category_style(value: &Value) -> Option<crate::database::CategoryStyle> {
    let cols=value.get("cols").and_then(Value::as_i64).filter(|value|(1..=4).contains(value));
    let rows=value.get("rows").and_then(Value::as_i64).filter(|value|(1..=12).contains(value));
    let width=value.get("layout_flexBasisPercent").and_then(Value::as_f64).filter(|value|value.is_finite() && (0.0..=100.0).contains(value));
    (cols.is_some() || rows.is_some() || width.is_some()).then_some(crate::database::CategoryStyle {cols,rows,layout_flex_basis_percent:width})
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::{Emitter, State};

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
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
        json!({ "kind": "discoveryCategories", "sourceId": source_id }),
    );
    Ok(result)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
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
        json!({ "kind": "discoveryResults", "sourceId": source_id, "categoryId": category_id }),
    );
    Ok(result)
}

impl ApplicationService {
    // 发现与 RSS 的结果仅在此投影和缓存，不在此发起搜索。
    pub(crate) async fn store_processed_results(
        &self,
        keyword: &str,
        page: u32,
        source_results: Vec<(SourceRecord, Value)>,
        errors: Vec<Value>,
    ) -> Result<Value, String> {
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let mut public_results = Vec::with_capacity(source_results.len());
        let mut private_results = Vec::with_capacity(source_results.len());
        for (source, book) in source_results {
            let result_id = source_book_id(&source, &book);
            let leaf = project_search_result(&result_id, &source, &book);
            let cache = json!({
                "searchId": search_id,
                "sourceId": source.id,
                "book": book,
                "result": leaf
            });
            public_results.push(serde_json::to_value(leaf).map_err(|error| error.to_string())?);
            private_results.push((result_id, cache));
        }
        self.searches
            .lock()
            .await
            .search_result_documents
            .extend(private_results);
        let count = public_results.len();
        let document = json!({
            "searchId": search_id,
            "keyword": keyword,
            "page": page.max(1),
            "results": public_results,
            "errors": errors,
            "complete": true,
        });
        Ok(json!({ "search": document, "bookCount": count, "errors": errors }))
    }
}

use crate::source_catalog::SourceRecord;
