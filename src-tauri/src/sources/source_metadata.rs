//! 过滤书源私有定义并生成前端可消费的公开元数据。
//! Public metadata classification for private source records.
//!
//! This module intentionally exposes only the RSS distinction needed by the
//! UI. It delegates format recognition to the existing RSS predicates and
//! never projects, parses, or returns source rules.

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
