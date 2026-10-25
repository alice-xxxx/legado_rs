//! 书源协议的分类规则，不检查正文或脚本内容。
use serde_json::Value;
pub fn is_legacy_rss_source(source: &Value) -> bool {
    let Some(object) = source.as_object() else {
        return false;
    };
    !object.contains_key("bookSourceUrl")
        && object.contains_key("sourceUrl")
        && ["ruleArticles", "sortUrl", "ruleTitle", "ruleLink"]
            .iter()
            .any(|key| object.contains_key(*key))
}
pub fn is_rss_source(source: &Value) -> bool {
    source["bookSourceType"].as_i64() == Some(5)
        || is_legacy_rss_source(source)
        || is_standard_feed_source(source)
}
pub fn is_standard_feed_source(source: &Value) -> bool {
    let Some(object) = source.as_object() else {
        return false;
    };
    if object.contains_key("bookSourceUrl") || is_legacy_rss_source(source) {
        return false;
    }
    ["feedUrl", "sourceUrl", "url"]
        .iter()
        .any(|key| object.get(*key).and_then(Value::as_str).is_some())
        && ![
            "searchUrl",
            "ruleSearch",
            "ruleBookInfo",
            "ruleToc",
            "ruleContent",
        ]
        .iter()
        .any(|key| object.contains_key(*key))
}
