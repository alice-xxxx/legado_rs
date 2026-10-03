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
    pub reader: ReaderDefaults,
}

impl Default for SettingsDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            reader: ReaderDefaults::default(),
        }
    }
}

/// Initial values written into chapter HTML. User settings are applied by the
/// WebView at display time and do not rewrite the cached chapter.
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
    #[serde(default = "default_text_align")]
    pub text_align: String,
    #[serde(default)]
    pub theme: ReaderTheme,
    #[serde(default = "default_preload_count")]
    pub preload_count: usize,
    /// Display-only replacements run by the WebView and never change cached HTML.
    #[serde(default)]
    pub replacements: Vec<serde_json::Value>,
}

/// Themes supported by the Vue reader. `light` is accepted only as a legacy
/// alias; serializing it always writes the canonical `system` value.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReaderTheme {
    #[default]
    Paper,
    Sepia,
    Dark,
    #[serde(alias = "light")]
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

#[cfg(test)]
mod tests {
    use super::{ReaderDefaults, ReaderTheme, SettingsDocument};

    #[test]
    fn reader_defaults_match_vue_and_round_trip_canonical_settings() {
        let settings = SettingsDocument::default();
        let json = serde_json::to_value(&settings).expect("settings serialize");
        assert_eq!(json["reader"]["fontSizePx"], 19.0);
        assert_eq!(json["reader"]["fontFamily"], "serif");
        assert_eq!(json["reader"]["textColor"], "#3f3b34");
        assert_eq!(json["reader"]["backgroundColor"], "#f7f3e9");
        assert!((json["reader"]["lineHeight"].as_f64().unwrap() - 1.8).abs() < 0.00001);
        assert_eq!(json["reader"]["textAlign"], "justify");
        assert_eq!(json["reader"]["theme"], "paper");
        assert_eq!(json["reader"]["preloadCount"], 5);
        assert_eq!(json["reader"]["replacements"], serde_json::json!([]));

        let decoded: SettingsDocument = serde_json::from_value(json).expect("settings decode");
        assert_eq!(decoded.reader.theme, ReaderTheme::Paper);
        assert_eq!(decoded.reader.font_size_px, 19.0);
        let missing_theme: ReaderDefaults =
            serde_json::from_value(serde_json::json!({})).expect("reader defaults decode");
        assert_eq!(missing_theme.theme, ReaderTheme::Paper);
    }

    #[test]
    fn legacy_light_theme_reads_as_system_and_serializes_canonically() {
        let reader: ReaderDefaults = serde_json::from_value(serde_json::json!({"theme":"light"}))
            .expect("legacy theme remains readable");
        assert_eq!(reader.theme, ReaderTheme::System);
        assert_eq!(serde_json::to_value(reader).unwrap()["theme"], "system");
    }
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDocument {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cover_src: Option<ResourceRef>,
    #[serde(default)]
    pub chapter_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub latest_chapter: Option<String>,
    #[serde(default)]
    pub progress: ProgressSummary,
    #[serde(default)]
    pub chapters: Vec<ChapterDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDescriptor {
    pub id: String,
    pub title: String,
    pub index: usize,
    /// Stable local resource ref or direct HTTP(S) URL. `None` means the
    /// catalog entry is known but its local HTML has not been prepared yet.
    #[serde(default)]
    pub src: Option<ResourceRef>,
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
