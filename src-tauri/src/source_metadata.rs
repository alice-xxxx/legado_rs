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

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::is_rss_source_metadata;

    #[test]
    fn classifies_a_regular_book_source_as_non_rss() {
        let source = json!({
            "bookSourceName": "Novel source",
            "bookSourceUrl": "https://books.example/source",
            "bookSourceType": 0,
            "searchUrl": "https://books.example/search?q={{key}}",
            "ruleSearch": { "bookList": ".book" },
            "ruleToc": { "chapterList": ".chapter" },
        });

        assert!(!is_rss_source_metadata(&source));
    }

    #[test]
    fn classifies_existing_rss_book_source_type() {
        let source = json!({
            "bookSourceName": "RSS source",
            "bookSourceUrl": "https://feeds.example/source",
            "bookSourceType": 5,
            "ruleArticles": { "articleList": ".article" },
        });

        assert!(is_rss_source_metadata(&source));
    }

    #[test]
    fn classifies_legacy_rss_definition_without_exposing_its_rules() {
        let source = json!({
            "sourceName": "Legacy RSS",
            "sourceUrl": "https://feeds.example/legacy.xml",
            "ruleTitle": "title",
            "ruleLink": "link",
            "ruleArticles": { "title": "secret selector" },
        });

        assert!(is_rss_source_metadata(&source));
    }

    #[test]
    fn classifies_plain_feed_url_as_rss() {
        let source = json!({
            "sourceName": "Atom feed",
            "feedUrl": "https://feeds.example/updates.atom",
        });

        assert!(is_rss_source_metadata(&source));
    }

    #[test]
    fn does_not_mistake_book_rules_or_ambiguous_values_for_feed_sources() {
        let book_rules_on_source_url = json!({
            "sourceUrl": "https://books.example/source",
            "ruleSearch": { "bookList": ".book" },
        });
        let feed_url_with_book_rules = json!({
            "url": "https://books.example/source",
            "ruleContent": { "content": ".text" },
        });
        let wrong_type = json!({
            "bookSourceUrl": "https://other.example/source",
            "bookSourceType": 2,
        });

        assert!(!is_rss_source_metadata(&book_rules_on_source_url));
        assert!(!is_rss_source_metadata(&feed_url_with_book_rules));
        assert!(!is_rss_source_metadata(&wrong_type));
        assert!(!is_rss_source_metadata(&json!("not a source object")));
        assert!(!is_rss_source_metadata(&Value::Null));
    }
}
