//! 对外数据与宿主端口；不负责调度，不引用平台实现。
mod models;
pub use models::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;

pub type Result<T> = std::result::Result<T, String>;
pub type EngineFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct SourceDocument(Value);
impl TryFrom<Value> for SourceDocument {
    type Error = String;
    fn try_from(value: Value) -> Result<Self> {
        if value.is_object() {
            Ok(Self(value))
        } else {
            Err("书源必须是对象".into())
        }
    }
}
impl TryFrom<&Value> for SourceDocument {
    type Error = String;
    fn try_from(value: &Value) -> Result<Self> {
        value.clone().try_into()
    }
}
impl<'de> Deserialize<'de> for SourceDocument {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        Map::deserialize(decoder).map(|map| Self(Value::Object(map)))
    }
}
impl SourceDocument {
    pub fn as_value(&self) -> &Value {
        &self.0
    }
}
impl BookRequest {
    pub fn new(source: SourceDocument, mut book: Book) -> Self {
        let kind = match source.0["bookSourceType"].as_u64() {
            Some(1) => 32,
            Some(2) => 64,
            Some(3) => 136,
            Some(4) => 2048,
            Some(5) => 4096,
            _ if crate::source::is_rss_source(&source.0) => 4096,
            _ => 8,
        };
        book.r#type =
            Some((book.r#type.unwrap_or_default() & !(8 | 32 | 64 | 128 | 2048 | 4096)) | kind);
        Self { source, book }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TextOr<T> {
    Text(String),
    Data(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Namespace {
    SourceCache,
    Cookies,
    FileCache,
    PersistentFileCache,
    ExploreKinds,
    BookVariables,
}
impl Namespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceCache => "source-cache",
            Self::Cookies => "cookies",
            Self::FileCache => "file-cache",
            Self::PersistentFileCache => "persistent-file-cache",
            Self::ExploreKinds => "explore-kinds",
            Self::BookVariables => "book-variables",
        }
    }
}
pub enum KeyFilter {
    Key(String),
    Prefixes(Vec<String>),
    Contains(String),
    All,
}
pub trait StorageHost: Send + Sync {
    fn get(&self, namespace: Namespace, key: &str) -> Result<Option<String>>;
    fn put(
        &self,
        namespace: Namespace,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<()>;
    fn remove(&self, namespace: Namespace, filter: KeyFilter) -> Result<Vec<String>>;
}

#[derive(Clone)]
pub struct EngineContext {
    pub data_dir: PathBuf,
    pub resource_dir: Option<PathBuf>,
    pub storage: Arc<dyn StorageHost>,
    pub cancellation: Option<CancellationToken>,
}

pub trait SourceExecutor: Send + Sync {
    fn search<'a>(
        &'a self,
        request: SearchRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, SearchPage>;
    fn explore_kinds<'a>(
        &'a self,
        request: SourceRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, ExploreKinds>;
    fn explore<'a>(
        &'a self,
        request: ExploreRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, SearchPage>;
    fn detail<'a>(
        &'a self,
        request: BookRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, Book>;
    fn chapters<'a>(
        &'a self,
        request: BookRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, ChapterList>;
    fn content<'a>(
        &'a self,
        request: ContentRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, ChapterContent>;
    fn resolve_media<'a>(
        &'a self,
        request: MediaRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, MediaLocation>;
    fn http_tts_audio<'a>(
        &'a self,
        request: TtsRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, AudioData>;
    fn login<'a>(
        &'a self,
        request: LoginRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, LoginState>;
}
