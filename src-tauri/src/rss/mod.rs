//! 管理 RSS 订阅、文章解析、状态与正文资源。
//! RSS subscription workflows use existing KMP rules when present and a
//! mature feed parser for plain RSS/Atom URLs. Source-rule JSON stays private;
//! standard feed entries become processed card JSON and sanitized HTML
//! resources.

use std::time::Duration;

use feed_rs::model::{Entry, Feed};
use feed_rs::parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    application::{ApplicationService, SourceRecord},
    discovery::{self, DiscoveryCategory, PrivateCategory},
    resources::ResourceRef,
};

const MAX_FEED_BYTES: usize = 8 * 1024 * 1024;
const FEED_PAGE_SIZE: usize = 50;
const RSS_FILTER_SCAN_PAGE_LIMIT: u32 = 8;
const MAX_TRACKED_ARTICLES: usize = 50_000;
const RSS_STATE_DOCUMENT: &str = "rss-state";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssStateDocument {
    pub schema_version: u32,
    pub subscriptions: Vec<RssSubscriptionState>,
    pub articles: Vec<RssArticleState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssSubscriptionState {
    pub source_id: String,
    pub filter: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RssArticleState {
    pub source_id: String,
    pub article_id: String,
    pub is_read: bool,
    pub is_favorite: bool,
    pub updated_at_ms: u64,
    /// Exact version of the rendered article body, not an inferred chapter path.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_ref: Option<ResourceRef>,
}

impl Default for RssStateDocument {
    fn default() -> Self {
        Self {
            schema_version: crate::models::CURRENT_SCHEMA_VERSION,
            subscriptions: Vec::new(),
            articles: Vec::new(),
        }
    }
}

/// Return whether this private source record is an older RSS source schema
/// that must go through the existing KMP converter.
pub(crate) fn is_legacy_rss_source(source: &Value) -> bool {
    let Some(object) = source.as_object() else {
        return false;
    };
    !object.contains_key("bookSourceUrl")
        && object.contains_key("sourceUrl")
        && ["ruleArticles", "sortUrl", "ruleTitle", "ruleLink"]
            .iter()
            .any(|field| object.contains_key(*field))
}

/// Identify current RSS BookSource records, legacy rule-based RSS, and plain
/// feed URL records. This examines only the source envelope, never rules.
pub(crate) fn is_rss_source(source: &Value) -> bool {
    if is_legacy_rss_source(source) || is_standard_feed_source(source) {
        return true;
    }
    source.get("bookSourceType").and_then(Value::as_i64) == Some(5)
}

pub(crate) fn is_standard_feed_source(source: &Value) -> bool {
    let Some(object) = source.as_object() else {
        return false;
    };
    if object.contains_key("bookSourceUrl") || is_legacy_rss_source(source) {
        return false;
    }
    let has_url = ["feedUrl", "sourceUrl", "url"]
        .iter()
        .any(|field| object.get(*field).and_then(Value::as_str).is_some());
    let has_book_rules = [
        "searchUrl",
        "ruleSearch",
        "ruleBookInfo",
        "ruleToc",
        "ruleContent",
    ]
    .iter()
    .any(|field| object.contains_key(*field));
    has_url && !has_book_rules
}

/// Load a subscription's processed categories as a public JSON resource.
pub async fn list_rss_categories(
    service: &ApplicationService,
    source_id: &str,
) -> Result<Value, String> {
    let source = service.source_record(source_id).await?;
    if !source.enabled {
        return Err("Selected source is disabled".to_owned());
    }
    let (public_categories, private_categories) = if is_standard_feed_source(&source.source) {
        let feed_url = standard_feed_url(&source.source)?;
        let id = discovery::stable_category_id(source_id, &feed_url);
        (
            vec![DiscoveryCategory {
                category_id: Some(id.clone()),
                title: "最新文章".to_owned(),
                kind: Some("feed".to_owned()),
                style: None,
            }],
            vec![PrivateCategory {
                category_id: id,
                title: "最新文章".to_owned(),
                url: None,
                kind: "feed".to_owned(),
            }],
        )
    } else {
        discovery::prepare_categories(service, source_id, Some(true)).await?
    };

    // Browsing a valid RSS category starts the subscription with the default
    // `all` filter. Preserve a previously selected filter. This is committed
    // only after category loading succeeds and after the source snapshot is
    // revalidated under the short source-mutation guard, so a delayed network
    // response cannot recreate an unsubscribed or deleted source's category
    // map, public categories, or subscription state.
    let _source_guard = service.lock_rss_source_snapshot(&source).await?;
    let mut result =
        discovery::publish_categories(service, source_id, public_categories, private_categories)
            .await?;
    let state_ref = ensure_subscription_locked(service, &source).await?;
    result["rssState"] = service.resource_descriptor(&state_ref);
    Ok(result)
}

/// Load a page of processed subscription entries from an opaque category ID.
/// Rule-based feeds can have a page removed entirely by the user's read/favorite
/// filter, so advance over a small bounded number of empty filtered pages. The
/// returned page and hasNextPage always describe the engine page we actually
/// return, allowing the caller to continue after the scan limit.
pub async fn list_rss_articles(
    service: &ApplicationService,
    source_id: &str,
    category_id: &str,
    page: u32,
) -> Result<Value, String> {
    let mut current_page = page.max(1);
    let mut result =
        discovery::list_books(service, source_id, category_id, current_page, Some(true)).await?;
    let source = service.source_record(source_id).await?;
    if is_standard_feed_source(&source.source) {
        return Ok(result);
    }
    for scanned_pages in 0..RSS_FILTER_SCAN_PAGE_LIMIT {
        let engine_has_next = result
            .get("hasNextPage")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        result = filter_rule_rss_page(service, source_id, category_id, result).await?;
        let visible_count = result.get("bookCount").and_then(Value::as_u64).unwrap_or(0);
        if visible_count > 0
            || !engine_has_next
            || scanned_pages + 1 >= RSS_FILTER_SCAN_PAGE_LIMIT
            || current_page == u32::MAX
        {
            if current_page == u32::MAX {
                result["hasNextPage"] = json!(false);
            }
            return Ok(result);
        }
        current_page += 1;
        result = discovery::list_books(service, source_id, category_id, current_page, Some(true))
            .await?;
    }
    Ok(result)
}

async fn filter_rule_rss_page(
    service: &ApplicationService,
    source_id: &str,
    category_id: &str,
    mut result: Value,
) -> Result<Value, String> {
    let source = service.source_record(source_id).await?;
    let _source_guard = service.lock_rss_source_snapshot(&source).await?;
    let resource_id = result
        .get("resource")
        .and_then(|resource| resource.get("resourceId"))
        .and_then(Value::as_str)
        .ok_or_else(|| "RSS results resource is missing its ID".to_owned())?;
    let result_ref = ResourceRef::new(resource_id).map_err(|error| error.to_string())?;
    let mut document = service
        .resource_store()
        .read_json_ref(&result_ref)
        .await
        .map_err(|error| error.to_string())?;
    let filter = subscription_filter(service, source_id).await?;
    let cards = document
        .get_mut("results")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "RSS result document is invalid".to_owned())?;
    let mut tracked = Vec::with_capacity(cards.len());
    for card in cards.iter_mut() {
        let book_url = card
            .get("bookUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| "RSS engine result is missing its article URL".to_owned())?;
        let article_id = legacy_article_id(source_id, category_id, book_url);
        card["articleId"] = json!(article_id);
        tracked.push(article_id);
    }
    register_articles(service, source_id, &tracked).await?;
    let state = read_state(service).await?;
    for card in cards.iter_mut() {
        let article_id = card
            .get("articleId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(article) = state
            .articles
            .iter()
            .find(|article| article.source_id == source_id && article.article_id == article_id)
        {
            card["isRead"] = json!(article.is_read);
            card["isFavorite"] = json!(article.is_favorite);
        }
    }
    cards.retain(|card| {
        let article_id = card
            .get("articleId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let state = state
            .articles
            .iter()
            .find(|article| article.source_id == source_id && article.article_id == article_id);
        matches_filter(filter.as_str(), state)
    });
    let count = cards.len();
    let filtered = json!(count != tracked.len() || filter != "all");
    document["rssFilter"] = json!(filter);
    document["filtered"] = filtered.clone();
    service
        .resource_store()
        .write_json_ref(&result_ref, &document)
        .await
        .map_err(|error| error.to_string())?;
    result["bookCount"] = json!(count);
    result["filter"] = json!(filter);
    result["filtered"] = filtered;
    Ok(result)
}

/// Execute the parser-backed path for a plain feed URL. Category URLs and feed
/// credentials never cross into public resource JSON.
pub(crate) async fn list_standard_feed_articles(
    service: &ApplicationService,
    source: &SourceRecord,
    category: PrivateCategory,
    page: u32,
) -> Result<Value, String> {
    if category.kind != "feed" || category.url.is_some() {
        return Err("Invalid standard feed category".to_owned());
    }
    let feed_url = standard_feed_url(&source.source)?;
    let feed = fetch_feed(&feed_url).await?;
    let _source_guard = service.lock_rss_source_snapshot(source).await?;
    let page = page.max(1);
    let existing_state = read_state(service).await?;
    let filter = existing_state
        .subscriptions
        .iter()
        .find(|subscription| subscription.source_id == source.id)
        .map(|subscription| subscription.filter.clone())
        .unwrap_or_else(|| "all".to_owned());
    let existing_articles = existing_state
        .articles
        .into_iter()
        .filter(|article| article.source_id == source.id)
        .map(|article| (article.article_id.clone(), article))
        .collect::<std::collections::HashMap<_, _>>();
    let mut result = json!({});
    let mut candidates = Vec::with_capacity(feed.entries.len());
    let mut identities = Vec::with_capacity(feed.entries.len());
    for entry in &feed.entries {
        let article_id = stable_feed_article_id(&feed_url, &entry.id);
        if matches_filter(&filter, existing_articles.get(&article_id)) {
            identities.push(article_id);
            candidates.push(entry);
        }
    }
    let total_count = candidates.len();
    let start = (page as usize - 1).saturating_mul(FEED_PAGE_SIZE);
    let end = start.saturating_add(FEED_PAGE_SIZE).min(total_count);
    let entries = candidates
        .iter()
        .skip(start.min(total_count))
        .take(end.saturating_sub(start))
        .copied()
        .collect::<Vec<_>>();
    let article_ids = identities
        .iter()
        .skip(start.min(total_count))
        .take(end.saturating_sub(start))
        .cloned()
        .collect::<Vec<_>>();
    let has_next_page = end < total_count;
    register_articles(service, &source.id, &article_ids).await?;
    let current_articles = read_state(service)
        .await?
        .articles
        .into_iter()
        .filter(|article| article.source_id == source.id)
        .map(|article| (article.article_id.clone(), article))
        .collect::<std::collections::HashMap<_, _>>();
    let mut cards = Vec::with_capacity(entries.len());
    let mut article_resources = Vec::with_capacity(entries.len());
    for (entry, article_id) in entries.iter().zip(article_ids.iter()) {
        let content = entry_html(entry);
        let content_base_url = entry
            .links
            .first()
            .map(|link| link.href.as_str())
            .unwrap_or(feed_url.as_str());
        let chapter_ref = service
            .resource_store()
            .write_chapter_rich_text(
                &rss_resource_book_id(&source.id),
                article_id,
                &content,
                Some(content_base_url),
            )
            .await
            .map_err(|error| error.to_string())?;
        let state = current_articles
            .get(article_id)
            .cloned()
            .unwrap_or_else(|| default_article_state(&source.id, article_id));
        let mut card = entry_card(entry);
        card["resultId"] = json!(article_id);
        card["articleId"] = json!(article_id);
        card["sourceId"] = json!(source.id);
        card["sourceName"] = json!(source.name);
        card["contentSrc"] = json!(chapter_ref.as_str());
        article_resources.push((article_id.clone(), chapter_ref));
        card["isRead"] = json!(state.is_read);
        card["isFavorite"] = json!(state.is_favorite);
        cards.push(card);
    }
    record_article_resources(service, &source.id, article_resources).await?;
    let search_ref = service
        .resource_store()
        .search_ref(&format!("rss-{}", uuid::Uuid::new_v4().simple()))
        .map_err(|error| error.to_string())?;
    let card_count = cards.len();
    let public_results = json!({
        "schemaVersion": crate::models::CURRENT_SCHEMA_VERSION,
        "keyword": category.title,
        "page": page,
        "results": cards,
        "errors": [],
        "complete": true,
    });
    service
        .resource_store()
        .write_json_ref(&search_ref, &public_results)
        .await
        .map_err(|error| error.to_string())?;
    let descriptor = service.resource_descriptor(&search_ref);
    result["sourceId"] = json!(source.id);
    result["categoryId"] = json!(category.category_id);
    result["page"] = json!(page);
    result["hasNextPage"] = json!(has_next_page);
    result["bookCount"] = json!(card_count);
    result["totalCount"] = json!(total_count);
    result["filter"] = json!(filter);
    result["filtered"] = json!(filter != "all");
    result["resource"] = descriptor;
    Ok(result)
}

/// Resolve a parsed feed article to its existing typed rich-text resource.
/// `article_id` is the opaque result ID returned in the list.
pub async fn read_standard_feed_article(
    service: &ApplicationService,
    source_id: &str,
    article_id: &str,
) -> Result<Value, String> {
    discovery::validate_source_id(source_id)?;
    validate_opaque_id(article_id, "articleId")?;
    let source = service.source_record(source_id).await?;
    if !is_standard_feed_source(&source.source) {
        return Err(
            "This subscription uses engine rules; open the article through its book resource"
                .to_owned(),
        );
    }
    if !is_rss_source(&source.source) {
        return Err("Selected source is not an RSS subscription".to_owned());
    }
    let state = lookup_article_state(service, source_id, article_id)
        .await?
        .ok_or_else(|| "RSS article is unavailable; refresh the subscription".to_owned())?;
    let typed_resource = state.content_ref
        .ok_or_else(|| "RSS article resource is missing; refresh the subscription".to_owned())?;
    if !is_rss_article_resource_ref(source_id, article_id, &typed_resource) {
        return Err("RSS article has an invalid resource reference".to_owned());
    }
    let store = service.resource_store();
    if !store
        .regular_file_exists(&typed_resource)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("RSS article resource is missing; refresh the subscription".to_owned());
    }
    set_article_state(service, source_id, article_id, Some(true), None).await?;
    Ok(json!({
        "sourceId": source_id,
        "articleId": article_id,
        "resource": service.resource_descriptor(&typed_resource),
    }))
}

/// Return the browser-readable RSS state resource, initializing its JSON file
/// on first use.
pub async fn rss_state_resource(
    store: &crate::resources::ResourceStore,
) -> Result<ResourceRef, String> {
    let reference = store
        .reading_ref(RSS_STATE_DOCUMENT)
        .map_err(|error| error.to_string())?;
    store
        .update_json_ref(&reference, |current| {
            let mut document = match current {
                Value::Null => RssStateDocument::default(),
                value => decode_state(value)?,
            };
            normalize_state(&mut document)?;
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

async fn ensure_subscription_locked(
    service: &ApplicationService,
    expected_source: &SourceRecord,
) -> Result<ResourceRef, String> {
    let store = service.resource_store();
    let reference = rss_state_resource(store).await?;
    let source_id = expected_source.id.clone();
    store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_state(current)?;
            if !document
                .subscriptions
                .iter()
                .any(|subscription| subscription.source_id == source_id)
            {
                document.subscriptions.push(RssSubscriptionState {
                    source_id,
                    filter: "all".to_owned(),
                });
            }
            normalize_state(&mut document)?;
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Set the persistent server-side filter for an RSS source.
pub async fn set_subscription_filter(
    service: &ApplicationService,
    source_id: &str,
    filter: &str,
) -> Result<ResourceRef, String> {
    discovery::validate_source_id(source_id)?;
    validate_filter(filter)?;
    let source = service.source_record(source_id).await?;
    if !is_rss_source(&source.source) {
        return Err("Selected source is not an RSS subscription".to_owned());
    }
    let _source_guard = service.lock_rss_source_snapshot(&source).await?;
    let store = service.resource_store();
    let reference = rss_state_resource(store).await?;
    let source_id = source_id.to_owned();
    let filter = filter.to_owned();
    store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_state(current)?;
            if let Some(subscription) = document
                .subscriptions
                .iter_mut()
                .find(|subscription| subscription.source_id == source_id)
            {
                subscription.filter = filter;
            } else {
                document
                    .subscriptions
                    .push(RssSubscriptionState { source_id, filter });
            }
            normalize_state(&mut document)?;
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Update one article's read/favorite flags. The article must have appeared in
/// a Rust-produced RSS list first; arbitrary IDs cannot create state records.
pub async fn set_article_state(
    service: &ApplicationService,
    source_id: &str,
    article_id: &str,
    is_read: Option<bool>,
    is_favorite: Option<bool>,
) -> Result<ResourceRef, String> {
    discovery::validate_source_id(source_id)?;
    validate_opaque_id(article_id, "articleId")?;
    if is_read.is_none() && is_favorite.is_none() {
        return Err("RSS article state update is empty".to_owned());
    }
    let source = service.source_record(source_id).await?;
    if !is_rss_source(&source.source) {
        return Err("Selected source is not an RSS subscription".to_owned());
    }
    let _source_guard = service.lock_rss_source_snapshot(&source).await?;
    let store = service.resource_store();
    let reference = rss_state_resource(store).await?;
    let source_id = source_id.to_owned();
    let article_id = article_id.to_owned();
    store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_state(current)?;
            let article = document
                .articles
                .iter_mut()
                .find(|article| article.source_id == source_id && article.article_id == article_id)
                .ok_or_else(|| "RSS article is unavailable; refresh the subscription".to_owned())?;
            if let Some(is_read) = is_read {
                article.is_read = is_read;
            }
            if let Some(is_favorite) = is_favorite {
                article.is_favorite = is_favorite;
            }
            article.updated_at_ms = now_ms();
            normalize_state(&mut document)?;
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Remove a subscription's read/favorite/filter state, discovery category map,
/// and only its reserved RSS chapter resource namespace. Callers validate the
/// source while holding the source-mutation lock before invoking this cleanup.
pub async fn remove_subscription_data(
    service: &ApplicationService,
    source_id: &str,
) -> Result<ResourceRef, String> {
    discovery::validate_source_id(source_id)?;
    let store = service.resource_store();
    let reference = rss_state_resource(store).await?;
    let removed_id = source_id.to_owned();
    store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_state(current)?;
            document
                .subscriptions
                .retain(|subscription| subscription.source_id != removed_id);
            document
                .articles
                .retain(|article| article.source_id != removed_id);
            normalize_state(&mut document)?;
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    discovery::publish_categories(service, source_id, Vec::new(), Vec::new()).await?;
    store
        .remove_rss_chapter_resources(source_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

async fn subscription_filter(
    service: &ApplicationService,
    source_id: &str,
) -> Result<String, String> {
    let document = read_state(service).await?;
    Ok(document
        .subscriptions
        .iter()
        .find(|subscription| subscription.source_id == source_id)
        .map(|subscription| subscription.filter.clone())
        .unwrap_or_else(|| "all".to_owned()))
}

async fn read_state(service: &ApplicationService) -> Result<RssStateDocument, String> {
    let reference = rss_state_resource(service.resource_store()).await?;
    let value = service
        .resource_store()
        .read_json_ref(&reference)
        .await
        .map_err(|error| error.to_string())?;
    decode_state(value)
}

async fn lookup_article_state(
    service: &ApplicationService,
    source_id: &str,
    article_id: &str,
) -> Result<Option<RssArticleState>, String> {
    let document = read_state(service).await?;
    Ok(document
        .articles
        .into_iter()
        .find(|article| article.source_id == source_id && article.article_id == article_id))
}

async fn register_articles(
    service: &ApplicationService,
    source_id: &str,
    article_ids: &[String],
) -> Result<(), String> {
    discovery::validate_source_id(source_id)?;
    for article_id in article_ids {
        validate_opaque_id(article_id, "articleId")?;
    }
    let store = service.resource_store();
    let reference = rss_state_resource(store).await?;
    let source_id = source_id.to_owned();
    let article_ids = article_ids.to_vec();
    store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_state(current)?;
            for article_id in article_ids {
                if document.articles.iter().any(|article| {
                    article.source_id == source_id && article.article_id == article_id
                }) {
                    continue;
                }
                if document.articles.len() >= MAX_TRACKED_ARTICLES {
                    return Err(
                        "RSS article state limit reached; unsubscribe from old feeds".into(),
                    );
                }
                document
                    .articles
                    .push(default_article_state(&source_id, &article_id));
            }
            normalize_state(&mut document)?;
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Only Rust may publish the versioned body for a tracked article. Publish
/// the mapping after all page bodies have been prepared, so a partial feed
/// refresh cannot retarget previously saved article resources.
async fn record_article_resources(
    service: &ApplicationService,
    source_id: &str,
    entries: Vec<(String, ResourceRef)>,
) -> Result<(), String> {
    for (article_id, reference) in &entries {
        if !is_rss_article_resource_ref(source_id, article_id, reference) {
            return Err("Invalid RSS article resource reference".to_owned());
        }
    }
    let resource = rss_state_resource(service.resource_store()).await?;
    let source_id = source_id.to_owned();
    service.resource_store()
        .update_json_ref(&resource, move |current| {
            let mut document = decode_state(current)?;
            for (article_id, reference) in entries {
                let tracked = document.articles.iter_mut()
                    .find(|article| article.source_id == source_id && article.article_id == article_id)
                    .ok_or_else(|| "RSS article state was removed during refresh".to_owned())?;
                tracked.content_ref = Some(reference);
            }
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn is_rss_article_resource_ref(
    source_id: &str,
    article_id: &str,
    reference: &ResourceRef,
) -> bool {
    if !reference.is_local() {
        return false;
    }
    let prefix = format!(
        "books/{}/chapters/{:x}-",
        rss_resource_book_id(source_id),
        Sha256::digest(article_id.as_bytes())
    );
    reference.path().strip_prefix(&prefix)
        .and_then(|content| content.strip_suffix(".json"))
        .is_some_and(|hash| {
            hash.len() == 64
                && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn decode_state(value: Value) -> Result<RssStateDocument, String> {
    let mut document: RssStateDocument = serde_json::from_value(value)
        .map_err(|error| format!("Invalid RSS state JSON: {error}"))?;
    if document.schema_version != crate::models::CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported RSS state schema version {}",
            document.schema_version
        ));
    }
    normalize_state(&mut document)?;
    Ok(document)
}

/// Validate a persisted RSS document using the same limits and schema version
/// as live reads. Backup validation uses this function as well.
pub(crate) fn validate_rss_state(value: &Value) -> Result<(), String> {
    decode_state(value.clone()).map(|_| ())
}

fn normalize_state(document: &mut RssStateDocument) -> Result<(), String> {
    if document.schema_version != crate::models::CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported RSS state schema version {}",
            document.schema_version
        ));
    }
    if document.articles.len() > MAX_TRACKED_ARTICLES {
        return Err("RSS article state exceeds the supported limit".to_owned());
    }
    let mut subscriptions = std::collections::HashMap::new();
    for subscription in document.subscriptions.drain(..) {
        discovery::validate_source_id(&subscription.source_id)?;
        validate_filter(&subscription.filter)?;
        subscriptions.insert(subscription.source_id, subscription.filter);
    }
    document.subscriptions = subscriptions
        .into_iter()
        .map(|(source_id, filter)| RssSubscriptionState { source_id, filter })
        .collect();
    document
        .subscriptions
        .sort_by(|left, right| left.source_id.cmp(&right.source_id));

    let mut articles = std::collections::HashMap::new();
    for article in document.articles.drain(..) {
        discovery::validate_source_id(&article.source_id)?;
        validate_opaque_id(&article.article_id, "articleId")?;
        if article.content_ref.as_ref().is_some_and(|reference| {
            !is_rss_article_resource_ref(&article.source_id, &article.article_id, reference)
        }) {
            return Err("RSS article state contains an invalid resource ref".to_owned());
        }
        articles.insert(
            (article.source_id.clone(), article.article_id.clone()),
            article,
        );
    }
    document.articles = articles.into_values().collect();
    document.articles.sort_by(|left, right| {
        left.source_id
            .cmp(&right.source_id)
            .then_with(|| left.article_id.cmp(&right.article_id))
    });
    Ok(())
}

fn default_article_state(source_id: &str, article_id: &str) -> RssArticleState {
    RssArticleState {
        source_id: source_id.to_owned(),
        article_id: article_id.to_owned(),
        is_read: false,
        is_favorite: false,
        updated_at_ms: now_ms(),
        content_ref: None,
    }
}

fn matches_filter(filter: &str, article: Option<&RssArticleState>) -> bool {
    match filter {
        "all" => true,
        "unread" => article.is_none_or(|article| !article.is_read),
        "read" => article.is_some_and(|article| article.is_read),
        "favorites" => article.is_some_and(|article| article.is_favorite),
        _ => false,
    }
}

fn validate_filter(filter: &str) -> Result<(), String> {
    if matches!(filter, "all" | "unread" | "read" | "favorites") {
        Ok(())
    } else {
        Err("RSS filter must be all, unread, read, or favorites".to_owned())
    }
}

fn stable_feed_article_id(feed_url: &str, entry_id: &str) -> String {
    let identity = if entry_id.trim().is_empty() {
        "missing-entry-id"
    } else {
        entry_id.trim()
    };
    stable_id(&[feed_url.trim(), identity])
}

fn legacy_article_id(source_id: &str, category_id: &str, article_url: &str) -> String {
    stable_id(&[source_id, category_id, article_url])
}

fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let token = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("article-{token}")
}

fn rss_resource_book_id(source_id: &str) -> String {
    format!("rss-{source_id}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn entry_card(entry: &Entry) -> Value {
    let title = entry
        .title
        .as_ref()
        .map(|title| title.content.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(entry.id.as_str());
    let title = ammonia::clean(title);
    let author = entry
        .authors
        .first()
        .map(|person| person.name.trim())
        .filter(|author| !author.is_empty())
        .unwrap_or_default();
    let published = entry
        .published
        .or(entry.updated)
        .map(|date| date.to_rfc3339());
    json!({
        "title": truncate(&title, 512),
        "author": truncate(author, 512),
        "latestChapter": published,
    })
}

fn entry_html(entry: &Entry) -> String {
    let intro = entry
        .summary
        .as_ref()
        .map(|summary| summary.content.clone())
        .unwrap_or_default();
    entry
        .content
        .as_ref()
        .and_then(|content| content.body.clone())
        .filter(|body| !body.trim().is_empty())
        .unwrap_or(intro)
}

async fn fetch_feed(url: &str) -> Result<Feed, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "RSS feed URL is invalid".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("RSS feed URL must be a credential-free HTTP(S) URL".to_owned());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("legado_rs/0.1 (+RSS reader)")
        .build()
        .map_err(|_| "Cannot prepare RSS network request".to_owned())?;
    let mut response = client
        .get(parsed)
        .send()
        .await
        .map_err(|_| "Cannot fetch RSS feed".to_owned())?
        .error_for_status()
        .map_err(|_| "RSS server returned an error".to_owned())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Cannot read RSS feed response".to_owned())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_FEED_BYTES {
            return Err("RSS feed exceeds the 8 MiB size limit".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    parser::parse(&bytes[..]).map_err(|_| "RSS or Atom feed could not be parsed".to_owned())
}

fn standard_feed_url(source: &Value) -> Result<String, String> {
    ["feedUrl", "sourceUrl", "url"]
        .iter()
        .find_map(|key| source.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "RSS subscription URL is missing".to_owned())
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn validate_opaque_id(id: &str, field: &str) -> Result<(), String> {
    let token = id.strip_prefix("article-").unwrap_or_default();
    if token.len() != 32
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("Invalid {field}"));
    }
    Ok(())
}
