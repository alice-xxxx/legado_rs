//! Rust application services and Tauri commands.
//!
//! 服务实现按书籍、书源、阅读状态和资源处理等职责拆分到子模块中。
//!
//! Rust owns source definition persistence and validation. The management UI
//! reads business tables through a read-only SQLite pool. File resources serve
//! chapter bodies and media; only KMP executes source rules.

use std::io::{Cursor, Read};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, atomic::AtomicU64},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use std::time::Instant;

use base64::Engine as _;
use image::{ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::{
    models::{
        CURRENT_SCHEMA_VERSION, ChapterDescriptor, ProgressSummary,
        ReaderDefaults, ReaderTheme, ShelfDocument,
    },
    resources::{
        ChapterVideoSource, ResourceRef, ResourceServer, ResourceStore, ResourceStoreWriterGuard,
    },
    source_engine::SourceEngineRequest,
};

// 备份与书籍域服务。
mod backup_service;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use backup_service::restore_error_response;
mod book_catalog;
mod more_information;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use more_information::*;
mod book_display;

// 本地文件、章节资源和通用数据处理。
mod file_import;
use file_import::PendingPdfImportRegistry;
pub use file_import::{
    PendingExternalFileDescriptor, PendingExternalFileRegistry, PickerFile,
    schedule_pending_external_file_cleanup,
};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use file_import::{
    copy_stream_limited, infer_picker_extension, picker_display_name, safe_picker_file_name,
    PendingExternalFileImportClaim, PickerFileCleanupOwner,
};

// 书源执行、搜索、切换与状态。
mod main_text;
mod chapter_resources;
mod general;
mod reader_state;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod source_executor;
mod source_helpers;
mod source_management;
pub(crate) use source_management::{validate_source_definition, validate_source_records};
pub(crate) use general::stable_hash;
#[path = "../search/search.rs"]
mod source_search;
pub use source_search::*;
mod state;

// 后台任务、TTS 和前端命令适配。
#[cfg(test)]
mod performance_tests;
mod task_history;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod tauri_commands;
mod tts;

// 子模块通过父模块门面共享必要的业务辅助函数。
use book_display::*;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use main_text::*;
use general::*;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use source_executor::TauriSourceExecutor;
use source_helpers::*;
pub(crate) use source_helpers::metadata as source_metadata_rows;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use tauri_commands::*;

pub type EngineFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

/// The boundary for executing a source rule. A source definition is only ever
/// supplied to this executor by Rust, never by the WebView.
pub trait SourceExecutor: Send + Sync {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a>;
}

const PENDING_PDF_IMPORT_TTL: Duration = Duration::from_secs(5 * 60);
const PENDING_EXTERNAL_FILE_IMPORT_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING_EXTERNAL_FILE_IMPORTS: usize = 8;
const MAX_SOURCE_IMPORT_BODY_BYTES: usize = 32 * 1024 * 1024;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
const MAX_REPLACEMENT_RULE_IMPORT_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_DICTIONARY_WORD_CHARS: usize = 128;
const MAX_DICTIONARY_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HTTP_TTS_TEXT_CHARS: usize = 20_000;
const MAX_HTTP_TTS_AUDIO_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTTP_TTS_AUDIO_BASE64_BYTES: usize = ((MAX_HTTP_TTS_AUDIO_BYTES + 2) / 3) * 4;
const MAX_VIDEO_MEDIA_VARIANTS: usize = 16;
const VIDEO_ADDRESS_UNAVAILABLE: &str = "媒体地址无效";
const VIDEO_RESOLUTION_UNAVAILABLE: &str = "清晰度解析失败";
const MEDIA_FORMAT_HLS: &str = "hls";
const MEDIA_FORMAT_DIRECT: &str = "direct";
const MAX_PICKER_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BOOK_COVER_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BOOK_COVER_PIXELS: u64 = 16_000_000;
const MAX_BOOK_COVER_DIMENSION: u32 = 16_384;
const READER_BACKGROUND_BOOK_ID: &str = "reader-theme-backgrounds";
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
const MAX_PICKER_SOURCE_JSON_BYTES: u64 = 32 * 1024 * 1024;

/// Resource-oriented service for source processing and resource delivery.
#[derive(Clone)]
pub struct ApplicationService {
    root: PathBuf,
    _process_lock: Arc<crate::resource_transactions::AppDataProcessLock>,
    store: Arc<ResourceStore>,
    server: Arc<ResourceServer>,
    executor: Arc<dyn SourceExecutor>,
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    app: Option<tauri::AppHandle>,
    book_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    sources_lock: Arc<tokio::sync::Mutex<()>>,
    source_catalog: Arc<tokio::sync::Mutex<Option<Arc<source_management::SourceCatalog>>>>,
    source_revisions: Arc<tokio::sync::Mutex<HashMap<String, u64>>>,
    searches: Arc<tokio::sync::Mutex<source_search::SearchRegistry>>,
    search_slots: Arc<tokio::sync::Semaphore>,
    information_cancel: tokio::sync::broadcast::Sender<Option<(String, String)>>,
    text_cancel: tokio::sync::broadcast::Sender<Option<(String, String)>>,
    admission_gate: Arc<tokio::sync::RwLock<()>>,
    operation_gate: Arc<tokio::sync::RwLock<()>>,
    webdav_backup_upload_gate: Arc<tokio::sync::Mutex<()>>,
    webdav_backup_config_gate: Arc<tokio::sync::Mutex<()>>,
    restore_barrier: Arc<tokio::sync::watch::Sender<bool>>,
    restore_serial: Arc<tokio::sync::Mutex<()>>,
    restore_epoch: Arc<AtomicU64>,
    pending_pdf_imports: PendingPdfImportRegistry,
}

struct RestoreAdmissionGuard {
    barrier: Arc<tokio::sync::watch::Sender<bool>>,
    release_on_drop: bool,
}

pub struct OperationReadGuard {
    _admission: tokio::sync::OwnedRwLockReadGuard<()>,
    _operation: tokio::sync::OwnedRwLockReadGuard<()>,
}

impl Drop for RestoreAdmissionGuard {
    fn drop(&mut self) {
        if self.release_on_drop {
            self.barrier.send_replace(false);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) source: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetadata {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    pub enabled: bool,
    pub is_rss: bool,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub capabilities: SourceCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent_override: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCapabilities {
    pub search: bool,
    pub detail: bool,
    pub toc: bool,
    pub content: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchResultSource {
    /// 保存可换来源时保留原始书籍上下文，避免详情规则丢失自定义字段。
    pub(crate) engine: Value,
    pub(crate) result_id: String,
    pub(crate) source_id: String,
    pub(crate) source_name: String,
    #[serde(default)]
    pub(crate) content_type: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) cover_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) latest_chapter: Option<String>,
    pub(crate) book_url: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) kind: Option<String>,
}

pub(crate) fn project_search_result(
    result_id: &str,
    source: &SourceRecord,
    book: &Value,
) -> SearchResultSource {
    let mut result = SearchResultSource {
        engine: book.clone(),
        result_id: result_id.to_owned(),
        source_id: source.id.clone(),
        source_name: source.name.clone(),
        content_type: crate::source_metadata::content_type(&source.source).to_owned(),
        title: "Untitled".to_owned(),
        author: None,
        cover_src: None,
        intro: None,
        latest_chapter: None,
        book_url: text_at(book, &["bookUrl", "url", "origin"]).unwrap_or_default(),
        media_type: None,
        kind: None,
    };
    // UI filters consume the same typed source projection as the reader.
    // Do not expose private source rules or infer media types from a title.
    if let Some(media_type) = crate::source_metadata::media_type(&source.source) {
        result.media_type = Some(media_type.to_owned());
    } else if let Some(kind) = book
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| matches!(*kind, "image" | "manga" | "manhua" | "manhwa" | "漫画"))
    {
        result.kind = Some(kind.to_owned());
    }
    let read = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| book.get(*key).filter(|value| !value.is_null()).cloned())
            .and_then(|value| value.as_str().map(str::to_owned))
    };
    result.title = read(&["name", "title"])
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Untitled".to_owned());
    result.author = read(&["author"]);
    result.cover_src = read(&["coverUrl", "cover", "coverSrc"]);
    result.intro = read(&["intro", "introduction"]);
    result.latest_chapter = read(&["lastChapter", "latestChapter"]);
    result
}

// Keep punctuation significant for keyword matching (e.g. C++ and C#), while
// ignoring surrounding whitespace and letter case across source responses.
fn search_match_rank(title: &str, author: Option<&str>, keyword: &str) -> u8 {
    let title = title.trim().to_lowercase();
    let author = author.unwrap_or_default().trim().to_lowercase();
    if title == keyword || author == keyword {
        0
    } else if title.contains(keyword) || author.contains(keyword) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod search_content_type_tests {
    use super::*;

    #[test]
    fn search_matching_ignores_case_and_edge_whitespace_but_keeps_punctuation() {
        assert_eq!(search_match_rank(" DUNE ", None, "dune"), 0);
        assert_eq!(search_match_rank("Children of Dune", None, "dune"), 1);
        assert_eq!(search_match_rank("其他书", Some(" Dune "), "dune"), 0);
        assert_eq!(search_match_rank("C# 入门", None, "c++"), 2);
        assert_eq!(search_match_rank("其他书", Some("作者"), "dune"), 2);
    }
}

/// 搜索结果即书籍定位，Vue 无需等待详情 API 返回 ID。
fn source_book_id(source: &SourceRecord, book: &Value) -> String {
    let url = text_at(book, &["bookUrl", "url", "origin"]).unwrap_or_else(||book.to_string());
    let kind = crate::source_metadata::content_type(&source.source);
    format!("cache-{:016x}", stable_hash(&format!("{}\0{kind}\0{url}", source.id)))
}
