//! 处理发现页分类、书目和收藏的资源读写。
//! Source discovery orchestration over the existing KMP WebBook engine.
//!
//! Category rule URLs are kept under private-data and are addressed from the
//! WebView only through opaque category IDs. Public resources contain display
//! metadata and projected search cards, never source definitions or rules.

#[path = "home_config.rs"]
pub mod home_config;

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    application::{ApplicationService, SourceRecord},
    models::CURRENT_SCHEMA_VERSION,
    resources::ResourceRef,
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryFavorite {
    pub source_id: String,
    pub category_id: String,
    pub source_name: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
    pub position: u32,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryFavoritesDocument {
    pub schema_version: u32,
    pub favorites: Vec<DiscoveryFavorite>,
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

/// Ensure the public favorites resource exists with the current schema.
pub async fn favorites_resource(
    store: &crate::resources::ResourceStore,
) -> Result<ResourceRef, String> {
    let reference = store.discovery_favorites_ref();
    store
        .update_json_ref(&reference, |current| {
            let mut document = match current {
                Value::Null => DiscoveryFavoritesDocument {
                    schema_version: CURRENT_SCHEMA_VERSION,
                    favorites: Vec::new(),
                },
                value => serde_json::from_value::<DiscoveryFavoritesDocument>(value)
                    .map_err(|error| format!("Invalid discovery favorites JSON: {error}"))?,
            };
            document.schema_version = CURRENT_SCHEMA_VERSION;
            normalize_favorites(&mut document.favorites);
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Add/remove one processed source category from the persistent favorites.
/// Category identity is verified through private Rust storage; source rule
/// URLs and source definitions are never copied into the public JSON.
pub async fn set_favorite(
    service: &ApplicationService,
    source_id: &str,
    category_id: &str,
    favorite: bool,
) -> Result<ResourceRef, String> {
    validate_source_id(source_id)?;
    validate_category_id(category_id)?;
    let metadata = if favorite {
        let source = service.source_record(source_id).await?;
        if is_rss_source(&source.source) {
            return Err("RSS subscription categories cannot be discovery favorites".to_owned());
        }
        let private = private_category(service, source_id, category_id).await?;
        if private.kind != "engine" || private.url.is_none() {
            return Err("Only navigable discovery categories can be saved".to_owned());
        }
        let public_ref = service
            .resource_store()
            .discovery_ref(source_id)
            .map_err(|error| error.to_string())?;
        let public: DiscoveryCategoriesDocument = serde_json::from_value(
            service
                .resource_store()
                .read_json_ref(&public_ref)
                .await
                .map_err(|error| error.to_string())?,
        )
        .map_err(|_| "Discovery category display data is unavailable".to_owned())?;
        let category = public
            .categories
            .into_iter()
            .find(|category| category.category_id.as_deref() == Some(category_id))
            .ok_or_else(|| "Discovery category has expired; reload the categories".to_owned())?;
        Some((source.name, category))
    } else {
        None
    };
    let store = service.resource_store();
    let reference = favorites_resource(store).await?;
    let source_id = source_id.to_owned();
    let category_id = category_id.to_owned();
    store
        .update_json_ref(&reference, move |current| {
            let mut document: DiscoveryFavoritesDocument = serde_json::from_value(current)
                .map_err(|error| format!("Invalid discovery favorites JSON: {error}"))?;
            let existing = document
                .favorites
                .iter()
                .position(|item| item.source_id == source_id && item.category_id == category_id);
            if let Some((source_name, category)) = metadata {
                if let Some(index) = existing {
                    let entry = &mut document.favorites[index];
                    entry.source_name = source_name;
                    entry.title = category.title;
                    entry.kind = category.kind;
                    entry.style = category.style;
                } else {
                    let position = document.favorites.len() as u32;
                    document.favorites.push(DiscoveryFavorite {
                        source_id: source_id.to_owned(),
                        category_id: category_id.to_owned(),
                        source_name,
                        title: category.title,
                        kind: category.kind,
                        style: category.style,
                        position,
                        created_at_ms: now_ms(),
                    });
                }
            } else if let Some(index) = existing {
                document.favorites.remove(index);
            }
            document.schema_version = CURRENT_SCHEMA_VERSION;
            normalize_favorites(&mut document.favorites);
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

fn normalize_favorites(favorites: &mut Vec<DiscoveryFavorite>) {
    let mut seen = std::collections::HashSet::new();
    favorites.retain(|item| seen.insert((item.source_id.clone(), item.category_id.clone())));
    favorites.sort_by_key(|item| item.position);
    for (position, item) in favorites.iter_mut().enumerate() {
        item.position = position as u32;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

/// Evaluate discovery categories with the source engine and publish only the
/// displayable portion. Category URLs stay in the private mapping file.
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
    // Engine execution may take arbitrarily long. Only an RSS caller requests
    // this guard: revalidate the source after the network/engine work, then
    // keep the mutation lock across the public result write so an unsubscribe
    // cannot race a delayed RSS response into publishing a new result resource.
    let _source_guard = if require_rss == Some(true) {
        Some(service.lock_rss_source_snapshot(&source).await?)
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
