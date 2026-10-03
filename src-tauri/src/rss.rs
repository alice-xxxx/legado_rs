//! RSS subscription workflows use existing KMP rules when present and a
//! mature feed parser for plain RSS/Atom URLs. Source-rule JSON stays private;
//! standard feed entries become processed card JSON and sanitized HTML
//! resources.

use std::{path::Path, time::Duration};

use feed_rs::model::{Entry, Feed};
use feed_rs::parser;
use serde_json::{json, Value};

use crate::{
    application::{ApplicationService, SourceRecord},
    discovery::{self, DiscoveryCategory, PrivateCategory},
    models::ReaderDefaults,
    resources::ResourceRef,
};

const MAX_FEED_BYTES: usize = 8 * 1024 * 1024;
const FEED_PAGE_SIZE: usize = 50;

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
    if is_standard_feed_source(&source.source) {
        let id = format!("category-{}", uuid::Uuid::new_v4().simple());
        return discovery::publish_categories(
            service,
            source_id,
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
        .await;
    }
    discovery::list_categories(service, source_id, Some(true)).await
}

/// Load one page of processed subscription entries from an opaque category ID.
/// Article cards are projected with the same public SearchBook model as
/// discovery cards; the rule URL stays in private app data.
pub async fn list_rss_articles(
    service: &ApplicationService,
    source_id: &str,
    category_id: &str,
    page: u32,
) -> Result<Value, String> {
    discovery::list_books(service, source_id, category_id, page, Some(true)).await
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
    let page = page.max(1);
    let start = (page as usize - 1).saturating_mul(FEED_PAGE_SIZE);
    let end = start.saturating_add(FEED_PAGE_SIZE).min(feed.entries.len());
    let entries = feed
        .entries
        .iter()
        .skip(start.min(feed.entries.len()))
        .take(end.saturating_sub(start))
        .collect::<Vec<_>>();
    let has_next_page = end < feed.entries.len();
    let mut result = service
        .store_processed_results(
            &category.title,
            page,
            entries
                .iter()
                .map(|entry| (source.clone(), entry_card(entry)))
                .collect(),
            Vec::new(),
        )
        .await?;
    let search_ref = ResourceRef::new(
        result["resource"]["resourceId"]
            .as_str()
            .ok_or_else(|| "RSS results resource is missing its ID".to_owned())?,
    )
    .map_err(|error| error.to_string())?;
    let mut public_results = service
        .resource_store()
        .read_json_ref(&search_ref)
        .await
        .map_err(|error| error.to_string())?;
    let reader_defaults = reader_defaults(service).await;
    let cards = public_results
        .get_mut("results")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "RSS results document is invalid".to_owned())?;
    if cards.len() != entries.len() {
        return Err("RSS results could not be matched to parsed feed entries".to_owned());
    }
    for (card, entry) in cards.iter_mut().zip(entries.iter()) {
        let article_id = card["resultId"]
            .as_str()
            .ok_or_else(|| "RSS article result ID is missing".to_owned())?
            .to_owned();
        let content = entry_html(entry);
        let chapter_ref = service
            .resource_store()
            .write_chapter_html(&source.id, &article_id, &content, &reader_defaults)
            .await
            .map_err(|error| error.to_string())?;
        // Persist a stable resource:// reference. The resource server turns it
        // into the current loopback URL only while serving this JSON document.
        card["contentSrc"] = json!(chapter_ref.as_str());

        // The article body has a browser-consumable HTML resource now. Keep
        // only processed display metadata in the private result JSON.
        let private_path = Path::new("search-results").join(format!("{article_id}.json"));
        let mut private_result = service.read_private_json(&private_path).await?;
        if let Some(book) = private_result
            .get_mut("book")
            .and_then(Value::as_object_mut)
        {
            book.remove("rssHtml");
        }
        service
            .write_private_json(private_path, &private_result)
            .await?;
    }
    service
        .resource_store()
        .write_json_ref(&search_ref, &public_results)
        .await
        .map_err(|error| error.to_string())?;
    result["sourceId"] = json!(source.id);
    result["categoryId"] = json!(category.category_id);
    result["page"] = json!(page);
    result["hasNextPage"] = json!(has_next_page);
    Ok(result)
}

/// Resolve a parsed feed article to its existing sanitized HTML resource.
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
    let private = service
        .read_private_json(Path::new("search-results").join(format!("{article_id}.json")))
        .await?;
    if private.get("sourceId").and_then(Value::as_str) != Some(source_id) {
        return Err("RSS article is unavailable; refresh the subscription".to_owned());
    }
    let resource = service
        .resource_store()
        .chapter_ref(source_id, article_id)
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "sourceId": source_id,
        "articleId": article_id,
        "resource": service.resource_descriptor(&resource),
    }))
}

async fn reader_defaults(service: &ApplicationService) -> ReaderDefaults {
    service
        .resource_store()
        .read_json_ref(&service.resource_store().settings_ref())
        .await
        .ok()
        .and_then(|settings| settings.get("reader").cloned())
        .and_then(|reader| serde_json::from_value::<ReaderDefaults>(reader).ok())
        .unwrap_or_default()
}

fn entry_card(entry: &Entry) -> Value {
    let title = entry
        .title
        .as_ref()
        .map(|title| title.content.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(entry.id.as_str());
    let author = entry
        .authors
        .first()
        .map(|person| person.name.trim())
        .filter(|author| !author.is_empty())
        .unwrap_or_default();
    let intro = entry
        .summary
        .as_ref()
        .map(|summary| summary.content.clone())
        .unwrap_or_default();
    let published = entry
        .published
        .or(entry.updated)
        .map(|date| date.to_rfc3339());
    json!({
        "name": truncate(title, 512),
        "author": truncate(author, 512),
        "intro": truncate(&intro, 8_192),
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
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("Invalid {field}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
        path::Path,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    use crate::{
        application::{ApplicationService, EngineFuture, SourceExecutor},
        resources::ResourceRef,
        source_engine::SourceEngineRequest,
    };
    use serde_json::json;

    use super::{is_legacy_rss_source, is_rss_source, list_rss_articles, list_rss_categories};

    struct UnusedExecutor;

    impl SourceExecutor for UnusedExecutor {
        fn execute<'a>(&'a self, _request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async { Err("standard feeds do not use source rules".to_owned()) })
        }
    }

    struct FeedFixture {
        address: std::net::SocketAddr,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl FeedFixture {
        fn start() -> Self {
            let listener =
                TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("bind feed fixture");
            let address = listener.local_addr().expect("feed fixture address");
            listener
                .set_nonblocking(true)
                .expect("set fixture listener nonblocking");
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread = thread::spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if thread_stop.load(Ordering::SeqCst) {
                                break;
                            }
                            serve_feed(stream);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                address,
                stop,
                thread: Some(thread),
            }
        }

        fn url(&self) -> String {
            format!("http://{}/feed.xml", self.address)
        }
    }

    impl Drop for FeedFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn serve_feed(stream: TcpStream) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.is_empty() {
            return;
        }
        loop {
            line.clear();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                return;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
            <feed xmlns="http://www.w3.org/2005/Atom">
              <id>https://private.example/feed</id><title>Fixture Feed</title>
              <entry><id>entry-one</id><title>测试文章</title>
                <summary type="html">&lt;p&gt;正文中文&lt;/p&gt;&lt;script&gt;alert(1)&lt;/script&gt;</summary>
              </entry>
              <entry><id>entry-two</id><title>第二篇</title><summary>纯文本</summary></entry>
            </feed>"#;
        let mut stream = reader.into_inner();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/atom+xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("legado-rss-{}", uuid::Uuid::new_v4().simple()))
    }

    async fn json_get(url: &str) -> serde_json::Value {
        let body = reqwest::get(url)
            .await
            .expect("resource request")
            .error_for_status()
            .expect("resource status")
            .text()
            .await
            .expect("resource body");
        serde_json::from_str(&body).expect("resource JSON")
    }

    #[test]
    fn detects_source_envelopes_without_inspecting_rules() {
        assert!(is_legacy_rss_source(&json!({
            "sourceUrl": "https://example.test/rss",
            "ruleArticles": "@css:item"
        })));
        assert!(is_rss_source(&json!({ "bookSourceType": 5 })));
        assert!(!is_rss_source(
            &json!({ "bookSourceType": 0, "ruleSearch": "secret" })
        ));
        assert!(!is_legacy_rss_source(&json!({
            "bookSourceUrl": "https://example.test",
            "bookSourceType": 0,
            "ruleExplore": "secret"
        })));
    }

    #[tokio::test]
    async fn standard_atom_subscription_publishes_processed_cards_and_html_resources() {
        let fixture = FeedFixture::start();
        let root = temp_root();
        let service = ApplicationService::open_with_executor(&root, Arc::new(UnusedExecutor))
            .await
            .expect("open app service");
        let imported = service
            .import_sources(
                &json!([{
                    "sourceName": "Fixture Feed",
                    "sourceUrl": fixture.url()
                }])
                .to_string(),
            )
            .await
            .expect("import plain feed");
        let source_id = imported["sources"][0]["id"]
            .as_str()
            .expect("source ID")
            .to_owned();

        let categories = list_rss_categories(&service, &source_id)
            .await
            .expect("list feed category");
        let category_doc = json_get(categories["resource"]["src"].as_str().unwrap()).await;
        assert_eq!(category_doc["categories"][0]["title"], "最新文章");
        let category_id = category_doc["categories"][0]["categoryId"]
            .as_str()
            .expect("opaque category ID");

        let page = list_rss_articles(&service, &source_id, category_id, 1)
            .await
            .expect("list parsed articles");
        assert_eq!(page["bookCount"], 2);
        let result_doc = json_get(page["resource"]["src"].as_str().unwrap()).await;
        assert_eq!(result_doc["results"][0]["title"], "测试文章");
        let public_text = result_doc.to_string();
        assert!(!public_text.contains(&fixture.url()));
        assert!(!public_text.contains("rssHtml"));
        assert!(!public_text.contains("rule"));
        let article_id = result_doc["results"][0]["resultId"]
            .as_str()
            .expect("article ID");
        let content_src = result_doc["results"][0]["contentSrc"]
            .as_str()
            .expect("materialized article source");
        assert!(content_src.starts_with("http://127.0.0.1:"));

        let html = reqwest::get(content_src)
            .await
            .expect("article HTML request")
            .error_for_status()
            .expect("article HTML status")
            .text()
            .await
            .expect("article HTML body");
        assert!(html.contains("正文中文"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("alert(1)"));
        let private = service
            .read_private_json(Path::new("search-results").join(format!("{article_id}.json")))
            .await
            .expect("private result remains available");
        assert!(private["book"].get("rssHtml").is_none());

        let opened = super::read_standard_feed_article(&service, &source_id, article_id)
            .await
            .expect("resolve cached HTML resource");
        assert_eq!(
            opened["resource"]["resourceId"],
            format!("resource://books/{source_id}/chapters/{article_id}.html")
        );
        let stable_search_ref = ResourceRef::new(page["resource"]["resourceId"].as_str().unwrap())
            .expect("stable search reference");
        let on_disk_results = service
            .resource_store()
            .read_json_ref(&stable_search_ref)
            .await
            .expect("public result JSON");
        assert_eq!(
            on_disk_results["results"][0]["contentSrc"],
            format!("resource://books/{source_id}/chapters/{article_id}.html")
        );

        drop(service);
        let reopened = ApplicationService::open_with_executor(&root, Arc::new(UnusedExecutor))
            .await
            .expect("reopen app service");
        let reopened_search_descriptor = reopened.resource_descriptor(&stable_search_ref);
        let reopened_results = json_get(reopened_search_descriptor["src"].as_str().unwrap()).await;
        let reopened_content_src = reopened_results["results"][0]["contentSrc"]
            .as_str()
            .expect("re-materialized article source");
        assert!(reopened_content_src.starts_with("http://127.0.0.1:"));
        let persisted = super::read_standard_feed_article(&reopened, &source_id, article_id)
            .await
            .expect("resolve persisted HTML after restart");
        assert_eq!(
            persisted["resource"]["resourceId"],
            format!("resource://books/{source_id}/chapters/{article_id}.html")
        );
        let persisted_html = reqwest::get(reopened_content_src)
            .await
            .expect("persisted HTML request")
            .error_for_status()
            .expect("persisted HTML status")
            .text()
            .await
            .expect("persisted HTML body");
        assert!(persisted_html.contains("正文中文"));
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
