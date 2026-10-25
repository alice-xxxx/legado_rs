//! 管理 RSS 订阅、文章解析、状态与正文资源。
//! RSS subscription workflows use existing KMP rules when present and a
//! mature feed parser for plain RSS/Atom URLs. Source-rule JSON stays private;
//! standard feed entries become processed card JSON and sanitized HTML
//! resources.

use std::time::Duration;

use feed_rs::model::{Entry, Feed};
use feed_rs::parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    discovery::{self, DiscoveryCategory, PrivateCategory},
    resources::ResourceRef,
    search::Search,
};

const MAX_FEED_BYTES: usize = 8 * 1024 * 1024;
const FEED_PAGE_SIZE: usize = 50;
const RSS_FILTER_SCAN_PAGE_LIMIT: u32 = 8;

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
            schema_version: 1,
            subscriptions: Vec::new(),
            articles: Vec::new(),
        }
    }
}

/// Return whether this private source record is an older RSS source schema
/// that must go through the existing KMP converter.
pub(crate) use tauri_plugin_source_engine::source::{is_rss_source, is_standard_feed_source};

/// Load a subscription's processed categories as a public JSON resource.
pub(crate) async fn load_rss_categories(
    service: &Search,
    database: &crate::database::Database,
    source_id: &str,
) -> Result<Value, String> {
    let source = service.sources.get(source_id).await?;
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
    let _source_guard = service.sources.rss_snapshot(&source).await?;
    let result =
        discovery::publish_categories(database, source_id, public_categories, private_categories)
            .await?;
    ensure_subscription_locked(database, &source).await?;
    Ok(result)
}

/// Load a page of processed subscription entries from an opaque category ID.
/// Rule-based feeds can have a page removed entirely by the user's read/favorite
/// filter, so advance over a small bounded number of empty filtered pages. The
/// returned page and hasNextPage always describe the engine page we actually
/// return, allowing the caller to continue after the scan limit.
pub(crate) async fn load_rss_articles(
    service: &Search,
    database: &crate::database::Database,
    resources: &crate::resources::Resources,
    source_id: &str,
    category_id: &str,
    page: u32,
) -> Result<Value, String> {
    let mut current_page = page.max(1);
    let mut result = discovery::list_books(
        service,
        database,
        resources,
        source_id,
        category_id,
        current_page,
        Some(true),
    )
    .await?;
    let source = service.sources.get(source_id).await?;
    if is_standard_feed_source(&source.source) {
        return Ok(result);
    }
    for scanned_pages in 0..RSS_FILTER_SCAN_PAGE_LIMIT {
        let engine_has_next = result
            .get("hasNextPage")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        result = filter_rule_rss_page(service, database, source_id, category_id, result).await?;
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
        result = discovery::list_books(
            service,
            database,
            resources,
            source_id,
            category_id,
            current_page,
            Some(true),
        )
        .await?;
    }
    Ok(result)
}

async fn filter_rule_rss_page(
    service: &Search,
    database: &crate::database::Database,
    source_id: &str,
    category_id: &str,
    mut result: Value,
) -> Result<Value, String> {
    let source = service.sources.get(source_id).await?;
    let _source_guard = service.sources.rss_snapshot(&source).await?;
    let filter = subscription_filter(database, source_id).await?;
    let cards = result["search"]
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
    register_articles(database, source_id, &tracked).await?;
    let state = read_state(database).await?;
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
    result["bookCount"] = json!(count);
    result["filter"] = json!(filter);
    result["filtered"] = filtered;
    Ok(result)
}

/// Execute the parser-backed path for a plain feed URL. Category URLs and feed
/// credentials never cross into public resource JSON.
pub(crate) async fn list_standard_feed_articles(
    service: &Search,
    database: &crate::database::Database,
    resources: &crate::resources::Resources,
    source: &SourceRecord,
    category: PrivateCategory,
    page: u32,
) -> Result<Value, String> {
    if category.kind != "feed" || category.url.is_some() {
        return Err("Invalid standard feed category".to_owned());
    }
    let feed_url = standard_feed_url(&source.source)?;
    let feed = fetch_feed(&feed_url).await?;
    let _source_guard = service.sources.rss_snapshot(source).await?;
    let page = page.max(1);
    let existing_state = read_state(database).await?;
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
    register_articles(database, &source.id, &article_ids).await?;
    let current_articles = read_state(database)
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
        let chapter_ref = crate::book_files::html(
            resources,
            "html",
            &rss_resource_book_id(&source.id),
            article_id,
            &content,
            Some(content_base_url),
            false,
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
    record_article_resources(database, &source.id, article_resources).await?;
    let card_count = cards.len();
    let public_results = json!({
        "schemaVersion": 1,
        "keyword": category.title,
        "page": page,
        "results": cards,
        "errors": [],
        "complete": true,
    });
    result["sourceId"] = json!(source.id);
    result["categoryId"] = json!(category.category_id);
    result["page"] = json!(page);
    result["hasNextPage"] = json!(has_next_page);
    result["bookCount"] = json!(card_count);
    result["totalCount"] = json!(total_count);
    result["filter"] = json!(filter);
    result["filtered"] = json!(filter != "all");
    result["search"] = public_results;
    Ok(result)
}

/// Resolve a parsed feed article to its existing typed rich-text resource.
/// `article_id` is the opaque result ID returned in the list.
pub(crate) async fn read_standard_feed_article(
    service: &Search,
    database: &crate::database::Database,
    resources: &crate::resources::Resources,
    source_id: &str,
    article_id: &str,
) -> Result<Value, String> {
    discovery::validate_source_id(source_id)?;
    validate_opaque_id(article_id, "articleId")?;
    let source = service.sources.get(source_id).await?;
    if !is_standard_feed_source(&source.source) {
        return Err(
            "This subscription uses engine rules; open the article through its book resource"
                .to_owned(),
        );
    }
    if !is_rss_source(&source.source) {
        return Err("Selected source is not an RSS subscription".to_owned());
    }
    let state = lookup_article_state(database, source_id, article_id)
        .await?
        .ok_or_else(|| "RSS article is unavailable; refresh the subscription".to_owned())?;
    let typed_resource = state
        .content_ref
        .ok_or_else(|| "RSS article resource is missing; refresh the subscription".to_owned())?;
    if !is_rss_article_resource_ref(source_id, article_id, &typed_resource) {
        return Err("RSS article has an invalid resource reference".to_owned());
    }
    if !resources
        .cache
        .contains(&typed_resource)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("RSS article resource is missing; refresh the subscription".to_owned());
    }
    set_article_state(service, database, source_id, article_id, Some(true), None).await?;
    Ok(json!({
        "sourceId": source_id,
        "articleId": article_id,
        "resource": resources.descriptor(&typed_resource),
    }))
}

async fn ensure_subscription_locked(
    database: &crate::database::Database,
    expected_source: &SourceRecord,
) -> Result<(), String> {
    let source_id = expected_source.id.clone();
    database
        .ensure_rss_subscription(source_id)
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 更新已登记文章的阅读和收藏标记。
pub(crate) async fn set_article_state(
    service: &Search,
    database: &crate::database::Database,
    source_id: &str,
    article_id: &str,
    is_read: Option<bool>,
    is_favorite: Option<bool>,
) -> Result<(), String> {
    discovery::validate_source_id(source_id)?;
    validate_opaque_id(article_id, "articleId")?;
    if is_read.is_none() && is_favorite.is_none() {
        return Err("RSS article state update is empty".to_owned());
    }
    let source = service.sources.get(source_id).await?;
    if !is_rss_source(&source.source) {
        return Err("Selected source is not an RSS subscription".to_owned());
    }
    let _source_guard = service.sources.rss_snapshot(&source).await?;
    let source_id = source_id.to_owned();
    let article_id = article_id.to_owned();
    database
        .save_rss_articles(
            source_id,
            vec![crate::database::RssArticlePatch {
                article_id,
                is_read,
                is_favorite,
                content_ref: None,
            }],
        )
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn subscription_filter(
    database: &crate::database::Database,
    source_id: &str,
) -> Result<String, String> {
    let document = read_state(database).await?;
    Ok(document
        .subscriptions
        .iter()
        .find(|subscription| subscription.source_id == source_id)
        .map(|subscription| subscription.filter.clone())
        .unwrap_or_else(|| "all".to_owned()))
}

async fn read_state(database: &crate::database::Database) -> Result<RssStateDocument, String> {
    database.rss_state().map_err(|e| e.to_string())
}

async fn lookup_article_state(
    database: &crate::database::Database,
    source_id: &str,
    article_id: &str,
) -> Result<Option<RssArticleState>, String> {
    let document = read_state(database).await?;
    Ok(document
        .articles
        .into_iter()
        .find(|article| article.source_id == source_id && article.article_id == article_id))
}

async fn register_articles(
    database: &crate::database::Database,
    source_id: &str,
    article_ids: &[String],
) -> Result<(), String> {
    discovery::validate_source_id(source_id)?;
    for article_id in article_ids {
        validate_opaque_id(article_id, "articleId")?;
    }
    let source_id = source_id.to_owned();
    let articles = article_ids
        .iter()
        .map(|id| crate::database::RssArticlePatch {
            article_id: id.clone(),
            ..Default::default()
        })
        .collect();
    database
        .save_rss_articles(source_id, articles)
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Only Rust may publish the versioned body for a tracked article. Publish
/// the mapping after all page bodies have been prepared, so a partial feed
/// refresh cannot retarget previously saved article resources.
async fn record_article_resources(
    database: &crate::database::Database,
    source_id: &str,
    entries: Vec<(String, ResourceRef)>,
) -> Result<(), String> {
    for (article_id, reference) in &entries {
        if !is_rss_article_resource_ref(source_id, article_id, reference) {
            return Err("Invalid RSS article resource reference".to_owned());
        }
    }
    let source_id = source_id.to_owned();
    let articles = entries
        .into_iter()
        .map(|(article_id, reference)| crate::database::RssArticlePatch {
            article_id,
            content_ref: Some(reference.as_str().to_owned()),
            ..Default::default()
        })
        .collect();
    database
        .save_rss_articles(source_id, articles)
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn is_rss_article_resource_ref(source_id: &str, article_id: &str, reference: &ResourceRef) -> bool {
    if !reference.is_local() {
        return false;
    }
    let prefix = format!(
        "cache/html/{}/{:x}-",
        rss_resource_book_id(source_id),
        Sha256::digest(article_id.as_bytes())
    );
    reference
        .path()
        .strip_prefix(&prefix)
        .and_then(|content| content.strip_suffix(".html"))
        .is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
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
    let client = crate::http_client::builder()
        .timeout(Duration::from_secs(20))
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
    ["bookSourceUrl", "feedUrl", "sourceUrl", "url"]
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

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::{Emitter, State};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn list_rss_categories(
    database: tauri::State<'_, crate::database::Database>,
    app: tauri::AppHandle,
    source_id: String,
    service: State<'_, std::sync::Arc<Search>>,
) -> Result<Value, String> {
    let _operation = service.backup.operation_read().await;
    let result = crate::rss::load_rss_categories(&service, &database, &source_id).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssCategories", "sourceId": source_id }),
    );
    let _ = app.emit(
        "resource-updated",
        json!({"kind":"rssState","sourceId":source_id}),
    );
    Ok(result)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn list_rss_articles(
    database: tauri::State<'_, crate::database::Database>,
    resources: tauri::State<'_, std::sync::Arc<crate::resources::Resources>>,
    app: tauri::AppHandle,
    source_id: String,
    category_id: String,
    page: Option<u32>,
    service: State<'_, std::sync::Arc<Search>>,
) -> Result<Value, String> {
    let _operation = service.backup.operation_read().await;
    let result = crate::rss::load_rss_articles(
        &service,
        &database,
        &resources,
        &source_id,
        &category_id,
        page.unwrap_or(1),
    )
    .await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssArticles", "sourceId": source_id, "categoryId": category_id }),
    );
    Ok(result)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn open_rss_article(
    database: tauri::State<'_, crate::database::Database>,
    resources: tauri::State<'_, std::sync::Arc<crate::resources::Resources>>,
    app: tauri::AppHandle,
    source_id: String,
    article_id: String,
    service: State<'_, std::sync::Arc<Search>>,
) -> Result<Value, String> {
    let _operation = service.backup.operation_read().await;
    let result = crate::rss::read_standard_feed_article(
        &service,
        &database,
        &resources,
        &source_id,
        &article_id,
    )
    .await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "rssArticle", "resource": result["resource"] }),
    );
    Ok(result)
}

use crate::sources::SourceRecord;
