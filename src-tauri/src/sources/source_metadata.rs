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
    source.get("bookSourceType").and_then(Value::as_i64).unwrap_or(0) == expected
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

pub(crate) fn book_content_type<'a>(book: &'a Value, source: &Value) -> &'a str {
    if source.is_object() {
        return content_type(source);
    }
    book.get("contentType").and_then(Value::as_str)
        .map(|kind| if kind == "html" { "text" } else { kind })
        .filter(|kind| matches!(*kind, "text" | "html" | "image" | "audio" | "video" | "pdf"))
        .or_else(|| book.get("mediaType").and_then(Value::as_str)
            .filter(|kind| matches!(*kind, "audio" | "video")))
        .unwrap_or("text")
}

/// Image rules may return a newline-separated or JSON URL list instead of HTML.
/// Only image sources interpret those values as image addresses.
pub(crate) fn normalize_chapter_content(source: &Value, content: &str) -> String {
    if !is_comic_source_metadata(source) || content.contains('<') {
        return content.to_owned();
    }
    let images = serde_json::from_str::<Vec<String>>(content.trim())
        .unwrap_or_else(|_| content.lines().map(str::to_owned).collect());
    images.iter().map(|image| {
        let image = image.trim();
        if image.is_empty() { return String::new(); }
        format!("<img src=\"{}\">", quick_xml::escape::escape(image))
    }).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_reader_type_survives_search_and_details_without_overwriting_genres() {
        for (source_type, expected) in [(0, "text"), (1, "audio"), (2, "image"), (4, "video")] {
            let source = crate::application::SourceRecord {
                id: "source-a".into(), name: "Source".into(), group: None, enabled: true,
                source: json!({"bookSourceType": source_type}),
            };
            let book = json!({"name":"Book", "kind":"悬疑"});
            let result = crate::application::project_search_result("result-a", &source, &book);
            assert_eq!(result.content_type, expected);
            let details = crate::book_metadata::project_book_metadata(&book, &source);
            assert_eq!(details.content_type.as_deref(), Some(expected));
            assert_eq!(details.kind.as_deref(), Some("悬疑"));
        }
    }
}
