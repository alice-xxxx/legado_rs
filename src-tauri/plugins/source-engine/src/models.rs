use crate::wire::TextOr;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoginItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExploreLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TtsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrent_rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_check_js: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub js_lib: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_cookie_jar: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_dangerous_api: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<TextOr<BTreeMap<String, String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_ui: Option<TextOr<Vec<LoginItem>>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Book {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toc_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub r#type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_chapter_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_chapter_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<TextOr<BTreeMap<String, String>>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Chapter {
    pub url: String,
    pub title: String,
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_url: Option<String>,
    pub is_volume: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_vip: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_pay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_fragment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_fragment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<TextOr<BTreeMap<String, String>>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct SourceDocument(Value);

impl TryFrom<Value> for SourceDocument {
    type Error = String;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if value.is_object() {
            Ok(Self(value))
        } else {
            Err("Source definition must be an object".into())
        }
    }
}
impl TryFrom<&Value> for SourceDocument {
    type Error = String;
    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        value.clone().try_into()
    }
}
impl<'de> Deserialize<'de> for SourceDocument {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        Map::deserialize(decoder).map(|value| Self(Value::Object(value)))
    }
}
impl SourceDocument {
    pub fn as_value(&self) -> &Value {
        &self.0
    }
    fn book_flags(&self) -> u64 {
        match self.0["bookSourceType"].as_u64() {
            Some(1) => 32,
            Some(2) => 64,
            Some(3) => 8 | 128,
            Some(4) => 2048,
            Some(5) => 4096,
            _ if crate::source::is_rss_source(&self.0) => 4096,
            _ => 8,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRequest {
    pub source: SourceDocument,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub source: SourceDocument,
    pub keyword: String,
    pub page: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreRequest {
    pub source: SourceDocument,
    pub url: String,
    pub page: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookRequest {
    pub source: SourceDocument,
    pub book: Book,
}

impl BookRequest {
    pub fn new(source: SourceDocument, mut book: Book) -> Self {
        let flags = book
            .r#type
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or_default()
            & !(8 | 32 | 64 | 128 | 2048 | 4096);
        book.r#type = Some((flags | source.book_flags()) as i64);
        Self { source, book }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRequest {
    pub source: SourceDocument,
    pub book: Book,
    pub chapter: Chapter,
    pub next_chapter_url: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRequest {
    pub source: SourceDocument,
    pub book: Book,
    pub chapter: Chapter,
    pub url: String,
    pub headers: HashMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsRequest {
    pub config: TtsConfig,
    pub text: String,
    pub speech_rate: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LoginAction {
    Open,
    Clear,
    Submit {
        fields: HashMap<String, String>,
        button: Option<usize>,
    },
    ImportCookies {
        url: String,
        cookies: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub source: SourceDocument,
    pub action: LoginAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub books: Vec<Book>,
    pub has_next_page: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExploreCategory {
    pub title: Option<String>,
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExploreKinds {
    pub categories: Vec<ExploreCategory>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChapterList {
    pub chapters: Vec<Chapter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChapterContent {
    pub content: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaLocation {
    pub url: String,
    pub headers: HashMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioData {
    pub bytes_base64: String,
    pub content_type: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoginField {
    pub name: String,
    pub label: String,
    pub r#type: String,
    pub password: bool,
    pub choices: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginButton {
    pub id: usize,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginLayout {
    pub kind: String,
    pub name: Option<String>,
    pub id: Option<usize>,
    pub label: Option<String>,
    #[serde(default = "single_cell")]
    pub cols: i32,
    #[serde(default = "single_cell")]
    pub rows: i32,
}
fn single_cell() -> i32 {
    1
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoginForm {
    pub mode: String,
    pub can_login: bool,
    pub fields: Vec<LoginField>,
    pub actions: Vec<LoginButton>,
    pub layout: Vec<LoginLayout>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginWeb {
    pub url: String,
    pub cookie_header: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoginState {
    pub has_login_state: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<LoginForm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<LoginWeb>,
    pub messages: Vec<String>,
    pub input_updates: HashMap<String, String>,
    pub refresh_targets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy_text: Option<String>,
}

/// Private native browser login capabilities. These payloads only cross the Rust/native plugin
/// boundary; the application WebView receives only the opaque session ID.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebStart {
    pub source_id: String,
    pub session_id: String,
    pub source_revision: u64,
    pub restore_epoch: u64,
    pub login_url: String,
    pub cookie_header: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebSession {
    pub source_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebStarted {
    pub started: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebCapture {
    pub login_url: String,
    pub source_revision: u64,
    pub restore_epoch: u64,
    pub cookie_header: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebCancelled {
    pub cancelled: bool,
}
