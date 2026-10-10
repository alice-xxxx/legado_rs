//! 定义 Rust 与资源 JSON 共用的持久化数据结构。
//! Public JSON models for the resource-oriented application contract.
//!
//! These models describe processed, frontend-consumable resources. Book source
//! definitions and parser rules belong to the source engine and are never
//! represented by these types.

use serde::{Deserialize, Serialize};

use crate::resources::ResourceRef;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfDocument {
    pub schema_version: u32,
    #[serde(default = "default_shelf_sort")]
    pub sort: String,
    #[serde(default = "default_shelf_sort_order")]
    pub sort_order: String,
    #[serde(default)]
    pub groups: Vec<String>,
    pub books: Vec<BookSummary>,
}

impl Default for ShelfDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            sort: default_shelf_sort(),
            sort_order: default_shelf_sort_order(),
            groups: Vec::new(),
            books: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSummary {
    pub id: String,
    pub title: String,
    /// Optional typed display classification, projected from the stored book.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub media_type: Option<BookMediaType>,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cover_src: Option<ResourceRef>,
    #[serde(default)]
    pub chapter_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub latest_chapter: Option<String>,
    #[serde(default)]
    pub progress: ProgressSummary,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSummary {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chapter_id: Option<String>,
    #[serde(default)]
    pub chapter_index: usize,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDocument {
    pub schema_version: u32,
    #[serde(default = "default_source_http_timeout_seconds")]
    pub source_http_timeout_seconds: u64,
    pub reader: ReaderDefaults,
}

impl Default for SettingsDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            source_http_timeout_seconds: default_source_http_timeout_seconds(),
            reader: ReaderDefaults::default(),
        }
    }
}

fn default_source_http_timeout_seconds() -> u64 {
    15
}

/// Reader display defaults. Cached chapter resources remain content data;
/// the WebView applies these settings when building the current display document.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderDefaults {
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size_px")]
    pub font_size_px: f32,
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    #[serde(default = "default_text_color")]
    pub text_color: String,
    #[serde(default = "default_background_color")]
    pub background_color: String,
    /// Stable reader background image reference. It is materialized to a
    /// process-local URL only when the settings JSON is served to the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image_src: Option<ResourceRef>,
    #[serde(default = "default_text_align")]
    pub text_align: String,
    #[serde(default)]
    pub theme: ReaderTheme,
    #[serde(default = "default_preload_count")]
    pub preload_count: usize,
    /// Display-only replacements run by the WebView and never mutate cached chapter data.
    #[serde(default)]
    pub replacements: Vec<serde_json::Value>,
}

/// Themes supported by the Vue reader.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReaderTheme {
    #[default]
    Paper,
    Sepia,
    Dark,
    System,
}

impl Default for ReaderDefaults {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size_px: default_font_size_px(),
            line_height: default_line_height(),
            text_color: default_text_color(),
            background_color: default_background_color(),
            background_image_src: None,
            text_align: default_text_align(),
            theme: ReaderTheme::Paper,
            preload_count: default_preload_count(),
            replacements: Vec::new(),
        }
    }
}

fn default_font_family() -> String {
    "serif".to_owned()
}

fn default_font_size_px() -> f32 {
    19.0
}

fn default_line_height() -> f32 {
    1.8
}

fn default_text_color() -> String {
    "#3f3b34".to_owned()
}

fn default_background_color() -> String {
    "#f7f3e9".to_owned()
}

fn default_text_align() -> String {
    "justify".to_owned()
}

fn default_shelf_sort() -> String {
    "updatedAt".to_owned()
}

fn default_shelf_sort_order() -> String {
    "descending".to_owned()
}

fn default_preload_count() -> usize {
    5
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDocument {
    pub schema_version: u32,
    pub book_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chapter_id: Option<String>,
    #[serde(default)]
    pub chapter_index: usize,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BookMediaType {
    Audio,
    Video,
}

impl BookMediaType {
    pub(crate) fn from_source_value(value: &str) -> Option<Self> {
        match value {
            "audio" => Some(Self::Audio),
            "video" => Some(Self::Video),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDocument {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    /// Rust-owned source capability projection. Local imports have no private
    /// engine source and therefore cannot be switched to another source.
    #[serde(default)]
    pub can_change_source: bool,
    #[serde(default)]
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cover_src: Option<ResourceRef>,
    /// Rust-projected, plain-text description from processed book details.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub kind: Option<String>,
    /// Typed playback mode for content whose ordered units are audio or video.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub media_type: Option<BookMediaType>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub word_count: Option<String>,
    /// Public source labels only; raw source definitions stay private.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_group: Option<String>,
    /// Rust-owned processed base values and user display-only overrides.
    /// Neither field contains a source definition or KMP engine book.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub display_base: Option<BookDisplayBase>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub display_overrides: Option<BookDisplayOverrides>,
    #[serde(default)]
    pub chapter_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub latest_chapter: Option<String>,
    #[serde(default)]
    pub progress: ProgressSummary,
    #[serde(default)]
    pub chapters: Vec<ChapterDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookDisplayBase {
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cover_src: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookDisplayOverrides {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub intro: Option<String>,
    /// An empty string explicitly hides the default cover; `None` falls back
    /// to the current processed base cover.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cover_src: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterResourceDescriptor {
    /// Opaque stable identity. The WebView must not parse this value.
    pub resource_id: ResourceRef,
    /// Stable persisted ref; the resource server materializes this field into
    /// the current process-local capability URL when serving book.json.
    pub src: ResourceRef,
    pub content_type: String,
    pub format: String,
    /// Semantic chapter content kind supplied by the chapter resource writer.
    pub content_format: String,
}

impl ChapterResourceDescriptor {
    pub fn for_chapter(reference: &ResourceRef, content_format: &str) -> Self {
        Self {
            resource_id: reference.clone(),
            src: reference.clone(),
            content_type: "application/json; charset=utf-8".to_owned(),
            format: "json".to_owned(),
            content_format: content_format.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDescriptor {
    pub id: String,
    pub title: String,
    pub index: usize,
    /// None means that this catalog chapter has not been prepared.
    /// Its descriptor owns the single authoritative resource location and format.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resource: Option<ChapterResourceDescriptor>,
}

/// Search result resource shape. The application layer may put richer
/// processed result objects in `results`; parser inputs never go to the UI.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultsDocument {
    pub schema_version: u32,
    pub keyword: String,
    #[serde(default = "default_search_page")]
    pub page: u32,
    #[serde(default)]
    pub results: Vec<serde_json::Value>,
    #[serde(default)]
    pub errors: Vec<SearchError>,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchError {
    #[serde(default)]
    pub source_id: Option<String>,
    pub source_name: String,
    pub message: String,
}

fn default_search_page() -> u32 {
    1
}
