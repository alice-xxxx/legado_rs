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
        "comic"
    } else {
        media_type(source).unwrap_or("text")
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_reader_type_survives_search_and_details_without_overwriting_genres() {
        for (source_type, expected) in [(0, "text"), (1, "audio"), (2, "comic"), (4, "video")] {
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
