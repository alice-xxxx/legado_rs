//! Safe public projection of processed book detail values.
//!
//! Source rules and the complete engine book stay private. This module copies
//! only display metadata from a Rust-owned engine result and ordinary source
//! labels from `SourceRecord` into a browser-safe value.

use serde::Serialize;
use serde_json::Value;

use crate::{application::SourceRecord, resources::ResourceRef};

const MAX_TITLE_CHARS: usize = 512;
const MAX_AUTHOR_CHARS: usize = 512;
const MAX_INTRO_CHARS: usize = 12_000;
const MAX_KIND_CHARS: usize = 2_048;
const MAX_WORD_COUNT_CHARS: usize = 128;
const MAX_LATEST_CHAPTER_CHARS: usize = 512;
const MAX_SOURCE_ID_CHARS: usize = 256;
const MAX_SOURCE_NAME_CHARS: usize = 512;
const MAX_SOURCE_GROUP_CHARS: usize = 512;

/// A processed, display-only metadata resource. Canonical engine identity and
/// user display overrides intentionally live outside this projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessedBookMetadata {
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cover_src: Option<ResourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) word_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_chapter: Option<String>,
    pub(crate) source_id: String,
    pub(crate) source_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_group: Option<String>,
}

/// Project already processed `bookInfo` output and private source labels into
/// the small public metadata shape consumed by the WebView. The source's raw
/// definition, engine book URL, rules, headers, login scripts, and arbitrary
/// parser fields are never copied.
pub(crate) fn project_book_metadata(
    engine_book: &Value,
    source: &SourceRecord,
) -> ProcessedBookMetadata {
    ProcessedBookMetadata {
        title: text_at(engine_book, &["name", "title"], MAX_TITLE_CHARS, true)
            .unwrap_or_else(|| "Untitled".to_owned()),
        author: text_at(engine_book, &["author"], MAX_AUTHOR_CHARS, true),
        cover_src: text_at(
            engine_book,
            &["coverUrl", "cover", "coverSrc"],
            4_096,
            false,
        )
        .and_then(|cover| ResourceRef::new(cover).ok()),
        intro: text_at(
            engine_book,
            &["intro", "introduction"],
            MAX_INTRO_CHARS,
            true,
        ),
        kind: kind_at(engine_book),
        word_count: word_count_at(engine_book),
        latest_chapter: text_at(
            engine_book,
            &["lastChapter", "latestChapter", "latestChapterTitle"],
            MAX_LATEST_CHAPTER_CHARS,
            true,
        ),
        source_id: sanitize_text(&source.id, MAX_SOURCE_ID_CHARS, false).unwrap_or_default(),
        source_name: sanitize_text(&source.name, MAX_SOURCE_NAME_CHARS, true).unwrap_or_default(),
        source_group: source
            .group
            .as_deref()
            .and_then(|group| sanitize_text(group, MAX_SOURCE_GROUP_CHARS, true)),
    }
}

fn kind_at(book: &Value) -> Option<String> {
    let value = book.get("kind").or_else(|| book.get("category"))?;
    let mut kinds = Vec::new();
    match value {
        Value::String(kind) => {
            if let Some(kind) = sanitize_text(kind, MAX_KIND_CHARS, true) {
                kinds.push(kind);
            }
        }
        Value::Array(values) => {
            for value in values {
                if let Some(kind) = value
                    .as_str()
                    .and_then(|kind| sanitize_text(kind, MAX_KIND_CHARS, true))
                {
                    kinds.push(kind);
                }
            }
        }
        _ => return None,
    }
    let joined = kinds.join(", ");
    sanitize_text(&joined, MAX_KIND_CHARS, true)
}

fn word_count_at(book: &Value) -> Option<String> {
    ["wordCount", "word_count"].iter().find_map(|key| {
        let value = book.get(*key)?;
        let text = value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_number().map(ToString::to_string))?;
        sanitize_text(&text, MAX_WORD_COUNT_CHARS, true)
    })
}

fn text_at(value: &Value, keys: &[&str], max_chars: usize, strip_markup: bool) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .and_then(|text| sanitize_text(text, max_chars, strip_markup))
    })
}

/// Keep strings as inert plain text, trim them, discard control characters,
/// and cap by Unicode scalar count. Detail rules may return rich intro HTML;
/// strip tags here because this projection is intentionally text-only.
fn sanitize_text(text: &str, max_chars: usize, strip_markup: bool) -> Option<String> {
    let mut output = String::new();
    let mut in_tag = false;
    let mut output_chars = 0;
    for character in text.chars() {
        if strip_markup {
            if character == '<' {
                in_tag = true;
                continue;
            }
            if in_tag {
                if character == '>' {
                    in_tag = false;
                }
                continue;
            }
        }
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            continue;
        }
        if output_chars >= max_chars {
            break;
        }
        output.push(character);
        output_chars += 1;
    }
    let output = output.trim();
    (!output.is_empty()).then(|| output.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{project_book_metadata, MAX_INTRO_CHARS};
    use crate::application::SourceRecord;

    fn source() -> SourceRecord {
        SourceRecord {
            id: "source-novel-1".to_owned(),
            name: "小说来源".to_owned(),
            group: Some("中文站点".to_owned()),
            enabled: true,
            source: json!({
                "bookSourceUrl": "https://source.example",
                "ruleBookInfo": { "name": "private selector" },
                "loginUrl": "https://source.example/login",
                "headers": { "Authorization": "private token" },
            }),
        }
    }

    #[test]
    fn projects_only_processed_display_fields_and_source_labels() {
        let engine_book = json!({
            "name": "仙途",
            "author": "作者甲",
            "coverUrl": "https://img.example/cover.jpg",
            "intro": "<p>第一行</p><script>unsafe()</script><p>第二行</p>",
            "kind": "玄幻,仙侠",
            "wordCount": "12.3万字",
            "lastChapter": "第一百章",
            "bookUrl": "https://books.example/private-id",
            "sourceId": "source-injected-by-engine-payload",
            "sourceName": "injected source name",
            "displayOverrides": { "title": "用户改名", "author": "用户作者名" },
            "ruleBookInfo": { "name": "secret rule" },
            "headers": { "Authorization": "secret header" },
            "loginScript": "secret script",
            "unexpectedPrivateData": "do not project",
        });

        let projected = project_book_metadata(&engine_book, &source());
        let public = serde_json::to_value(projected).unwrap();
        assert_eq!(public["title"], "仙途");
        assert_eq!(public["author"], "作者甲");
        assert_eq!(public["coverSrc"], "https://img.example/cover.jpg");
        assert_eq!(public["intro"], "第一行unsafe()第二行");
        assert_eq!(public["kind"], "玄幻,仙侠");
        assert_eq!(public["wordCount"], "12.3万字");
        assert_eq!(public["latestChapter"], "第一百章");
        assert_eq!(public["sourceId"], "source-novel-1");
        assert_eq!(public["sourceName"], "小说来源");
        assert_eq!(public["sourceGroup"], "中文站点");
        for private in [
            "bookUrl",
            "displayOverrides",
            "source",
            "ruleBookInfo",
            "headers",
            "loginScript",
            "unexpectedPrivateData",
        ] {
            assert!(public.get(private).is_none(), "unexpected {private} field");
        }
        let encoded = public.to_string();
        for private_value in [
            "private-id",
            "用户改名",
            "用户作者名",
            "source-injected-by-engine-payload",
            "injected source name",
            "private selector",
            "secret rule",
            "secret header",
            "secret script",
        ] {
            assert!(!encoded.contains(private_value));
        }
    }

    #[test]
    fn intro_is_plain_text_bounded_and_control_characters_are_removed() {
        let long_intro = format!("<b>{}</b>\u{0001}", "中".repeat(MAX_INTRO_CHARS + 50));
        let projected = project_book_metadata(
            &json!({ "name": "  <em>标题</em>\u{0007}", "intro": long_intro }),
            &source(),
        );
        assert_eq!(projected.title, "标题");
        let intro = projected.intro.unwrap();
        assert_eq!(intro.chars().count(), MAX_INTRO_CHARS);
        assert!(intro.chars().all(|character| !character.is_control()));
        assert!(!intro.contains('<'));
    }

    #[test]
    fn cover_accepts_stable_and_http_resources_but_drops_unsafe_urls() {
        for cover in [
            "resource://books/book-1/assets/cover.png",
            "https://img.example/cover.jpg",
            "http://img.example/cover.jpg",
        ] {
            let projected = project_book_metadata(&json!({ "coverSrc": cover }), &source());
            assert_eq!(projected.cover_src.unwrap().as_str(), cover);
        }
        for cover in [
            "javascript:alert(1)",
            "data:image/png;base64,AA==",
            "https://user:password@img.example/cover.jpg",
            "file:///tmp/cover.jpg",
        ] {
            let projected = project_book_metadata(&json!({ "coverSrc": cover }), &source());
            assert!(projected.cover_src.is_none(), "accepted {cover}");
        }
    }

    #[test]
    fn tolerates_missing_or_malformed_optional_engine_fields() {
        let projected = project_book_metadata(
            &json!({
                "title": "   ",
                "author": 7,
                "kind": ["奇幻", 9, "", "冒险"],
                "word_count": 12345,
                "cover": "invalid",
            }),
            &source(),
        );
        assert_eq!(projected.title, "Untitled");
        assert!(projected.author.is_none());
        assert_eq!(projected.kind.as_deref(), Some("奇幻, 冒险"));
        assert_eq!(projected.word_count.as_deref(), Some("12345"));
        assert!(projected.cover_src.is_none());
        assert!(projected.latest_chapter.is_none());
    }

    #[test]
    fn ignores_non_object_engine_payloads_without_leaking_source_definition() {
        let projected = project_book_metadata(&Value::Null, &source());
        let public = serde_json::to_value(projected).unwrap();
        assert_eq!(public["title"], "Untitled");
        assert_eq!(public["sourceId"], "source-novel-1");
        assert!(public.get("bookUrl").is_none());
        assert!(public.get("source").is_none());
    }
}
