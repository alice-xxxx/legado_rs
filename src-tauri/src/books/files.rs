//! 书籍正文的净化与缓存命名，文件操作交给 cache。
use crate::resources::{ResourceError, ResourceRef, Resources};
use serde_json::{json, Value};

pub(crate) fn kind(
    database: &crate::database::Database,
    book: &str,
) -> Result<String, ResourceError> {
    Ok(database.book_type(book)?.unwrap_or_else(|| {
        if book.starts_with("rss-") {
            "html"
        } else {
            "text"
        }
        .to_owned()
    }))
}

pub(crate) fn content_ref(
    kind: &str,
    book: &str,
    chapter: &str,
    body: &str,
    html: bool,
) -> Result<ResourceRef, ResourceError> {
    let name = format!(
        "{}-{}.{}",
        crate::support::sha256_hex(chapter.as_bytes()),
        crate::support::sha256_hex(body.as_bytes()),
        if html { "html" } else { "txt" }
    );
    ResourceRef::new(format!("resource://cache/{kind}/{book}/{name}"))
}

/// 在线相对地址使用来源 URL；本地 EPUB 保留相对地址，随后改为稳定缓存引用。
pub(crate) fn html_document(
    kind: &str,
    book: &str,
    chapter: &str,
    body: &str,
    base: Option<&str>,
    local: bool,
) -> Result<(ResourceRef, Value), ResourceError> {
    let mut cleaner = ammonia::Builder::default();
    cleaner
        .add_tags(&["link"])
        .add_tag_attributes("link", &["href", "rel", "type", "media"])
        .add_tag_attribute_values("link", "rel", &["stylesheet"])
        .add_tag_attributes("a", &["data-legado-chapter-id", "data-legado-fragment"]);
    let relative = if local {
        ammonia::UrlRelative::PassThrough
    } else if let Some(url) = base
        .and_then(|base| reqwest::Url::parse(base).ok())
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
        })
    {
        ammonia::UrlRelative::RewriteWithBase(url)
    } else {
        ammonia::UrlRelative::Deny
    };
    cleaner.url_relative(relative);
    let mut markup = cleaner.clean(body).to_string();
    if local {
        for attribute in ["src", "href", "poster"] {
            for quote in ['\'', '"'] {
                markup = markup.replace(
                    &format!("{attribute}={quote}../assets/"),
                    &format!("{attribute}={quote}resource://cache/{kind}/{book}/"),
                );
            }
        }
    }
    let reference = content_ref(kind, book, chapter, &markup, true)?;
    Ok((
        reference,
        json!({"schemaVersion":crate::models::CURRENT_SCHEMA_VERSION,"kind":"richText","markup":markup}),
    ))
}

pub(crate) async fn text(
    resources: &Resources,
    kind: &str,
    book: &str,
    chapter: &str,
    body: &str,
) -> Result<ResourceRef, ResourceError> {
    if kind == "html" {
        let markup = format!("<pre>{}</pre>", quick_xml::escape::escape(body));
        return html(resources, kind, book, chapter, &markup, None, true).await;
    }
    let reference = content_ref(kind, book, chapter, body, false)?;
    resources.cache.save_content(&reference, body).await?;
    Ok(reference)
}

pub(crate) async fn html(
    resources: &Resources,
    kind: &str,
    book: &str,
    chapter: &str,
    body: &str,
    base: Option<&str>,
    local: bool,
) -> Result<ResourceRef, ResourceError> {
    let (reference, document) = html_document(kind, book, chapter, body, base, local)?;
    resources
        .cache
        .save_content(&reference, document["markup"].as_str().unwrap())
        .await?;
    Ok(reference)
}

impl crate::application::ApplicationService {
    pub(crate) async fn read_book(&self, id: &str) -> Result<Value, String> {
        let database = self.database.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let book = database
                .book(&id)
                .map_err(|error| error.to_string())?
                .ok_or("书籍不存在")?;
            serde_json::to_value(book).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }
}
