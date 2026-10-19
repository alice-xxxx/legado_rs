//! 过滤书源私有定义并生成前端可消费的公开元数据。
//! Public metadata classification for private source records.
//!
//! Reader types come from the source definition, independently of book genres.
//! Source rules remain private.

use serde_json::Value;

/// Whether the private source definition should be offered in the RSS UI.
///
/// Keep this aligned with the source shapes accepted by `rss.rs`: existing
/// RSS BookSource records, legacy RSS definitions, and plain RSS/Atom feed
/// URLs. Other `bookSourceType` values are deliberately not interpreted here.
pub(crate) fn is_rss_source_metadata(source: &Value) -> bool {
    crate::rss::is_rss_source(source)
}

/// Whether a private BookSource record uses the legacy audio chapter model.
/// Only this exact source type is projected as an audio reader; content that
/// happens to contain media markup remains an ordinary chapter.
pub(crate) fn is_audio_source_metadata(source: &Value) -> bool {
    source.get("bookSourceType").and_then(Value::as_i64) == Some(1)
}

/// Whether a private BookSource record uses the legacy video chapter model.
pub(crate) fn is_video_source_metadata(source: &Value) -> bool {
    source.get("bookSourceType").and_then(Value::as_i64) == Some(4)
}

pub(crate) fn is_comic_source_metadata(source: &Value) -> bool {
    source.get("bookSourceType").and_then(Value::as_i64) == Some(2)
}

pub(crate) fn content_type(source: &Value) -> &'static str {
    if is_comic_source_metadata(source) {
        "image"
    } else {
        media_type(source).unwrap_or("text")
    }
}

pub(crate) fn matches_search_type(source: &Value, requested: &str) -> bool {
    let expected = match requested {
        "text" => 0,
        "audio" => 1,
        "image" => 2,
        "video" => 4,
        _ => return false,
    };
    source
        .get("bookSourceType")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        == expected
        && !is_rss_source_metadata(source)
}

/// The streamed-media reader mode supported for a private book source.
pub(crate) fn media_type(source: &Value) -> Option<&'static str> {
    if is_audio_source_metadata(source) {
        Some("audio")
    } else if is_video_source_metadata(source) {
        Some("video")
    } else {
        None
    }
}

/// Book.type is a bitmask, unlike the source's single-valued bookSourceType.
pub(crate) fn engine_book_type(source: &Value) -> u64 {
    match source.get("bookSourceType").and_then(Value::as_u64) {
        Some(1) => 32,
        Some(2) => 64,
        Some(3) => 8 | 128,
        Some(4) => 2048,
        Some(5) => 4096,
        _ if is_rss_source_metadata(source) => 4096,
        _ => 8,
    }
}

/// Image rules may return a newline-separated or JSON URL list instead of HTML.
/// Only image sources interpret those values as image addresses.
pub(crate) fn normalize_chapter_content(source: &Value, content: &str) -> String {
    if !is_comic_source_metadata(source) || content.contains('<') {
        return content.to_owned();
    }
    let images = serde_json::from_str::<Vec<String>>(content.trim())
        .unwrap_or_else(|_| content.lines().map(str::to_owned).collect());
    images
        .iter()
        .map(|image| {
            let image = image.trim();
            if image.is_empty() {
                return String::new();
            }
            format!("<img src=\"{}\">", quick_xml::escape::escape(image))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
