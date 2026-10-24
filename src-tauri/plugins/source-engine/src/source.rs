//! Source format compatibility shared by engine calls and application workflows.
use serde_json::Value;
pub fn is_legacy_rss_source(source: &Value) -> bool {
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
pub fn is_rss_source(source: &Value) -> bool {
    if is_legacy_rss_source(source) || is_standard_feed_source(source) {
        return true;
    }
    source.get("bookSourceType").and_then(Value::as_i64) == Some(5)
}

pub fn is_standard_feed_source(source: &Value) -> bool {
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
