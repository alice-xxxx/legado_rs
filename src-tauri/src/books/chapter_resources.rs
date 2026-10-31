//! 正文转换与离线下载；缓存写入只走 resources.cache。
use tauri_plugin_source_engine::{ContentRequest, MediaRequest, SourceExecutor};
use tokio_util::sync::CancellationToken;
use crate::{
    resources::ResourceRef,
};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};

async fn media(
    resources: &crate::resources::Resources,
    executor: &dyn SourceExecutor,
    book: &str,
    chapter: &str,
    context: &ContentRequest,
    cancellation: &CancellationToken,
    url: &str,
    headers: Option<HashMap<String, String>>,
) -> Result<(ResourceRef, &'static str), String> {
    let resolved = executor.resolve_media(MediaRequest {
        source: context.source.clone(), book: context.book.clone(), chapter: context.chapter.clone(),
        url: url.to_owned(), headers: headers.unwrap_or_default(),
    }, cancellation.clone()).await?;
    let url = resolved.url.as_str();
    let mut headers = resolved.headers.into_iter().collect::<Vec<_>>();
    headers.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let fingerprint =
        serde_json::to_vec(&(book, chapter, url, &headers)).map_err(|e| e.to_string())?;
    let id = format!("chapter-media-{}", crate::support::sha256_hex(&fingerprint));
    let reference = resources.register_proxy(book, &id, url, &headers).await?;
    let hls = reqwest::Url::parse(url)
        .map_err(|e| e.to_string())?
        .path()
        .to_ascii_lowercase()
        .ends_with(".m3u8");
    Ok((reference, if hls { "hls" } else { "direct" }))
}

fn image_urls(body: &str, base: Option<&str>) -> Result<Vec<String>, String> {
    let images = regex::Regex::new(r#"(?i)<img\b[^>]*\bsrc\s*=\s*["']([^"']+)["']"#).map_err(|e| e.to_string())?;
    let html = crate::support::looks_like_html(body);
    let urls: Vec<_> = if html {
        images
            .captures_iter(body)
            .map(|c| {
                quick_xml::escape::unescape(&c[1])
                    .map(|v| v.into_owned())
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<_, _>>()?
    } else if body.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<String>>(body).map_err(|e| e.to_string())?
    } else {
        body.lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    };
    if urls.is_empty() {
        return Err("漫画正文没有图片".into());
    }
    urls.into_iter()
        .map(|url| {
            reqwest::Url::parse(&url)
                .or_else(|_| {
                    reqwest::Url::parse(base.unwrap_or("")).and_then(|base| base.join(&url))
                })
                .map(|url| url.to_string())
                .map_err(|e| e.to_string())
        })
        .collect()
}

pub(crate) async fn chapter_content(
    database: &crate::database::Database,
    resources: &crate::resources::Resources,
    executor: &dyn SourceExecutor,
    book: &str,
    chapter_id: &str,
    context: &ContentRequest,
    response: String,
    mut chapter: Value,
    cancellation: CancellationToken,
) -> Result<Value, String> {
    let body = response.as_str();
    if body.trim().is_empty() { return Err("来源返回空正文".into()); }
    let base = Some(context.chapter.url.clone()).filter(|url| !url.trim().is_empty())
        .or_else(|| context.chapter.extra.get("chapterUrl").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| context.book.book_url.clone().filter(|url| !url.trim().is_empty()))
        .or_else(|| context.book.extra.get("url").and_then(Value::as_str).map(str::to_owned));
    let fields = chapter.as_object_mut().ok_or("章节数据不是对象")?;
    for key in ["resource", "url", "pages", "sources", "protocol", "files"] {
        fields.remove(key);
    }
    let kind = crate::sources::content_type(context.source.as_value());
    let (reference, format) = match kind {
        "image" => {
            let mut pages = Vec::new();
            for (index, url) in image_urls(body, base.as_deref())?.iter().enumerate() {
                let (reference, _) =
                    media(resources, executor, book, chapter_id, context, &cancellation, url, None).await?;
                pages.push(json!({"id":format!("p{}",index+1),"url":reference.as_str()}));
            }
            chapter["pages"] = json!(pages);
            return Ok(chapter);
        }
        "audio" | "video" => {
            let (candidates, default, headers) = candidates(body)?;
            let mut sources = Vec::new();
            for (index, (label, url)) in candidates.into_iter().enumerate() {
                match media(resources, executor, book, chapter_id, context, &cancellation,&url,headers.clone()).await {
                    Ok((reference,protocol)) => sources.push(json!({"src":reference.as_str(),"label":label,"format":protocol,"selected":index==default,"unavailableReason":null})),
                    Err(error) if kind == "audio" => return Err(error),
                    Err(_) => sources.push(json!({"src":null,"label":label,"format":null,"selected":false,"unavailableReason":"清晰度解析失败"})),
                }
            }
            let selected = sources
                .iter()
                .position(|s| s["selected"] == true && s["src"].is_string())
                .or_else(|| sources.iter().position(|s| s["src"].is_string()))
                .ok_or("没有可播放的媒体地址")?;
            for (index, source) in sources.iter_mut().enumerate() {
                source["selected"] = json!(index == selected);
            }
            let reference = ResourceRef::new(sources[selected]["src"].as_str().ok_or("媒体地址无效")?)
                .map_err(|e| e.to_string())?;
            chapter["protocol"] = sources[selected]["format"].clone();
            chapter["sources"] = if kind == "video" {
                json!(sources)
            } else {
                json!([])
            };
            (reference, kind)
        }
        _ if crate::support::looks_like_html(body) => {
            database
                .bind_book_type(book.to_owned(), "html".into())
                .map_err(|e| e.to_string())?
                .await
                .map_err(|e| e.to_string())?;
            let (_, document) = crate::book_files::html_document(
                "html",
                book,
                chapter_id,
                body,
                base.as_deref(),
                false,
            )
            .map_err(|e| e.to_string())?;
            let mut markup = document["markup"].as_str().ok_or("HTML 正文数据无效")?.to_owned();
            let images =
                regex::Regex::new(r#"(?i)<img\b[^>]*\bsrc\s*=\s*["']([^"']+)["']"#).map_err(|e| e.to_string())?;
            let urls = images
                .captures_iter(&markup)
                .map(|c| c[1].to_owned())
                .collect::<Vec<_>>();
            for escaped in urls {
                let url = quick_xml::escape::unescape(&escaped).map_err(|e| e.to_string())?;
                let (reference, _) =
                    media(resources, executor, book, chapter_id, context, &cancellation, &url, None).await?;
                markup = markup.replace(&escaped, reference.as_str());
            }
            let reference = crate::book_files::content_ref("html", book, chapter_id, &markup, true)
                .map_err(|e| e.to_string())?;
            resources
                .cache
                .save_content(&reference, &markup)
                .await
                .map_err(|e| e.to_string())?;
            (reference, "richText")
        }
        _ => (
            crate::book_files::text(resources, kind, book, chapter_id, body)
                .await
                .map_err(|e| e.to_string())?,
            "text",
        ),
    };
    chapter["resource"] = json!(crate::book_files::ChapterResourceDescriptor::for_chapter(
        &reference, format
    ));
    Ok(chapter)
}

type Candidates = (
    Vec<(String, String)>,
    usize,
    Option<HashMap<String, String>>,
);
fn candidates(body: &str) -> Result<Candidates, String> {
    let body = body.trim();
    if body.starts_with('{') {
        let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
        let items = value["resolutions"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 16)
            .ok_or("无效清晰度列表")?;
        let default = value["defaultIndex"].as_u64().unwrap_or(0) as usize;
        if default >= items.len() {
            return Err("默认清晰度超出范围".into());
        }
        let headers = value
            .get("headers")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| e.to_string())?;
        return Ok((
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    (
                        item["name"]
                            .as_str()
                            .map(|s| s.chars().take(80).collect())
                            .unwrap_or_else(|| format!("清晰度 {}", i + 1)),
                        item["url"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
            default,
            headers,
        ));
    }
    let items =
        if !body.starts_with("http://") && !body.starts_with("https://") && body.contains("::") {
            body.lines()
                .filter_map(|s| s.split_once("::"))
                .map(|(name, url)| {
                    (
                        name.trim().chars().take(80).collect(),
                        url.trim().to_owned(),
                    )
                })
                .collect::<Vec<_>>()
        } else {
            vec![(String::new(), body.to_owned())]
        };
    if items.is_empty() || items.len() > 16 {
        return Err("无效媒体列表".into());
    }
    Ok((items, 0, None))
}

/// 下载完整依赖后才替换章节引用；取消与失败不会覆盖数据库中的旧章节。
pub(crate) async fn cache_chapter_files(
    database: &crate::database::Database,
    resources: &crate::resources::Resources,
    book: &str,
    chapter_id: &str,
    mut chapter: Value,
) -> Result<Value, String> {
    let kind = crate::book_files::kind(database, book).map_err(|e| e.to_string())?;
    let mut markup = None;
    let mut roots = Vec::new();
    if kind == "image" {
        for page in chapter["pages"].as_array().into_iter().flatten() {
            if let Some(url) = page["url"].as_str() {
                roots.push(url.to_owned());
            }
        }
    } else if kind == "audio" || kind == "video" {
        if let Some(url) = chapter["resource"]["resourceId"].as_str() {
            roots.push(url.to_owned());
        }
        for source in chapter["sources"].as_array().into_iter().flatten() {
            if let Some(url) = source["src"].as_str() {
                roots.push(url.to_owned());
            }
        }
    } else if kind == "html" {
        if let Some(url) = chapter["resource"]["resourceId"].as_str() {
            let reference = ResourceRef::new(url).map_err(|e| e.to_string())?;
            let document = resources
                .cache
                .read_content(&reference)
                .await
                .map_err(|e| e.to_string())?;
            let body = document["markup"].as_str().unwrap_or_default().to_owned();
            let pattern = regex::Regex::new(r#"resource://proxy/[^\s"'<>]+"#).map_err(|e| e.to_string())?;
            roots.extend(pattern.find_iter(&body).map(|v| v.as_str().to_owned()));
            markup = Some(body);
        }
    }
    roots.retain(|url| url.starts_with("resource://proxy/"));
    if roots.is_empty() {
        return Ok(chapter);
    }
    let client = crate::http_client::raw_builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let mut queue = VecDeque::new();
    let mut names = HashMap::new();
    for root in &roots {
        let url = resources.url_for(&ResourceRef::new(root).map_err(|e| e.to_string())?);
        if names.insert(url.clone(), String::new()).is_none() {
            queue.push_back(url);
        }
    }
    let links = regex::Regex::new(r#"(?m)^[^#\r\n][^\r\n]*|URI="([^"]+)""#).map_err(|e| e.to_string())?;
    let mut playlists = Vec::new();
    let mut files = Vec::new();
    while let Some(url) = queue.pop_front() {
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?
            .error_for_status()
            .map_err(|e| e.without_url().to_string())?;
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let mut chunks = response.bytes_stream();
        use futures_util::StreamExt;
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            if bytes.len() + chunk.len() > 512 * 1024 * 1024 {
                return Err("离线资源超过 512 MiB".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Err("下载资源为空".into());
        }
        let playlist = bytes.starts_with(b"#EXTM3U");
        let extension = if playlist {
            "m3u8"
        } else if mime.contains("png") {
            "png"
        } else if mime.contains("webp") {
            "webp"
        } else if mime.starts_with("image/") {
            "jpg"
        } else if mime.contains("audio/mp4") {
            "m4a"
        } else if mime.contains("audio/aac") {
            "aac"
        } else if mime.starts_with("audio/") {
            "mp3"
        } else if mime.contains("video/mp4") {
            "mp4"
        } else {
            "ts"
        };
        let stable = url
            .strip_prefix(resources.base_url())
            .ok_or("离线地址超出本地服务")?;
        let name = format!(
            "{chapter_id}-{}.{}",
            crate::support::sha256_hex(stable.as_bytes()),
            extension
        );
        names.insert(url.clone(), name.clone());
        if playlist {
            let body = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            if !body.contains("#EXT-X-ENDLIST") && !body.contains("#EXT-X-STREAM-INF") {
                return Err("直播列表无法完整离线保存".into());
            }
            let base = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
            let mut dependencies = Vec::new();
            for capture in links.captures_iter(&body) {
                let raw = capture
                    .get(1)
                    .or_else(|| capture.get(0))
                    .map(|value| value.as_str()).unwrap_or_default()
                    .trim();
                if raw.is_empty() {
                    continue;
                }
                let child = base.join(raw).map_err(|e| e.to_string())?.to_string();
                if !child.starts_with(resources.base_url()) {
                    return Err("离线依赖超出本地服务".into());
                }
                if !names.contains_key(&child) {
                    if names.len() >= 8192 {
                        return Err("离线依赖过多".into());
                    }
                    names.insert(child.clone(), String::new());
                    queue.push_back(child.clone());
                }
                dependencies.push((raw.to_owned(), child));
            }
            playlists.push((name, body, dependencies));
        } else {
            resources
                .cache
                .save_asset(&kind, book, &name, &bytes)
                .await
                .map_err(|e| e.to_string())?;
            files.push(name);
        }
    }
    for (name, mut body, mut dependencies) in playlists {
        dependencies.sort_by_key(|(raw, _)| std::cmp::Reverse(raw.len()));
        for (raw, child) in dependencies {
            body = body.replace(&raw, &names[&child]);
        }
        resources
            .cache
            .save_asset(&kind, book, &name, body.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        files.push(name);
    }
    let mut replacements = HashMap::new();
    for root in roots {
        let url = resources.url_for(&ResourceRef::new(&root).map_err(|e| e.to_string())?);
        let reference = ResourceRef::new(format!("resource://cache/{kind}/{book}/{}", names[&url]))
            .map_err(|e| e.to_string())?;
        replacements.insert(root, reference.to_string());
    }
    fn replace(value: &mut Value, addresses: &HashMap<String, String>) {
        match value {
            Value::String(text) => {
                if let Some(next) = addresses.get(text) {
                    *text = next.clone();
                }
            }
            Value::Array(values) => {
                for value in values {
                    replace(value, addresses);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    replace(value, addresses);
                }
            }
            _ => {}
        }
    }
    replace(&mut chapter, &replacements);
    if let Some(mut body) = markup {
        for (old, new) in replacements {
            body = body.replace(&old, &new);
        }
        let reference = crate::book_files::content_ref(&kind, book, chapter_id, &body, true)
            .map_err(|e| e.to_string())?;
        resources
            .cache
            .save_content(&reference, &body)
            .await
            .map_err(|e| e.to_string())?;
        chapter["resource"] = json!(crate::book_files::ChapterResourceDescriptor::for_chapter(
            &reference, "richText"
        ));
    }
    chapter["files"] = json!(files);
    Ok(chapter)
}
