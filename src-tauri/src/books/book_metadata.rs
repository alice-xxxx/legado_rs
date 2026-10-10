//! 合并和更新书源返回的书籍元数据。
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
