//! Rust application services and Tauri commands.
//!
//! 服务实现按书籍、书源、阅读状态和资源处理等职责拆分到子模块中。
//!
//! Rust owns source definition persistence and validation. The management UI
//! reads the complete JSON from an AppBootstrap resource; display pages consume
//! processed resource JSON/HTML, and only KMP executes source rules.

use std::io::{Cursor, Read, Write};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Weak, atomic::AtomicU64},
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
        BookDocument, BookMediaType, CURRENT_SCHEMA_VERSION, ChapterDescriptor, ProgressDocument,
        ProgressSummary, ReaderDefaults, ReaderTheme,
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
mod book_display;
mod book_shelf;

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
mod chapter_refresh;
mod chapter_resources;
mod general;
mod reader_state;
mod source_change;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod source_executor;
mod source_helpers;
mod source_management;
pub(crate) use source_management::validate_source_records;
mod source_search;
mod state;

// 后台任务、TTS 和前端命令适配。
mod task_execution;
mod task_lifecycle;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod tauri_commands;
mod tts;

// 子模块通过父模块门面共享必要的业务辅助函数。
use book_catalog::*;
use book_display::*;
use chapter_resources::*;
use general::*;
use source_change::*;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use source_executor::TauriSourceExecutor;
use source_helpers::*;
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
const BOOK_SOURCE_CANDIDATE_TTL_MS: u64 = 30 * 60 * 1000;
const MAX_SOURCE_IMPORT_BODY_BYTES: usize = 32 * 1024 * 1024;
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
    private_root: PathBuf,
    _process_lock: Arc<crate::resource_transactions::AppDataProcessLock>,
    store: Arc<ResourceStore>,
    server: Arc<ResourceServer>,
    executor: Arc<dyn SourceExecutor>,
    book_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    chapter_locks: Arc<tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
    sources_lock: Arc<tokio::sync::Mutex<()>>,
    tasks: Arc<tokio::sync::Mutex<TaskRegistry>>,
    task_slots: Arc<tokio::sync::Semaphore>,
    task_notifier: Arc<std::sync::RwLock<Option<Arc<dyn Fn(Value) + Send + Sync>>>>,
    task_recovery_notifier: Arc<std::sync::RwLock<Option<Arc<dyn Fn(Value) + Send + Sync>>>>,
    search_snapshot_notifier: Arc<std::sync::RwLock<Option<Arc<dyn Fn(Value) + Send + Sync>>>>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppTask {
    id: String,
    kind: String,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    book_id: Option<String>,
    /// Stable chapter resources captured when a download request is created.
    /// This keeps resumable work anchored when a catalog is reordered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chapter_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keyword: Option<String>,
    #[serde(default)]
    page: u32,
    #[serde(default)]
    from_index: usize,
    #[serde(default)]
    total: usize,
    #[serde(default)]
    completed: usize,
    #[serde(default)]
    check_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    search_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct TaskSignal {
    paused: bool,
    cancelled: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct BookRemovalOutcome {
    pub(crate) book_id: String,
    pub(crate) shelf: Value,
    pub(crate) recovery_required: bool,
    pub(crate) warning: Option<String>,
}

impl BookRemovalOutcome {
    fn into_value(self) -> Value {
        json!({
            "outcome": "deleted",
            "commitState": "committed",
            "bookId": self.book_id,
            "shelf": self.shelf,
            "recoveryRequired": self.recovery_required,
            "warning": self.warning,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BookRemovalError {
    pub(crate) commit_state: crate::resource_transactions::CommitState,
    pub(crate) recovery_required: bool,
    pub(crate) detail: String,
}

impl BookRemovalError {
    fn not_committed(detail: impl Into<String>, recovery_required: bool) -> Self {
        Self {
            commit_state: crate::resource_transactions::CommitState::NotCommitted,
            recovery_required,
            detail: detail.into(),
        }
    }

    fn transaction(error: crate::resource_transactions::TransactionError) -> Self {
        Self {
            commit_state: error.commit_state(),
            recovery_required: error.recovery_required_flag(),
            detail: error.to_string(),
        }
    }

    fn ui_message(&self) -> String {
        use crate::resource_transactions::CommitState;
        let outcome = match (self.commit_state, self.recovery_required) {
            (CommitState::NotCommitted, false) => {
                "Book was not removed; the deletion was not committed and its prior files were retained."
            }
            (CommitState::NotCommitted, true) => {
                "Book removal was not committed, but rollback or an earlier recovery is incomplete. Files may be mixed; restart the application before retrying."
            }
            (CommitState::Committed, _) => {
                "Book removal was committed. Cleanup is pending; restart the application before further writes."
            }
            (CommitState::Indeterminate, _) => {
                "Book removal status is unknown. Do not retry; restart the application to recover the transaction first."
            }
        };
        format!("{outcome} Details: {}", self.detail)
    }

    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    fn into_value(self) -> Value {
        use crate::resource_transactions::CommitState;
        let commit_state = match self.commit_state {
            CommitState::NotCommitted => "notCommitted",
            CommitState::Committed => "committed",
            CommitState::Indeterminate => "indeterminate",
        };
        json!({
            "commitState": commit_state,
            "recoveryRequired": self.recovery_required,
            "error": self.detail,
        })
    }
}

#[derive(Default)]
struct TaskRegistry {
    records: Vec<AppTask>,
    signals: HashMap<String, tokio::sync::watch::Sender<TaskSignal>>,
    /// Only this discovery-search task may publish result snapshots.
    active_search_task_id: Option<String>,
    /// Accumulated search documents stay in memory until an immutable UI snapshot is published.
    search_documents: HashMap<String, Value>,
    /// Canonical source-engine search data is session-only; result IDs never name private files.
    search_result_documents: HashMap<String, Value>,
    /// Candidate identity contexts remain in memory with the candidate task.
    search_replacement_contexts: HashMap<String, Value>,
    /// Associates a prepared shelf book with the discovery group it came from.
    search_book_group_roots: HashMap<String, String>,
}

fn empty_search_task_document(task: &AppTask) -> Value {
    json!({
        "schemaVersion": CURRENT_SCHEMA_VERSION,
        "searchId": task.search_id,
        "taskId": task.id,
        "keyword": task.keyword,
        "sourceIds": task.source_ids,
        "page": task.page,
        "results": [],
        "errors": [],
        "status": task.status,
        "completedSources": task.completed,
        "totalSources": task.total,
        "complete": false,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) source: Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetadata {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    pub enabled: bool,
    pub is_rss: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub capabilities: SourceCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent_override: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
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
    pub(crate) result_id: String,
    pub(crate) source_id: String,
    pub(crate) source_name: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) cover_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) latest_chapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) book_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) requires_identity_confirmation: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchBookResult {
    #[serde(flatten)]
    pub(crate) primary: SearchResultSource,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) sources: Option<Vec<SearchResultSource>>,
}

pub(crate) fn project_search_result(
    result_id: &str,
    source: &SourceRecord,
    book: &Value,
) -> SearchResultSource {
    let mut result = SearchResultSource {
        result_id: result_id.to_owned(),
        source_id: source.id.clone(),
        source_name: source.name.clone(),
        title: "Untitled".to_owned(),
        author: None,
        cover_src: None,
        intro: None,
        latest_chapter: None,
        book_url: None,
        media_type: None,
        kind: None,
        requires_identity_confirmation: None,
    };
    // UI filters consume the same typed source projection as the reader.
    // Do not expose private source rules or infer media types from a title.
    if let Some(media_type) = crate::source_metadata::media_type(&source.source) {
        result.media_type = Some(media_type.to_owned());
    } else if let Some(kind) = book
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| matches!(*kind, "comic" | "manga" | "manhua" | "manhwa" | "漫画"))
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
    result.book_url = read(&["bookUrl"]);
    result
}

fn search_result_identity(title: &str, author: Option<&str>) -> (String, String) {
    (
        source_change::normalize_identity(title),
        source_change::normalize_identity(author.unwrap_or_default()),
    )
}

/// Add one source leaf to the display group for its normalized title and
/// author. The primary leaf remains the top-level result used by existing
/// add/open commands; `sources` contains every distinct source candidate.
type SearchResultGroupIndexes = HashMap<(String, String), usize>;

fn search_result_group_indexes(results: &[Value]) -> SearchResultGroupIndexes {
    let mut indexes = SearchResultGroupIndexes::new();
    for (index, result) in results.iter().enumerate() {
        let title = result.get("title").and_then(Value::as_str).unwrap_or_default();
        let author = result.get("author").and_then(Value::as_str);
        indexes
            .entry(search_result_identity(title, author))
            .or_insert(index);
    }
    indexes
}

fn merge_search_result(
    results: &mut Vec<Value>,
    indexes: &mut SearchResultGroupIndexes,
    source: SearchResultSource,
) -> Result<bool, String> {
    let identity = search_result_identity(&source.title, source.author.as_deref());
    if let Some(index) = indexes.get(&identity).copied() {
        let existing = results
            .get_mut(index)
            .ok_or_else(|| "Search result group index is out of range".to_owned())?;
        if existing
            .get("sources")
            .and_then(Value::as_array)
            .is_some_and(|sources| sources.iter().any(|candidate| {
                candidate.get("sourceId").and_then(Value::as_str)
                    == Some(source.source_id.as_str())
                    && candidate.get("bookUrl").and_then(Value::as_str)
                        == source.book_url.as_deref()
            }))
        {
            return Ok(false);
        }
        let source = serde_json::to_value(source)
            .map_err(|error| format!("Cannot encode search result source: {error}"))?;
        if let Some(sources) = existing.get_mut("sources").and_then(Value::as_array_mut) {
            sources.push(source);
        } else {
            let primary = existing.clone();
            let object = existing.as_object_mut()
                .ok_or_else(|| "Search result group is not an object".to_owned())?;
            object.insert("sources".into(), json!([primary, source]));
        }
        return Ok(true);
    }
    let index = results.len();
    let group = SearchBookResult {
        primary: source.clone(),
        sources: Some(vec![source]),
    };
    results.push(serde_json::to_value(group)
        .map_err(|error| format!("Cannot encode grouped search result: {error}"))?);
    indexes.insert(identity, index);
    Ok(true)
}
