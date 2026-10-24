//! 类型化业务数据；这里只声明列和关系，不保存对象副本。
use crate::source_wire::TextOr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct SourceDefinition {
    pub book_source_name: String,
    pub book_source_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_source_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_source_type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_url_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_explore: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_review: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub js_lib: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_cookie_jar: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_dangerous_api: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrent_rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_check_js: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_decode_js: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_source_comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable_comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respond_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explore_screen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explore_style: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legado_rs_user_agent_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<TextOr<BTreeMap<String, String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_ui: Option<TextOr<Vec<LoginItem>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explore_url: Option<TextOr<Vec<ExploreLink>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_search: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_explore: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_book_info: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_toc: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_review: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Source {
    pub id: String,
    pub definition: SourceDefinition,
}

pub(crate) use tauri_plugin_source_engine::{
    Book as EngineBook, Chapter as EngineChapter, TtsConfig as TtsDefinition, LoginItem, ExploreLink,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Page {
    pub id: String,
    #[serde(rename = "url", alias = "src")]
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Stream {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "url")]
    pub src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    #[serde(alias = "isDefault")]
    pub selected: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Chapter {
    pub id: String,
    pub title: String,
    #[serde(alias = "position")]
    pub index: usize,
    pub volume: bool,
    pub engine_data: EngineChapter,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    pub pages: Vec<Page>,
    pub sources: Vec<Stream>,
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<crate::book_files::ChapterResourceDescriptor>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Book {
    pub id: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "coverSrc")]
    pub cover: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<i64>,
    /// 最近一次详情和目录成功入库的时间，不要求目录发生变化。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_fetched_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "latestChapter")]
    pub latest_chapter_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineBook>,
    pub chapters: Vec<Chapter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ProgressInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    pub chapter_index: u64,
    pub offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ReadingUpdate {
    pub book_id: String,
    pub progress: ProgressInput,
    pub read: u64,
    pub chapter_changed: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct MediaRequest {
    pub id: String,
    pub book_id: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CategoryStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_flex_basis_percent: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct RssArticlePatch {
    pub article_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_read: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<String>,
}

// 缺省保留，null 清空，其余值替换。
#[derive(Debug, Default)]
pub(crate) enum Change {
    #[default]
    Keep,
    Clear,
    Set(String),
}
impl<'de> Deserialize<'de> for Change {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        Ok(Option::<String>::deserialize(input)?.map_or(Self::Clear, Self::Set))
    }
}
impl Change {
    pub(super) fn parts(self) -> (bool, Option<String>) {
        match self {
            Self::Keep => (false, None),
            Self::Clear => (true, None),
            Self::Set(value) => (true, Some(value)),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceInput {
    pub id: Option<String>,
    #[serde(deserialize_with = "crate::source_wire::decode")]
    pub definition: Option<SourceDefinition>,
    pub enabled: Option<bool>,
    pub group: Change,
    pub removed: Option<bool>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BookPatch {
    pub title: Change,
    pub author: Change,
    pub intro: Change,
    pub cover: Change,
    pub tags: Option<Vec<String>>,
    pub alternatives: Option<Vec<String>>,
    pub page_count: Option<i64>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OnlineBookInput {
    pub source_id: String,
    pub book_url: String,
    pub title: String,
    pub author: Option<String>,
    pub cover: Option<String>,
    pub latest: Option<String>,
    pub engine: Option<EngineBook>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TtsInput {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(deserialize_with = "crate::source_wire::required")]
    pub definition: TtsDefinition,
}

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
    pub books: Vec<ShelfBookPosition>,
}

impl Default for ShelfDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            sort: default_shelf_sort(),
            sort_order: default_shelf_sort_order(),
            groups: Vec::new(),
            books: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// 书架顺序和分组；书籍资料、目录和阅读位置使用各自的关系表。
pub struct ShelfBookPosition {
    pub id: String,
    #[serde(default)]
    pub groups: Vec<String>,
}

fn default_shelf_sort() -> String {
    "updatedAt".to_owned()
}

fn default_shelf_sort_order() -> String {
    "descending".to_owned()
}
