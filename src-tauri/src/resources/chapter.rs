//! 正文的文本、图片和媒体资源转换。
use super::{ChapterVideoSource, ResourceRef, ResourceServer};
use crate::{
    models::CURRENT_SCHEMA_VERSION,
    source_engine::SourceEngineRequest,
    source_engine::SourceExecutor,
    support::{looks_like_html, sha256_hex, text_at},
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use std::collections::VecDeque;

struct VideoMediaCandidate {
    source_index: usize,
    label: String,
    url: Option<String>,
    headers: Option<HashMap<String, String>>,
}

/// 只转换正文资源，不改动书籍业务记录。
pub(crate) async fn chapter_content(
    server: &ResourceServer,
    executor: &dyn SourceExecutor,
    book_id: &str,
    chapter_id: &str,
    request: &SourceEngineRequest,
    response: Value,
    mut chapter: Value,
) -> Result<Value, String> {
    let content = response
        .as_str()
        .ok_or("Source engine returned non-text chapter content")?;
    if content.trim().is_empty() {
        return Err("Source engine returned empty chapter content; no cache was written".into());
    }
    let base = request
        .chapter
        .as_ref()
        .and_then(|chapter| text_at(chapter, &["url", "chapterUrl"]))
        .or_else(|| {
            request
                .book
                .as_ref()
                .and_then(|book| text_at(book, &["bookUrl", "url"]))
        });
    // 刷新只替换缓存字段，章节身份和阅读进度仍由目录管理。
    for key in [
        "resource", "url", "pages", "sources", "protocol", "media", "files",
    ] {
        chapter.as_object_mut().unwrap().remove(key);
    }
    let (reference, format) = match crate::source_catalog::content_type(&request.source) {
        kind @ ("audio" | "video") => {
            let (candidates, default) = if kind == "video" {
                video_media_candidates(content)?
            } else {
                (
                    vec![VideoMediaCandidate {
                        source_index: 0,
                        label: String::new(),
                        url: Some(content.trim().into()),
                        headers: None,
                    }],
                    0,
                )
            };
            let mut sources = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let mut media = request.clone();
                media.media_url = candidate.url;
                media.media_headers = candidate.headers;
                let resolved = if media.media_url.is_some() {
                    resolve_chapter_media(
                        server,
                        executor,
                        book_id,
                        chapter_id,
                        candidate.source_index,
                        media,
                    )
                    .await
                } else {
                    Err(VIDEO_ADDRESS_UNAVAILABLE.into())
                };
                let (media_ref, protocol, unavailable_reason) = match resolved {
                    Ok((reference, protocol)) => (Some(reference), Some(protocol), None),
                    Err(error) if kind == "audio" => return Err(error),
                    Err(_) => (None, None, Some(VIDEO_RESOLUTION_UNAVAILABLE)),
                };
                sources.push(ChapterVideoSource {
                    label: candidate.label,
                    media_ref,
                    media_format: protocol,
                    unavailable_reason,
                    selected: candidate.source_index == default,
                });
            }
            // 默认清晰度不可用时选择第一个可用地址。
            let selected = sources
                .iter()
                .position(|source| source.selected && source.media_ref.is_some())
                .or_else(|| sources.iter().position(|source| source.media_ref.is_some()))
                .ok_or("Video source has no playable HTTP media resolution")?;
            for (index, source) in sources.iter_mut().enumerate() {
                source.selected = index == selected;
            }
            let reference = sources[selected].media_ref.as_ref().unwrap();
            let protocol = sources[selected].media_format.unwrap();
            let (_, document) = server
                .store
                .chapter_media_document(
                    book_id,
                    chapter_id,
                    reference,
                    kind,
                    protocol,
                    if kind == "video" { &sources } else { &[] },
                )
                .map_err(|error| error.to_string())?;
            chapter["protocol"] = json!(protocol);
            if kind == "video" {
                chapter["sources"] = document["sources"].clone();
            }
            (reference.clone(), kind)
        }
        "image" => {
            chapter["pages"] = source_image_pages(
                server,
                executor,
                book_id,
                chapter_id,
                request,
                content,
                base.as_deref(),
            )
            .await?;
            return Ok(chapter);
        }
        _ if looks_like_html(content) => {
            let (reference, document) = source_rich_text_document(
                server,
                executor,
                book_id,
                chapter_id,
                request,
                content,
                base.as_deref(),
            )
            .await?;
            server
                .store
                .write_json_ref(&reference, &document)
                .await
                .map_err(|error| error.to_string())?;
            (reference, "richText")
        }
        _ => (
            server
                .store
                .write_chapter_text(book_id, chapter_id, content)
                .await
                .map_err(|error| error.to_string())?,
            "text",
        ),
    };
    chapter["resource"] = json!(crate::models::ChapterResourceDescriptor::for_chapter(
        &reference, format
    ));
    Ok(chapter)
}

/// 音频、视频和正文图片共用解析与登记，避免先准备再重复登记。
async fn resolve_chapter_media(
    server: &ResourceServer,
    executor: &dyn SourceExecutor,
    book_id: &str,
    chapter_id: &str,
    variant: usize,
    mut request: SourceEngineRequest,
) -> Result<(ResourceRef, &'static str), String> {
    request.operation = "resolveMedia".into();
    let resolved = executor.execute(request).await?;
    let (url, headers) = resolved_media_result(&resolved)?;
    if !is_direct_http_media_url(&url) {
        return Err("Unsupported media URL".into());
    }
    let protocol = if is_hls_media_url(&url) {
        MEDIA_FORMAT_HLS
    } else {
        MEDIA_FORMAT_DIRECT
    };
    let id = chapter_media_variant_id(book_id, chapter_id, variant, &url, &headers);
    let reference = server
        .register_media_with_id(book_id, &id, &url, &headers)
        .await
        .map_err(|error| error.to_string())?;
    Ok((reference, protocol))
}
async fn source_image_pages(
    server: &ResourceServer,
    executor: &dyn SourceExecutor,
    book_id: &str,
    chapter_id: &str,
    context: &SourceEngineRequest,
    content: &str,
    base_url: Option<&str>,
) -> Result<Value, String> {
    // 直接读取 HTML、JSON 或逐行图片地址，不先拼成 HTML 再拆回来。
    let addresses = if content.contains('<') {
        let images = regex::Regex::new(r#"(?i)<img\b[^>]*\bsrc\s*=\s*["']([^"']+)["']"#)
            .map_err(|error| error.to_string())?;
        images
            .captures_iter(content)
            .map(|capture| {
                quick_xml::escape::unescape(&capture[1])
                    .map(|address| address.into_owned())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        serde_json::from_str::<Vec<String>>(content.trim())
            .unwrap_or_else(|_| content.lines().map(str::to_owned).collect())
    };
    let mut pages = Vec::new();
    for (index, address) in addresses
        .into_iter()
        .map(|address| address.trim().to_owned())
        .filter(|address| !address.is_empty())
        .enumerate()
    {
        if address.starts_with("resource://media/") {
            let reference = ResourceRef::new(&address).map_err(|error| error.to_string())?;
            let opaque_id = reference
                .path()
                .strip_prefix("media/")
                .ok_or("Invalid image proxy")?;
            if !server.store.database().has_media(book_id, opaque_id).map_err(|e| e.to_string())? {
                return Err("Image proxy does not belong to this content".into());
            }
            pages.push(json!({"id": format!("{}-p{}", chapter_id, index + 1), "url": address}));
            continue;
        }
        let address = if is_direct_http_media_url(&address) {
            address
        } else {
            let base = base_url
                .and_then(|url| reqwest::Url::parse(url).ok())
                .ok_or_else(|| "Image chapter has no valid base URL".to_owned())?;
            base.join(&address)
                .map_err(|error| error.to_string())?
                .to_string()
        };
        if !is_direct_http_media_url(&address) {
            return Err("Image chapter contains an unsupported image URL".into());
        }
        let mut request = context.clone();
        request.media_url = Some(address);
        let (reference, _) =
            resolve_chapter_media(server, executor, book_id, chapter_id, 0, request).await?;
        pages.push(
            json!({"id": format!("{}-p{}", chapter_id, index + 1), "url": reference.as_str()}),
        );
    }
    if pages.is_empty() {
        return Err("Image chapter contains no images".into());
    }
    Ok(json!(pages))
}

/// 图片经书源规则解析，私有请求头留在 Rust。
async fn source_rich_text_document(
    server: &ResourceServer,
    executor: &dyn SourceExecutor,
    book_id: &str,
    chapter_id: &str,
    context: &SourceEngineRequest,
    content: &str,
    base_url: Option<&str>,
) -> Result<(ResourceRef, Value), String> {
    if server.store.book_type(book_id).map_err(|e| e.to_string())? != "image" {
        server
            .store
            .bind_book_type(book_id, "html")
            .await
            .map_err(|e| e.to_string())?;
    }
    let (_, mut document) = server
        .store
        .online_chapter_rich_text_document(book_id, chapter_id, content, base_url)
        .map_err(|error| error.to_string())?;
    let markup = document["markup"].as_str().unwrap_or_default();
    // HTML 已经过清理，图片属性使用标准双引号。
    let images = regex::Regex::new(r#"(?i)<img\b[^>]*\ssrc="([^"]*)""#)
        .map_err(|error| error.to_string())?;
    let mut output = String::with_capacity(markup.len());
    let mut cursor = 0;
    let mut resolved_images = HashMap::<String, ResourceRef>::new();
    let addresses = images
        .captures_iter(markup)
        .map(|image| image.get(1).expect("image address capture"))
        .collect::<Vec<_>>();
    for address in addresses {
        let input = quick_xml::escape::unescape(address.as_str())
            .map_err(|error| error.to_string())?
            .into_owned();
        if !is_direct_http_media_url(&input) {
            continue;
        }
        let reference = if let Some(reference) = resolved_images.get(&input) {
            reference.clone()
        } else {
            let mut request = context.clone();
            request.media_url = Some(input.clone());
            let (reference, _) =
                resolve_chapter_media(server, executor, book_id, chapter_id, 0, request).await?;
            resolved_images.insert(input, reference.clone());
            reference
        };
        output.push_str(&markup[cursor..address.start()]);
        output.push_str(reference.as_str());
        cursor = address.end();
    }
    output.push_str(&markup[cursor..]);
    document["markup"] = json!(output);
    let reference = server
        .store
        .chapter_version_ref(book_id, chapter_id, &document)
        .map_err(|error| error.to_string())?;
    Ok((reference, document))
}

/// 缓存地址包含 URL 和请求头，刷新不会改变旧地址的含义。
fn chapter_media_id(
    book_id: &str,
    chapter_id: &str,
    upstream_url: &str,
    headers: &[(String, String)],
) -> String {
    let fingerprint = serde_json::to_vec(&(book_id, chapter_id, upstream_url, headers))
        .expect("media request identity is serializable");
    format!("chapter-media-{}", sha256_hex(&fingerprint))
}

fn chapter_media_variant_id(
    book_id: &str,
    chapter_id: &str,
    source_index: usize,
    upstream_url: &str,
    headers: &[(String, String)],
) -> String {
    let base = chapter_media_id(book_id, chapter_id, upstream_url, headers);
    if source_index == 0 {
        base
    } else {
        format!("{base}-variant-{source_index}")
    }
}

/// 兼容视频清晰度列表、名称::地址及单地址。
fn video_media_candidates(content: &str) -> Result<(Vec<VideoMediaCandidate>, usize), String> {
    let content = content.trim();
    if content.starts_with("#BASE:") || content.starts_with("#EXTM3U") {
        return Err("Memory HLS video sources are not supported by the current reader".into());
    }

    if content.starts_with('{') {
        let source = serde_json::from_str::<Value>(content)
            .map_err(|_| "Video source JSON is invalid".to_owned())?;
        let resolutions = source
            .get("resolutions")
            .and_then(Value::as_array)
            .ok_or_else(|| "Video source has no resolution list".to_owned())?;
        if resolutions.len() > MAX_VIDEO_MEDIA_VARIANTS {
            return Err(format!(
                "Video source exceeds the supported limit of {MAX_VIDEO_MEDIA_VARIANTS} resolutions"
            ));
        }
        let index = match source.get("defaultIndex") {
            None => 0,
            Some(value) => value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    "Video source defaultIndex must be a nonnegative integer".to_owned()
                })?,
        };
        if resolutions.get(index).is_none() {
            return Err(
                "Video source default resolution is outside its resolution list".to_owned(),
            );
        }
        let headers = match source.get("headers") {
            None | Some(Value::Null) => None,
            Some(Value::Object(headers)) => Some(
                headers
                    .iter()
                    .map(|(name, value)| {
                        value
                            .as_str()
                            .map(|value| (name.clone(), value.to_owned()))
                            .ok_or_else(|| "Video source headers must be strings".to_owned())
                    })
                    .collect::<Result<HashMap<_, _>, _>>()?,
            ),
            Some(_) => return Err("Video source headers must be an object".into()),
        };
        let candidates = resolutions
            .iter()
            .enumerate()
            .map(|(source_index, resolution)| {
                let label = resolution
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| bounded_video_label(name, source_index))
                    .unwrap_or_else(|| format!("清晰度 {}", source_index + 1));
                let url = resolution
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(str::to_owned);
                VideoMediaCandidate {
                    source_index,
                    label,
                    url,
                    headers: headers.clone(),
                }
            })
            .collect();
        return Ok((candidates, index));
    }

    if !has_http_media_scheme(content) && content.contains("::") {
        let mut candidates = Vec::new();
        for line in content.lines() {
            if let Some((name, url)) = line.split_once("::") {
                let source_index = candidates.len();
                let label = name.trim().chars().take(80).collect::<String>();
                candidates.push(VideoMediaCandidate {
                    source_index,
                    label: if label.is_empty() {
                        format!("清晰度 {}", source_index + 1)
                    } else {
                        label
                    },
                    url: (!url.trim().is_empty()).then(|| url.trim().to_owned()),
                    headers: None,
                });
            }
        }
        if candidates.is_empty() {
            return Err("Video source has no direct URL in its resolution list".into());
        }
        if candidates.len() > MAX_VIDEO_MEDIA_VARIANTS {
            return Err(format!(
                "Video source exceeds the supported limit of {MAX_VIDEO_MEDIA_VARIANTS} resolutions"
            ));
        }
        return Ok((candidates, 0));
    }

    if !has_http_media_scheme(content) {
        return Err("Video source must provide a direct HTTP(S) media URL".into());
    }
    Ok((
        vec![VideoMediaCandidate {
            source_index: 0,
            label: "默认".to_owned(),
            url: Some(content.to_owned()),
            headers: None,
        }],
        0,
    ))
}

fn bounded_video_label(label: &str, source_index: usize) -> String {
    let label = label.chars().take(80).collect::<String>();
    if label.trim().is_empty() {
        format!("清晰度 {}", source_index + 1)
    } else {
        label
    }
}

fn has_http_media_scheme(value: &str) -> bool {
    value.split_once("://").is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

fn resolved_media_result(resolved: &Value) -> Result<(String, Vec<(String, String)>), String> {
    let media_url = resolved
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "Source engine returned no resolved media URL".to_owned())?
        .to_owned();
    let headers = resolved
        .get("headers")
        .and_then(Value::as_object)
        .ok_or_else(|| "Source engine returned invalid media headers".to_owned())?
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), value.to_owned()))
                .ok_or_else(|| "Source engine returned an invalid media header".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((media_url, headers))
}

fn is_direct_http_media_url(value: &str) -> bool {
    reqwest::Url::parse(value).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn is_hls_media_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .ok()
        .is_some_and(|url| url.path().to_ascii_lowercase().ends_with(".m3u8"))
}

/// cache=true 时在正文提交前下载完整资源；取消或失败都不提交新章节地址。
pub(crate) async fn cache_chapter_files(
    server: &ResourceServer,
    book_id: &str,
    chapter_id: &str,
    chapter: Value,
) -> Result<Value, String> {
    let kind = server
        .store
        .book_type(book_id)
        .map_err(|error| error.to_string())?;
    let mut markup = None;
    let mut roots = Vec::new();
    if kind == "image" {
        for page in chapter["pages"].as_array().into_iter().flatten() {
            if let Some(url) = page["url"].as_str() {
                roots.push(url.to_owned());
            }
        }
    } else if matches!(kind.as_str(), "audio" | "video") {
        if let Some(url) = chapter["resource"]["resourceId"]
            .as_str()
            .or_else(|| chapter["url"].as_str())
        {
            roots.push(url.to_owned());
        }
        for source in chapter["sources"].as_array().into_iter().flatten() {
            if let Some(url) = source["url"].as_str().or_else(|| source["src"].as_str()) {
                roots.push(url.to_owned());
            }
        }
    } else if kind == "html" {
        if let Some(raw) = chapter["resource"]["resourceId"].as_str() {
            let reference = ResourceRef::new(raw).map_err(|e| e.to_string())?;
            let document = server
                .store
                .read_json_ref(&reference)
                .await
                .map_err(|e| e.to_string())?;
            let body = document["markup"].as_str().unwrap_or_default().to_owned();
            let regex =
                regex::Regex::new(r#"resource://media/[^\s"'<>]+"#).map_err(|e| e.to_string())?;
            roots.extend(
                regex
                    .find_iter(&body)
                    .map(|value| value.as_str().to_owned()),
            );
            markup = Some(body);
        }
    }
    // 本地文件无需重复下载。
    roots.retain(|url| url.starts_with("resource://media/"));
    if roots.is_empty() {
        return Ok(chapter);
    }
    let mut names = HashMap::<String, String>::new();
    let mut queue = VecDeque::new();
    for root in &roots {
        let reference = ResourceRef::new(root).map_err(|e| e.to_string())?;
        let url = server.url_for(&reference);
        if !names.contains_key(&url) {
            queue.push_back(url.clone());
            names.insert(url, String::new());
        }
    }
    let client = crate::http_client::raw_builder()
        .build()
        .map_err(|e| e.to_string())?;
    let mut payloads = Vec::new();
    let mut files = Vec::new();
    let uri_attributes = regex::Regex::new(r#"URI="([^"]+)""#).map_err(|e| e.to_string())?;
    while let Some(url) = queue.pop_front() {
        let mut response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?
            .error_for_status()
            .map_err(|e| e.without_url().to_string())?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if bytes.len().saturating_add(chunk.len()) > 512 * 1024 * 1024 {
                return Err("A downloaded resource exceeds 512 MiB".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Err("Downloaded resource is empty".into());
        }
        let playlist = bytes.starts_with(b"#EXTM3U");
        let extension = if playlist {
            "m3u8"
        } else if content_type.contains("image/png") {
            "png"
        } else if content_type.contains("image/webp") {
            "webp"
        } else if content_type.contains("image/gif") {
            "gif"
        } else if content_type.contains("image/") {
            "jpg"
        } else if content_type.contains("audio/aac") {
            "aac"
        } else if content_type.contains("audio/mp4") {
            "m4a"
        } else if content_type.contains("audio/ogg") {
            "ogg"
        } else if content_type.contains("audio/wav") {
            "wav"
        } else if content_type.contains("audio/") {
            "mp3"
        } else if content_type.contains("video/mp4") {
            "mp4"
        } else if content_type.contains("webm") {
            "webm"
        } else if kind == "audio" {
            "mp3"
        } else {
            "ts"
        };
        let stable_id = url
            .strip_prefix(server.base_url())
            .ok_or("Download escaped the resource server")?;
        let name = format!(
            "{}-{}.{}",
            chapter_id,
            sha256_hex(stable_id.as_bytes()),
            extension
        );
        names.insert(url.clone(), name.clone());
        let mut dependencies = Vec::new();
        if playlist {
            let body = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
            if !body.contains("#EXT-X-ENDLIST") && !body.contains("#EXT-X-STREAM-INF") {
                return Err("Live playlists cannot be saved as a complete offline chapter".into());
            }
            for line in body.lines() {
                let line = line.trim();
                let refs = if line.starts_with('#') {
                    uri_attributes
                        .captures_iter(line)
                        .map(|c| c[1].to_owned())
                        .collect::<Vec<_>>()
                } else if line.is_empty() {
                    Vec::new()
                } else {
                    vec![line.to_owned()]
                };
                for raw in refs {
                    let child = reqwest::Url::parse(&url)
                        .and_then(|base| base.join(&raw))
                        .map_err(|e| e.to_string())?
                        .to_string();
                    if !child.starts_with(server.base_url()) {
                        return Err("Playlist dependency escaped the resource server".into());
                    }
                    if !names.contains_key(&child) {
                        if names.len() >= 8192 {
                            return Err("Offline playlist has too many dependencies".into());
                        }
                        names.insert(child.clone(), String::new());
                        queue.push_back(child.clone());
                    }
                    dependencies.push((raw, child));
                }
            }
        }
        if playlist {
            payloads.push((url, name, bytes, dependencies));
        } else {
            server
                .store
                .write_asset(book_id, &name, &bytes)
                .await
                .map_err(|e| e.to_string())?;
            files.push(name);
        }
    }
    for (_, name, bytes, dependencies) in payloads {
        let bytes = if dependencies.is_empty() {
            bytes
        } else {
            let mut body = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            // 先替换长地址，避免短地址误替换另一条地址的前缀。
            let mut dependencies = dependencies;
            dependencies.sort_by_key(|(raw, _)| std::cmp::Reverse(raw.len()));
            for (raw, child) in dependencies {
                body = body.replace(&raw, names.get(&child).ok_or("Missing offline dependency")?);
            }
            body.into_bytes()
        };
        server
            .store
            .write_asset(book_id, &name, &bytes)
            .await
            .map_err(|e| e.to_string())?;
        files.push(name);
    }
    let replacements = roots
        .iter()
        .map(|root| {
            let reference = ResourceRef::new(root).map_err(|e| e.to_string())?;
            let url = server.url_for(&reference);
            let name = names.get(&url).ok_or("Missing downloaded resource")?;
            Ok((
                root.clone(),
                server
                    .store
                    .asset_ref(book_id, name)
                    .map_err(|e| e.to_string())?
                    .to_string(),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let mut next_chapter = chapter.clone();
    rewrite_downloaded_refs(&mut next_chapter, &replacements);
    if let Some(mut body) = markup {
        for (from, to) in &replacements {
            body = body.replace(from, to);
        }
        // 正文已经清理过；这里只替换地址，重复清理会删掉 resource:// 本地图片。
        let document =
            json!({"schemaVersion":CURRENT_SCHEMA_VERSION,"kind":"richText","markup":body});
        let reference = server
            .store
            .chapter_version_ref(book_id, chapter_id, &document)
            .map_err(|e| e.to_string())?;
        server
            .store
            .write_json_ref(&reference, &document)
            .await
            .map_err(|e| e.to_string())?;
        next_chapter["resource"] = serde_json::to_value(
            crate::models::ChapterResourceDescriptor::for_chapter(&reference, "richText"),
        )
        .map_err(|e| e.to_string())?;
    }
    let roots = replacements
        .values()
        .filter_map(|url| url.rsplit('/').next())
        .collect::<HashSet<_>>();
    files.retain(|file| !roots.contains(file.as_str()));
    if !files.is_empty() {
        next_chapter["files"] = json!(files);
    }
    Ok(next_chapter)
}

fn rewrite_downloaded_refs(value: &mut Value, replacements: &HashMap<String, String>) {
    match value {
        Value::String(url) => {
            if let Some(next) = replacements.get(url) {
                *url = next.clone();
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite_downloaded_refs(value, replacements);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if !matches!(key.as_str(), "engineurl" | "engineData" | "engineTitle") {
                    rewrite_downloaded_refs(value, replacements);
                }
            }
        }
        _ => {}
    }
}

const MAX_VIDEO_MEDIA_VARIANTS: usize = 16;
const VIDEO_ADDRESS_UNAVAILABLE: &str = "媒体地址无效";
const VIDEO_RESOLUTION_UNAVAILABLE: &str = "清晰度解析失败";
const MEDIA_FORMAT_HLS: &str = "hls";
const MEDIA_FORMAT_DIRECT: &str = "direct";
