//! 书籍正文的净化与缓存命名，文件操作交给 cache。
use crate::resources::{ResourceError, ResourceRef, Resources};
use serde::{Deserialize, Serialize};
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
        json!({"schemaVersion":1,"kind":"richText","markup":markup}),
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
        .save_content(&reference, document["markup"].as_str().ok_or_else(|| ResourceError::new("正文 markup 必须是字符串"))?)
        .await?;
    Ok(reference)
}

/// 阅读预加载先检查缓存；显式正文请求则直接刷新。
pub(crate) async fn chapter_ready(
    resources: &crate::resources::Resources,
    context: &crate::database::ChapterContext,
) -> Result<bool, String> {
    let chapter = &context.chapter;
    let addresses = match context.content_type.as_str() {
        "image" => {
            if chapter.pages.is_empty() {
                return Ok(false);
            }
            chapter.pages.iter().map(|page| page.src.as_str()).collect()
        }
        "pdf" => context
            .file_url
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        _ => chapter.url.iter().map(String::as_str).collect(),
    };
    if addresses.is_empty() {
        return Ok(false);
    }
    for address in addresses {
        let reference = ResourceRef::new(address).map_err(|error| error.to_string())?;
        let exists = if let Some(id) = reference.path().strip_prefix("proxy/") {
            resources.proxy.contains(id)
        } else {
            resources
                .cache
                .contains(&reference)
                .await
                .map_err(|error| error.to_string())?
        };
        if !exists {
            return Ok(false);
        }
    }
    Ok(true)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterResourceDescriptor {
    /// 章节资源的稳定标识。
    pub resource_id: ResourceRef,
    /// 稳定引用，提供给界面时转换为本地 HTTP 地址。
    pub src: ResourceRef,
    pub content_type: String,
    pub format: String,
    /// 正文类型。
    pub content_format: String,
}

impl ChapterResourceDescriptor {
    pub fn for_chapter(reference: &ResourceRef, content_format: &str) -> Self {
        Self {
            resource_id: reference.clone(),
            src: reference.clone(),
            content_type: match content_format {
                "text" => "text/plain; charset=utf-8",
                "richText" => "text/html; charset=utf-8",
                "pdf" => "application/pdf",
                _ => "application/octet-stream",
            }
            .to_owned(),
            format: match content_format {
                "richText" => "html",
                kind => kind,
            }
            .to_owned(),
            content_format: content_format.to_owned(),
        }
    }
}
