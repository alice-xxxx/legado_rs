//! Source discovery orchestration over the existing KMP WebBook engine.
//!
//! Category rule URLs are kept under private-data and are addressed from the
//! WebView only through opaque category IDs. Public resources contain display
//! metadata and projected search cards, never source definitions or rules.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    application::{ApplicationService, SourceRecord},
    models::CURRENT_SCHEMA_VERSION,
    rss::{is_legacy_rss_source, is_rss_source},
};

const CATEGORY_CACHE_DIRECTORY: &str = "discovery-categories";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCategory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCategoriesDocument {
    pub schema_version: u32,
    pub source_id: String,
    pub categories: Vec<DiscoveryCategory>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrivateCategoryMap {
    schema_version: u32,
    pub(crate) source_id: String,
    pub(crate) categories: Vec<PrivateCategory>,
}

/// Evaluate discovery categories with the source engine and publish only the
/// displayable portion. Category URLs stay in the private mapping file.
pub async fn list_categories(
    service: &ApplicationService,
    source_id: &str,
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
    let operation = if is_legacy_rss_source(&source.source) {
        "rssExploreKinds"
    } else {
        "exploreKinds"
    };
    let raw = service
        .execute_source_operation(source_id, operation, None, None, None, None, None)
        .await?;
    let raw_categories = raw
        .as_array()
        .ok_or_else(|| "Source engine returned invalid discovery categories".to_owned())?;
    if raw_categories.len() > 512 {
        return Err("Source engine returned too many discovery categories".to_owned());
    }

    let mut public_categories = Vec::with_capacity(raw_categories.len());
    let mut private_categories = Vec::with_capacity(raw_categories.len());
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
            let category_id = format!("category-{}", uuid::Uuid::new_v4().simple());
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

    publish_categories(service, source_id, public_categories, private_categories).await
}

pub(crate) async fn publish_categories(
    service: &ApplicationService,
    source_id: &str,
    categories: Vec<DiscoveryCategory>,
    private_categories: Vec<PrivateCategory>,
) -> Result<Value, String> {
    validate_source_id(source_id)?;
    let private_map = PrivateCategoryMap {
        schema_version: CURRENT_SCHEMA_VERSION,
        source_id: source_id.to_owned(),
        categories: private_categories,
    };
    let private_value = serde_json::to_value(private_map).map_err(|error| error.to_string())?;
    service
        .write_private_json(category_map_path(source_id)?, &private_value)
        .await?;

    let document = DiscoveryCategoriesDocument {
        schema_version: CURRENT_SCHEMA_VERSION,
        source_id: source_id.to_owned(),
        categories,
    };
    let resource = service
        .resource_store()
        .discovery_ref(source_id)
        .map_err(|error| error.to_string())?;
    let public_value = serde_json::to_value(&document).map_err(|error| error.to_string())?;
    service
        .resource_store()
        .write_json_ref(&resource, &public_value)
        .await
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "sourceId": source_id,
        "categoryCount": document.categories.iter().filter(|category| category.category_id.is_some()).count(),
        "resource": service.resource_descriptor(&resource),
    }))
}

pub(crate) async fn private_category(
    service: &ApplicationService,
    source_id: &str,
    category_id: &str,
) -> Result<PrivateCategory, String> {
    validate_source_id(source_id)?;
    validate_category_id(category_id)?;
    let mapping = service
        .read_private_json(category_map_path(source_id)?)
        .await?;
    if mapping.get("sourceId").and_then(Value::as_str) != Some(source_id) {
        return Err("Discovery category has expired; reload the categories".to_owned());
    }
    let categories: Vec<PrivateCategory> = serde_json::from_value(
        mapping
            .get("categories")
            .cloned()
            .ok_or_else(|| "Discovery category mapping is invalid".to_owned())?,
    )
    .map_err(|_| "Discovery category mapping is invalid".to_owned())?;
    categories
        .into_iter()
        .find(|category| category.category_id == category_id)
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
        .execute_source_operation(
            source_id,
            operation,
            Some(category_url),
            Some(page.max(1)),
            None,
            None,
            None,
        )
        .await?;
    let raw_books = raw
        .get("books")
        .and_then(Value::as_array)
        .ok_or_else(|| "Source engine returned an invalid discovery page".to_owned())?;
    if raw_books.len() > 2_000 {
        return Err("Source engine returned too many discovery results".to_owned());
    }
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
    let source = service.source_record(source_id).await?;
    if !source.enabled {
        return Err("Selected source is disabled".to_owned());
    }
    Ok(source)
}

pub(crate) fn category_map_path(source_id: &str) -> Result<std::path::PathBuf, String> {
    validate_source_id(source_id)?;
    Ok(Path::new(CATEGORY_CACHE_DIRECTORY).join(format!("{source_id}.json")))
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
fn project_category_style(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let mut projected = serde_json::Map::new();
    if let Some(cols) = object
        .get("cols")
        .and_then(Value::as_u64)
        .filter(|cols| (1..=4).contains(cols))
    {
        projected.insert("cols".to_owned(), json!(cols));
    }
    if let Some(rows) = object
        .get("rows")
        .and_then(Value::as_u64)
        .filter(|rows| (1..=12).contains(rows))
    {
        projected.insert("rows".to_owned(), json!(rows));
    }
    if let Some(width) = object
        .get("layout_flexBasisPercent")
        .and_then(Value::as_f64)
        .filter(|width| width.is_finite() && (0.0..=100.0).contains(width))
    {
        projected.insert("layoutFlexBasisPercent".to_owned(), json!(width));
    }
    (!projected.is_empty()).then_some(Value::Object(projected))
}

#[cfg(test)]
mod tests {
    use super::{project_category_style, validate_category_id, validate_source_id};
    use serde_json::json;

    #[test]
    fn opaque_ids_reject_paths_and_rule_text() {
        assert!(validate_source_id("source-abc123").is_ok());
        assert!(validate_category_id("category-def456").is_ok());
        for invalid in ["../source", "https://host/rule", "x/y", ""] {
            assert!(validate_source_id(invalid).is_err());
            assert!(validate_category_id(invalid).is_err());
        }
    }

    #[test]
    fn category_style_projects_only_bounded_layout_fields() {
        assert_eq!(
            project_category_style(&json!({
                "cols": 2,
                "rows": 3,
                "layout_flexBasisPercent": 49.5,
                "url": "https://private.example/rule",
                "script": "secret"
            })),
            Some(json!({ "cols": 2, "rows": 3, "layoutFlexBasisPercent": 49.5 }))
        );
        assert_eq!(
            project_category_style(&json!({ "cols": 99, "script": "private" })),
            None
        );
    }
}
