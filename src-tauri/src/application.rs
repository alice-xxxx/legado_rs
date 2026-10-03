//! Rust application services and Tauri commands.
//!
//! Source definitions and parser output remain inside this module's private
//! data directory. The WebView receives processed resource JSON/HTML only.

use std::io::Write;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{
    models::{
        BookDocument, CURRENT_SCHEMA_VERSION, ChapterDescriptor, ProgressDocument, ProgressSummary,
        ReaderDefaults, ReaderTheme,
    },
    resources::{ResourceRef, ResourceServer, ResourceStore},
    source_engine::SourceEngineRequest,
};

pub type EngineFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

/// The boundary for executing a source rule. A source definition is only ever
/// supplied to this executor by Rust, never by the WebView.
pub trait SourceExecutor: Send + Sync {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a>;
}

const PENDING_PDF_IMPORT_TTL: Duration = Duration::from_secs(5 * 60);
const BOOK_SOURCE_CANDIDATE_TTL_MS: u64 = 30 * 60 * 1000;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
const MAX_PICKER_FILE_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
const MAX_PICKER_SOURCE_JSON_BYTES: u64 = 32 * 1024 * 1024;

pub struct PickerFile {
    path: PathBuf,
    temporary: bool,
    cleanup_dir: Option<PathBuf>,
}

impl PickerFile {
    /// Wrap a caller-owned file path for the shared PDF challenge flow.
    /// Native picker flows use a temporary staged path and transfer cleanup
    /// ownership into the same type before calling the service.
    pub fn from_existing_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            temporary: false,
            cleanup_dir: None,
        }
    }
}

impl Drop for PickerFile {
    fn drop(&mut self) {
        if self.temporary {
            let _ = std::fs::remove_file(&self.path);
            if let Some(directory) = &self.cleanup_dir {
                let _ = std::fs::remove_dir_all(directory);
            }
        }
    }
}

struct PendingPdfImport {
    file: PickerFile,
    created_at: Instant,
}

/// GUI-independent owner for temporary protected-PDF picker files.
/// Supplying an `Instant` to these methods keeps expiry, retry, and cleanup
/// behavior testable in headless builds without waiting for the production TTL.
#[derive(Clone)]
struct PendingPdfImportRegistry {
    ttl: Duration,
    state: Arc<std::sync::Mutex<PendingPdfImportState>>,
}

#[derive(Default)]
struct PendingPdfImportState {
    active_token: Option<String>,
    created_at: Option<Instant>,
    pending: Option<PendingPdfImport>,
}

impl PendingPdfImportRegistry {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            state: Arc::new(std::sync::Mutex::new(PendingPdfImportState::default())),
        }
    }

    fn insert_at(&self, file: PickerFile, now: Instant) -> Result<String, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        // A service has one active password challenge. Starting another
        // protected-file import invalidates and cleans up the older picker.
        let token = uuid::Uuid::new_v4().simple().to_string();
        let replaced = state.pending.replace(PendingPdfImport {
            file,
            created_at: now,
        });
        state.active_token = Some(token.clone());
        state.created_at = Some(now);
        drop(state);
        drop(replaced);
        Ok(token)
    }

    fn take_at(&self, token: &str, now: Instant) -> Result<Option<PendingPdfImport>, String> {
        self.prune_at(now)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        if state.active_token.as_deref() != Some(token) {
            return Ok(None);
        }
        Ok(state.pending.take())
    }

    /// Reinsert an attempted import after a recoverable parse/password error.
    /// Expiry remains anchored to the original picker selection time.
    fn reinsert_at(
        &self,
        token: String,
        pending: PendingPdfImport,
        now: Instant,
    ) -> Result<(), String> {
        if now
            .checked_duration_since(pending.created_at)
            .unwrap_or_default()
            >= self.ttl
        {
            self.expire_at(&token, now)?;
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        if state.active_token.as_deref() == Some(token.as_str()) && state.pending.is_none() {
            state.pending = Some(pending);
        }
        Ok(())
    }

    fn expire_at(&self, token: &str, now: Instant) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        let expired = state.active_token.as_deref() == Some(token)
            && state.created_at.is_some_and(|created_at| {
                now.checked_duration_since(created_at).unwrap_or_default() >= self.ttl
            });
        if expired {
            state.active_token = None;
            state.created_at = None;
            state.pending = None;
        }
        Ok(expired)
    }

    fn cancel_at(&self, token: &str, now: Instant) -> Result<bool, String> {
        self.prune_at(now)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        if state.active_token.as_deref() != Some(token) {
            return Ok(false);
        }
        state.active_token = None;
        state.created_at = None;
        state.pending = None;
        Ok(true)
    }

    fn clear(&self) -> Result<(), String> {
        let old = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
            std::mem::take(&mut *state)
        };
        drop(old);
        Ok(())
    }

    fn prune_at(&self, now: Instant) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        let expired = state.created_at.is_some_and(|created_at| {
            now.checked_duration_since(created_at).unwrap_or_default() >= self.ttl
        });
        if expired {
            *state = PendingPdfImportState::default();
        }
        Ok(())
    }
}

fn schedule_pending_pdf_cleanup(registry: PendingPdfImportRegistry, token: String) {
    tokio::spawn(async move {
        tokio::time::sleep(PENDING_PDF_IMPORT_TTL).await;
        let _ = registry.expire_at(&token, Instant::now());
    });
}

/// Desktop/JVM executor also used by the real-browser harness. Mobile app
/// builds use `TauriSourceExecutor`, which retains the native plugin bridge.
pub struct DesktopSourceExecutor {
    data_dir: PathBuf,
    resource_dir: Option<PathBuf>,
}

impl DesktopSourceExecutor {
    pub fn new(data_dir: PathBuf, resource_dir: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            resource_dir,
        }
    }
}

impl SourceExecutor for DesktopSourceExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(crate::source_engine::execute_with_resource_dir(
            request,
            self.data_dir.clone(),
            self.resource_dir.clone(),
        ))
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub struct TauriSourceExecutor {
    app: tauri::AppHandle,
    data_dir: PathBuf,
    resource_dir: Option<PathBuf>,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl TauriSourceExecutor {
    pub fn new(app: tauri::AppHandle, data_dir: PathBuf, resource_dir: Option<PathBuf>) -> Self {
        Self {
            app,
            data_dir,
            resource_dir,
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl SourceExecutor for TauriSourceExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                use tauri_plugin_source_engine::{SourceEngineCall, SourceEngineExt};

                let request_json = serde_json::json!({
                    "operation": request.operation,
                    "source": request.source,
                    "keyword": request.keyword,
                    "page": request.page,
                    "book": request.book,
                    "chapter": request.chapter,
                    "nextChapterUrl": request.next_chapter_url,
                })
                .to_string();
                let response = self
                    .app
                    .source_engine()
                    .execute(SourceEngineCall { request_json })
                    .map_err(|error| error.to_string())?;
                let result: Value =
                    serde_json::from_str(&response.result_json).map_err(|error| {
                        format!("Kotlin source engine returned invalid JSON: {error}")
                    })?;
                if let Some(error) = result.get("__sourceEngineError").and_then(Value::as_str) {
                    return Err(format!("Kotlin source engine failed:\n{error}"));
                }
                Ok(result)
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                crate::source_engine::execute_with_resource_dir(
                    request,
                    self.data_dir.clone(),
                    self.resource_dir.clone(),
                )
                .await
            }
        })
    }
}

/// Resource-oriented application service. It can run without Tauri and is
/// therefore also usable by core integration tests and the browser harness.
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
    admission_gate: Arc<tokio::sync::RwLock<()>>,
    operation_gate: Arc<tokio::sync::RwLock<()>>,
    restore_barrier: Arc<tokio::sync::watch::Sender<bool>>,
    restore_serial: Arc<tokio::sync::Mutex<()>>,
    pending_pdf_imports: PendingPdfImportRegistry,
}

struct RestoreAdmissionGuard {
    barrier: Arc<tokio::sync::watch::Sender<bool>>,
}

pub struct OperationReadGuard {
    _admission: tokio::sync::OwnedRwLockReadGuard<()>,
    _operation: tokio::sync::OwnedRwLockReadGuard<()>,
}

impl Drop for RestoreAdmissionGuard {
    fn drop(&mut self) {
        self.barrier.send_replace(false);
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

#[derive(Default)]
struct TaskRegistry {
    records: Vec<AppTask>,
    signals: HashMap<String, tokio::sync::watch::Sender<TaskSignal>>,
}

impl ApplicationService {
    pub async fn open(
        root: impl Into<PathBuf>,
        resource_dir: Option<PathBuf>,
    ) -> Result<Self, String> {
        let root = root.into();
        let executor = DesktopSourceExecutor::new(root.join("source-engine"), resource_dir);
        Self::open_with_executor(root, Arc::new(executor)).await
    }

    pub async fn open_with_executor(
        root: impl Into<PathBuf>,
        executor: Arc<dyn SourceExecutor>,
    ) -> Result<Self, String> {
        let requested_root = root.into();
        let process_lock = Arc::new(
            crate::resource_transactions::AppDataProcessLock::acquire(&requested_root)
                .map_err(|error| error.to_string())?,
        );
        let root = process_lock.data_root().to_path_buf();
        crate::backup::recover_interrupted_restore(&root)?;
        crate::resource_transactions::recover_all(&root).map_err(|error| error.to_string())?;
        let store = Arc::new(ResourceStore::open(root.clone()).map_err(|error| error.to_string())?);
        let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let server = Arc::new(
            store
                .start_http(bind)
                .await
                .map_err(|error| error.to_string())?,
        );
        let private_root = root.join("private-data");
        tokio::fs::create_dir_all(private_root.join("search-results"))
            .await
            .map_err(|error| {
                format!("Cannot create private application data directory: {error}")
            })?;
        tokio::fs::create_dir_all(private_root.join("books"))
            .await
            .map_err(|error| {
                format!("Cannot create private application data directory: {error}")
            })?;
        tokio::fs::create_dir_all(private_root.join("discovery-categories"))
            .await
            .map_err(|error| format!("Cannot create private discovery data directory: {error}"))?;
        let staged_picker_imports = private_root.join("picker-imports");
        match tokio::fs::remove_dir_all(&staged_picker_imports).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Cannot clear stale picker imports: {error}"));
            }
        }
        let root = store.root().to_path_buf();
        let task_ref = store
            .reading_ref("tasks")
            .map_err(|error| error.to_string())?;
        let task_path = root.join("reading").join("tasks.json");
        let mut task_records = if tokio::fs::metadata(&task_path).await.is_ok() {
            let document = store
                .read_json_ref(&task_ref)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::from_value::<Vec<AppTask>>(
                document.get("tasks").cloned().unwrap_or_else(|| json!([])),
            )
            .map_err(|error| format!("Cannot parse task state: {error}"))?
        } else {
            Vec::new()
        };
        let mut task_registry = TaskRegistry {
            records: task_records,
            signals: HashMap::new(),
        };
        for task in &mut task_registry.records {
            if matches!(
                task.status.as_str(),
                "queued" | "running" | "pausing" | "cancelling"
            ) {
                task.status = "interrupted".to_owned();
                task.updated_at_ms = now_ms();
            }
        }
        task_records = task_registry.records.clone();
        store
            .write_json_ref(
                &task_ref,
                &json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "tasks": task_records }),
            )
            .await
            .map_err(|error| error.to_string())?;
        let (restore_barrier, _) = tokio::sync::watch::channel(false);
        Ok(Self {
            root,
            private_root,
            _process_lock: process_lock,
            store,
            server,
            executor,
            book_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            chapter_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            sources_lock: Arc::new(tokio::sync::Mutex::new(())),
            tasks: Arc::new(tokio::sync::Mutex::new(task_registry)),
            task_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            task_notifier: Arc::new(std::sync::RwLock::new(None)),
            admission_gate: Arc::new(tokio::sync::RwLock::new(())),
            operation_gate: Arc::new(tokio::sync::RwLock::new(())),
            restore_barrier: Arc::new(restore_barrier),
            restore_serial: Arc::new(tokio::sync::Mutex::new(())),
            pending_pdf_imports: PendingPdfImportRegistry::new(PENDING_PDF_IMPORT_TTL),
        })
    }

    pub fn resource_server(&self) -> &ResourceServer {
        &self.server
    }

    pub fn resource_store(&self) -> &ResourceStore {
        &self.store
    }

    pub fn data_root(&self) -> &Path {
        &self.root
    }

    pub fn set_task_notifier(&self, notifier: Arc<dyn Fn(Value) + Send + Sync>) {
        if let Ok(mut target) = self.task_notifier.write() {
            *target = Some(notifier);
        }
    }

    pub async fn operation_read(&self) -> OperationReadGuard {
        let mut barrier = self.restore_barrier.subscribe();
        loop {
            if *barrier.borrow() {
                if barrier.changed().await.is_err() {
                    continue;
                }
                continue;
            }
            let admission = self.admission_gate.clone().read_owned().await;
            if *barrier.borrow() {
                drop(admission);
                continue;
            }
            let operation = self.operation_gate.clone().read_owned().await;
            if !*barrier.borrow() {
                return OperationReadGuard {
                    _admission: admission,
                    _operation: operation,
                };
            }
            drop(operation);
            drop(admission);
        }
    }

    pub fn resource_descriptor(&self, resource: &ResourceRef) -> Value {
        json!({
            "resourceId": resource.as_str(),
            "src": self.server.url_for(resource),
            "contentType": content_type(resource.as_str()),
        })
    }

    /// Return the browser-readable, processed home configuration resource.
    pub async fn get_home_config(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::discovery::home_config::home_config_resource(&self.store).await?;
        Ok(self.resource_descriptor(&resource))
    }

    /// Validate and save the home layout. Source/category IDs are resolved by
    /// Rust; the public document contains no source URLs or source rules.
    pub async fn save_home_config(
        &self,
        config: crate::discovery::home_config::HomeConfigDocument,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let input = serde_json::to_value(config)
            .map_err(|error| format!("Cannot encode home configuration: {error}"))?;
        let resource = crate::discovery::home_config::save_home_config(self, input).await?;
        Ok(self.resource_descriptor(&resource))
    }

    /// Return the RSS read/favorite/filter state resource.
    pub async fn get_rss_state(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::rss::rss_state_resource(&self.store).await?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn set_rss_filter(&self, source_id: &str, filter: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::rss::set_subscription_filter(self, source_id, filter).await?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn set_rss_article_state(
        &self,
        source_id: &str,
        article_id: &str,
        is_read: Option<bool>,
        is_favorite: Option<bool>,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource =
            crate::rss::set_article_state(self, source_id, article_id, is_read, is_favorite)
                .await?;
        Ok(self.resource_descriptor(&resource))
    }

    /// Remove an RSS subscription and its private category mapping, cached
    /// article HTML, and read/favorite/filter state under one restore guard.
    pub async fn unsubscribe_rss(&self, source_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let _sources = self.sources_lock.lock().await;
        let resource = crate::rss::remove_subscription_data(self, source_id).await?;
        let mut records = self.read_sources().await?;
        records.retain(|source| source.id != source_id);
        self.write_sources(&records).await?;
        Ok(json!({
            "sources": metadata(&records),
            "resource": self.resource_descriptor(&resource),
        }))
    }

    /// Initialize the TXT chapter-recognition document and return its public
    /// JSON resource. These are local import patterns, never book-source rules.
    pub async fn get_txt_toc_rules(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        crate::local_books::txt_toc_rules::read_document(&self.store).await?;
        let resource = crate::local_books::txt_toc_rules::resource_ref(&self.store)?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn upsert_txt_toc_rule(
        &self,
        rule: crate::local_books::txt_toc_rules::TxtTocRule,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        crate::local_books::txt_toc_rules::upsert_rule(&self.store, rule).await?;
        let resource = crate::local_books::txt_toc_rules::resource_ref(&self.store)?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn delete_txt_toc_rule(&self, rule_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        crate::local_books::txt_toc_rules::delete_rule(&self.store, rule_id).await?;
        let resource = crate::local_books::txt_toc_rules::resource_ref(&self.store)?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn bootstrap(&self) -> Result<Value, String> {
        crate::search_history::load(&self.store).await?;
        let search_history = crate::search_history::resource_ref(&self.store)?;
        Ok(json!({
            "shelf": self.resource_descriptor(&self.store.shelf_ref()),
            "settings": self.resource_descriptor(&self.store.settings_ref()),
            "sources": self.source_metadata().await?,
            "searchHistory": self.resource_descriptor(&search_history),
        }))
    }

    pub async fn get_search_history(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        crate::search_history::load(&self.store).await?;
        let resource = crate::search_history::resource_ref(&self.store)?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn delete_search_history(&self, query: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::search_history::delete_query(&self.store, query).await?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn clear_search_history(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::search_history::clear(&self.store).await?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn list_sources(&self) -> Result<Value, String> {
        Ok(json!({ "sources": self.source_metadata().await? }))
    }

    pub async fn import_sources(&self, source_json: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let imported: Value = serde_json::from_str(source_json)
            .map_err(|error| format!("Invalid source JSON: {error}"))?;
        let sources = extract_sources(imported)?;
        let _lock = self.sources_lock.lock().await;
        let mut records = self.read_sources().await?;
        for (position, source) in sources.into_iter().enumerate() {
            let name = text_at(&source, &["bookSourceName", "name", "sourceName"])
                .unwrap_or_else(|| format!("Source {}", position + 1));
            let source_url =
                text_at(&source, &["bookSourceUrl", "url", "sourceUrl"]).unwrap_or_default();
            if source_url.is_empty() {
                return Err(format!("Source '{name}' is missing bookSourceUrl"));
            }
            let group = text_at(&source, &["bookSourceGroup", "group"]);
            let enabled = bool_at(&source, &["enabled", "bookSourceEnabled"]).unwrap_or(true);
            let id = format!("source-{:016x}", stable_hash(&source_url));
            let record = SourceRecord {
                id: id.clone(),
                name,
                group,
                enabled,
                source,
            };
            if let Some(existing) = records.iter_mut().find(|existing| existing.id == id) {
                *existing = record;
            } else {
                records.push(record);
            }
        }
        self.write_sources(&records).await?;
        Ok(json!({ "sources": metadata(&records) }))
    }

    pub async fn remove_sources(&self, source_ids: &[String]) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let _lock = self.sources_lock.lock().await;
        let remove = source_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let mut records = self.read_sources().await?;
        records.retain(|source| !remove.contains(source.id.as_str()));
        self.write_sources(&records).await?;
        Ok(json!({ "sources": metadata(&records) }))
    }

    pub async fn update_source(&self, source_id: &str, patch: Value) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let patch = patch
            .as_object()
            .ok_or_else(|| "Source metadata patch must be an object".to_owned())?;
        if patch
            .keys()
            .any(|key| !matches!(key.as_str(), "name" | "group" | "enabled"))
        {
            return Err("Only source name, group, and enabled metadata can be changed".into());
        }
        let _lock = self.sources_lock.lock().await;
        let mut records = self.read_sources().await?;
        let source = records
            .iter_mut()
            .find(|source| source.id == source_id)
            .ok_or_else(|| format!("Unknown source '{source_id}'"))?;
        if let Some(name) = patch.get("name") {
            source.name = name
                .as_str()
                .ok_or_else(|| "Source name must be a string".to_owned())?
                .trim()
                .to_owned();
            if source.name.is_empty() {
                return Err("Source name cannot be empty".into());
            }
        }
        if let Some(group) = patch.get("group") {
            source.group = group
                .as_str()
                .map(str::trim)
                .filter(|group| !group.is_empty())
                .map(str::to_owned);
        }
        if let Some(enabled) = patch.get("enabled") {
            source.enabled = enabled
                .as_bool()
                .ok_or_else(|| "Source enabled must be a boolean".to_owned())?;
        }
        self.write_sources(&records).await?;
        Ok(json!({ "sources": metadata(&records) }))
    }

    pub async fn search_books<F>(
        &self,
        source_ids: &[String],
        keyword: &str,
        page: u32,
        mut on_progress: F,
    ) -> Result<Value, String>
    where
        F: FnMut(Value),
    {
        let _operation = self.operation_read().await;
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return Err("Search keyword cannot be empty".into());
        }
        let page = page.max(1);
        let all_sources = self.read_sources().await?;
        let selected = all_sources
            .into_iter()
            .filter(|source| {
                source.enabled
                    && !crate::source_metadata::is_rss_source_metadata(&source.source)
                    && (source_ids.is_empty() || source_ids.contains(&source.id))
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("No enabled book sources are selected".into());
        }
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        crate::search_history::record_search(&self.store, &search_id, keyword, now_ms()).await?;

        let mut source_results = Vec::new();
        let mut errors = Vec::new();
        for (position, source) in selected.iter().enumerate() {
            let response = self
                .executor
                .execute(engine_request(
                    "search",
                    &source.source,
                    Some(keyword.to_owned()),
                    Some(page as i32),
                    None,
                    None,
                    None,
                ))
                .await;
            match response {
                Ok(value) => {
                    let books = value
                        .get("books")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    for book in books {
                        source_results.push((source.clone(), book));
                    }
                }
                Err(error) => errors.push(
                    json!({ "sourceId": source.id, "sourceName": source.name, "message": error }),
                ),
            }
            on_progress(json!({
                "keyword": keyword,
                "page": page,
                "sourceId": source.id,
                "sourceName": source.name,
                "completedSources": position + 1,
                "totalSources": selected.len(),
                "resultCount": source_results.len(),
            }));
        }
        self.store_processed_results_with_search_id(
            &search_id,
            keyword,
            page,
            source_results,
            errors,
        )
        .await
    }

    pub async fn start_search(
        &self,
        source_ids: &[String],
        keyword: &str,
        page: u32,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return Err("Search keyword cannot be empty".into());
        }
        let page = page.max(1);
        let selected = self
            .read_sources()
            .await?
            .into_iter()
            .filter(|source| {
                source.enabled
                    && !crate::source_metadata::is_rss_source_metadata(&source.source)
                    && (source_ids.is_empty() || source_ids.contains(&source.id))
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("No enabled book sources are selected".into());
        }
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let resource = self
            .store
            .search_ref(&search_id)
            .map_err(|error| error.to_string())?;
        self.store
            .write_json_ref(
                &resource,
                &json!({
                    "schemaVersion": CURRENT_SCHEMA_VERSION,
                    "keyword": keyword,
                    "page": page,
                    "results": [],
                    "errors": [],
                    "complete": false,
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
        crate::search_history::record_search(&self.store, &search_id, keyword, now_ms()).await?;
        let now = now_ms();
        let task = AppTask {
            id: format!("task-{}", uuid::Uuid::new_v4().simple()),
            kind: "search".into(),
            status: "queued".into(),
            book_id: None,
            source_ids: Some(selected.iter().map(|source| source.id.clone()).collect()),
            keyword: Some(keyword.to_owned()),
            page,
            from_index: 0,
            total: selected.len(),
            completed: 0,
            check_only: false,
            search_id: Some(search_id),
            result: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let task_id = task.id.clone();
        let response_task_id = task_id.clone();
        let receiver = self.create_task(task).await?;
        let service = self.clone();
        tokio::spawn(async move {
            service.run_task(task_id, receiver).await;
        });
        let descriptor = self.resource_descriptor(&resource);
        Ok(
            json!({ "taskId": response_task_id, "task": self.task_summary_by_id(&response_task_id).await?, "resource": descriptor }),
        )
    }

    /// Search other enabled novel sources for a replacement for an existing
    /// book. The private context binds every result to this book's current
    /// source and catalog snapshot; ordinary search results cannot be used by
    /// `change_book_source`.
    pub async fn search_book_source_candidates(
        &self,
        book_id: &str,
        source_ids: &[String],
        keyword: Option<&str>,
        page: u32,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;

        let (selected, context, effective_keyword) = {
            // Keep the same lock order used by catalog commits. No source or
            // book lock is held while the executor searches the network.
            let _sources = self.sources_lock.lock().await;
            let records = self.read_sources().await?;
            let _book = self.book_lock(book_id).await;
            let private_path = Path::new("books").join(format!("{book_id}.json"));
            let mut private = self
                .read_private_json(&private_path)
                .await
                .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
            let book_ref = self
                .store
                .book_ref(book_id)
                .map_err(|error| error.to_string())?;
            let book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
            if private
                .get("bookInstanceId")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                private["bookInstanceId"] = json!(uuid::Uuid::new_v4().simple().to_string());
                self.write_private_json(&private_path, &private).await?;
            }
            let original_source_id = private["sourceId"]
                .as_str()
                .ok_or_else(|| "Private book is missing its source ID".to_owned())?
                .to_owned();
            if !can_change_source_from_private(&private) {
                return Err("This book is not linked to a replaceable online source".into());
            }
            let original_source = records
                .iter()
                .find(|source| source.id == original_source_id);
            let original_source_definition = original_source
                .map(|source| source.source.clone())
                .unwrap_or(Value::Null);
            let effective_keyword = keyword
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    text_at(&book, &["title"])
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                });
            if normalize_identity(&effective_keyword).is_empty() {
                return Err("A title or search keyword is required to find another source".into());
            }
            let fingerprint = book_source_fingerprint(
                &original_source_id,
                &original_source_definition,
                &private,
                &book,
            )?;
            let selected = records
                .iter()
                .filter(|source| {
                    source.enabled
                        && source.id != original_source_id
                        && !crate::source_metadata::is_rss_source_metadata(&source.source)
                        && (source_ids.is_empty() || source_ids.contains(&source.id))
                })
                .cloned()
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err("No other enabled novel sources are selected".into());
            }
            let now = now_ms();
            let context = json!({
                "schemaVersion": CURRENT_SCHEMA_VERSION,
                "targetBookId": book_id,
                "originalSourceId": original_source_id,
                "originalSourcePresent": original_source.is_some(),
                "originalSourceFingerprint": original_source
                    .map(|source| source_definition_fingerprint(&source.source))
                    .transpose()?,
                "originalSourceRevision": self.source_revision(&original_source_id).await?,
                "catalogFingerprint": fingerprint,
                "catalogGeneration": private.get("catalogGeneration").cloned().unwrap_or(Value::Null),
                "bookInstanceId": private["bookInstanceId"],
                "targetTitle": text_at(&book, &["title"]).unwrap_or_default(),
                "targetAuthor": text_at(&book, &["author"]).unwrap_or_default(),
                "createdAtMs": now,
                "expiresAtMs": now.saturating_add(BOOK_SOURCE_CANDIDATE_TTL_MS),
            });
            (selected, context, effective_keyword)
        };

        let search_id = format!("replace-{}", uuid::Uuid::new_v4().simple());
        let resource = self
            .store
            .search_ref(&search_id)
            .map_err(|error| error.to_string())?;
        self.store
            .write_json_ref(
                &resource,
                &json!({
                    "schemaVersion": CURRENT_SCHEMA_VERSION,
                    "keyword": effective_keyword,
                    "page": page.max(1),
                    "results": [],
                    "errors": [],
                    "complete": false,
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
        let context_path =
            Path::new("search-results").join(format!("{search_id}.replacement.json"));
        self.write_private_json(&context_path, &context).await?;

        let now = now_ms();
        let task = AppTask {
            id: format!("task-{}", uuid::Uuid::new_v4().simple()),
            kind: "bookSourceCandidates".into(),
            status: "queued".into(),
            book_id: Some(book_id.to_owned()),
            source_ids: Some(selected.iter().map(|source| source.id.clone()).collect()),
            keyword: Some(effective_keyword),
            page: page.max(1),
            from_index: 0,
            total: selected.len(),
            completed: 0,
            check_only: false,
            search_id: Some(search_id),
            result: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let task_id = task.id.clone();
        let receiver = self.create_task(task).await?;
        let service = self.clone();
        let worker_id = task_id.clone();
        tokio::spawn(async move {
            service.run_task(worker_id, receiver).await;
        });
        Ok(json!({
            "bookId": book_id,
            "taskId": task_id,
            "task": self.task_summary_by_id(&task_id).await?,
            "resource": self.resource_descriptor(&resource),
        }))
    }

    /// Confirm a source replacement. Both source engine calls happen without
    /// source/book locks; the method then reacquires locks in the established
    /// order and validates the candidate's book/source/catalog snapshot again
    /// before publishing any new resources.
    pub async fn change_book_source(
        &self,
        book_id: &str,
        result_id: &str,
        confirm_missing_author: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        validate_id(result_id, "resultId")?;
        let candidate_cache = self
            .read_private_json(Path::new("search-results").join(format!("{result_id}.json")))
            .await
            .map_err(|_| "Replacement candidate is unavailable; search again".to_owned())?;
        let context = candidate_cache
            .get("replacementContext")
            .cloned()
            .ok_or_else(|| "This search result is not a replacement candidate".to_owned())?;
        if context.get("targetBookId").and_then(Value::as_str) != Some(book_id) {
            return Err("Replacement candidate belongs to a different book".into());
        }
        let expires_at_ms = context["expiresAtMs"]
            .as_u64()
            .ok_or_else(|| "Replacement candidate has invalid expiry metadata".to_owned())?;
        if now_ms() > expires_at_ms {
            return Err("Replacement candidate expired; search again".into());
        }
        if context["requiresIdentityConfirmation"].as_bool() == Some(true)
            && !confirm_missing_author
        {
            return Err(
                "Confirm that this title is the intended book because its author could not be verified".into(),
            );
        }
        let target_source_id = candidate_cache["sourceId"]
            .as_str()
            .ok_or_else(|| "Replacement candidate has no source ID".to_owned())?
            .to_owned();
        let candidate_book = candidate_cache
            .get("book")
            .cloned()
            .ok_or_else(|| "Replacement candidate has no engine result".to_owned())?;
        let search_id = context["searchId"]
            .as_str()
            .ok_or_else(|| "Replacement candidate has no search binding".to_owned())?;
        validate_id(search_id, "searchId")?;
        let search_ref = self
            .store
            .search_ref(search_id)
            .map_err(|error| error.to_string())?;
        let search_document =
            self.store.read_json_ref(&search_ref).await.map_err(|_| {
                "Replacement candidate search is unavailable; search again".to_owned()
            })?;
        let belongs_to_completed_search = search_document["complete"].as_bool() == Some(true)
            && search_document["cancelled"].as_bool() != Some(true)
            && search_document["results"]
                .as_array()
                .is_some_and(|results| {
                    results.iter().any(|result| {
                        result["resultId"].as_str() == Some(result_id)
                            && result["sourceId"].as_str() == Some(target_source_id.as_str())
                    })
                });
        if !belongs_to_completed_search {
            return Err("Replacement candidate is not part of a completed search".into());
        }

        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let (old_source_id, old_source_definition, new_source, old_book) = {
            let _sources = self.sources_lock.lock().await;
            let sources = self.read_sources().await?;
            let old_source_id = context["originalSourceId"]
                .as_str()
                .ok_or_else(|| "Replacement candidate has no original source binding".to_owned())?;
            let original_source_present = context["originalSourcePresent"]
                .as_bool()
                .ok_or_else(|| "Replacement candidate has no source-presence binding".to_owned())?;
            let old_source = sources.iter().find(|source| source.id == old_source_id);
            if original_source_present != old_source.is_some() {
                return Err(
                    "Current book source availability changed after the search; search again"
                        .into(),
                );
            }
            let old_source_definition = old_source
                .map(|source| source.source.clone())
                .unwrap_or(Value::Null);
            let original_source_fingerprint = context["originalSourceFingerprint"].as_str();
            match (
                original_source_present,
                old_source,
                original_source_fingerprint,
            ) {
                (true, Some(source), Some(expected))
                    if source_definition_fingerprint(&source.source)? == expected => {}
                (true, _, _) => {
                    return Err("Current book source changed after the search; search again".into());
                }
                (false, None, None) => {}
                (false, _, _) => {
                    return Err(
                        "Current book source availability changed after the search; search again"
                            .into(),
                    );
                }
            }
            if self.source_revision(old_source_id).await?
                != context["originalSourceRevision"].as_u64().unwrap_or(0)
            {
                return Err("Current book source changed after the search; search again".into());
            }
            let new_source = sources
                .iter()
                .find(|source| source.id == target_source_id)
                .cloned()
                .ok_or_else(|| "The replacement source was removed; search again".to_owned())?;
            if !new_source.enabled
                || crate::source_metadata::is_rss_source_metadata(&new_source.source)
            {
                return Err("Replacement source is disabled or is not a novel source".into());
            }
            if old_source_id == new_source.id {
                return Err("Choose a different source for this book".into());
            }
            if source_definition_fingerprint(&new_source.source)?
                != context["candidateSourceFingerprint"]
                    .as_str()
                    .unwrap_or_default()
            {
                return Err("Replacement source changed after the search; search again".into());
            }
            let _book = self.book_lock(book_id).await;
            let old_private = self
                .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            let old_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            let actual_fingerprint = book_source_fingerprint(
                old_source_id,
                &old_source_definition,
                &old_private,
                &old_book,
            )?;
            if actual_fingerprint != context["catalogFingerprint"].as_str().unwrap_or_default()
                || old_private
                    .get("catalogGeneration")
                    .cloned()
                    .unwrap_or(Value::Null)
                    != context["catalogGeneration"]
            {
                return Err("Book catalog changed after the search; search again".into());
            }
            (
                old_source_id.to_owned(),
                old_source_definition,
                new_source,
                old_book,
            )
        };

        let raw_new_book = if candidate_book.is_object() {
            candidate_book
        } else {
            return Err("Replacement source returned invalid book metadata".into());
        };
        let rss = crate::rss::is_legacy_rss_source(&new_source.source);
        let engine_book = self
            .executor
            .execute(engine_request(
                if rss { "rssBookInfo" } else { "bookInfo" },
                &new_source.source,
                None,
                None,
                Some(raw_new_book.clone()),
                None,
                None,
            ))
            .await?;
        if !engine_book.is_object() {
            return Err("Replacement source returned invalid book details".into());
        }
        let old_title = text_at(&old_book, &["title"]).unwrap_or_default();
        let target_title = text_at(&engine_book, &["name", "title"])
            .ok_or_else(|| "Replacement source did not confirm the book title".to_owned())?;
        if normalize_identity(&target_title) != normalize_identity(&old_title) {
            return Err("Replacement source book title does not match the current book".into());
        }
        let old_author = text_at(&old_book, &["author"]).unwrap_or_default();
        let engine_author = text_at(&engine_book, &["author"]);
        if !old_author.trim().is_empty()
            && engine_author
                .as_deref()
                .is_some_and(|author| !author.trim().is_empty())
            && normalize_identity(engine_author.as_deref().unwrap_or_default())
                != normalize_identity(&old_author)
        {
            return Err("Replacement source author does not match the current book".into());
        }
        let author_unverified = old_author.trim().is_empty()
            || engine_author
                .as_deref()
                .is_none_or(|author| author.trim().is_empty());
        if author_unverified && !confirm_missing_author {
            return Err(
                "Confirm that this title is the intended book because its author could not be verified".into(),
            );
        }
        // Keep known metadata when the new source omits it; the source rule
        // engine remains the authority when it supplies a new value.
        let target_author = engine_author.unwrap_or(old_author.clone());
        let display_metadata =
            crate::book_metadata::project_book_metadata(&engine_book, &new_source);
        let raw_chapters = self
            .executor
            .execute(engine_request(
                if rss { "rssChapters" } else { "chapters" },
                &new_source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?
            .as_array()
            .cloned()
            .ok_or_else(|| "Replacement source returned an invalid chapter catalog".to_owned())?;
        if raw_chapters.is_empty() {
            return Err(
                "Replacement source returned no chapters; the current book was kept".into(),
            );
        }
        crate::catalog::reconcile_catalog(
            book_id,
            &[],
            &[],
            &raw_chapters,
            &ProgressSummary::default(),
            &HashSet::new(),
            now_ms(),
        )
        .map_err(|error| format!("Replacement chapter catalog is invalid: {error}"))?;

        let generation = uuid::Uuid::new_v4().simple().to_string();
        let new_chapters = raw_chapters
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                let raw_url =
                    text_at(raw, &["url", "chapterUrl"]).unwrap_or_else(|| format!("@{index}"));
                ChapterDescriptor {
                    id: chapter_id_for_generation(book_id, &generation, &raw_url),
                    title: text_at(raw, &["title", "chapterName", "name"])
                        .unwrap_or_else(|| format!("Chapter {}", index + 1)),
                    index,
                    // A new source always begins uncached, even if it returns
                    // URLs that happen to match the previous source.
                    src: None,
                }
            })
            .collect::<Vec<_>>();
        let mut new_ids = HashSet::with_capacity(new_chapters.len());
        if new_chapters
            .iter()
            .any(|chapter| !new_ids.insert(chapter.id.clone()))
        {
            return Err("Replacement catalog produces duplicate chapter IDs".into());
        }

        let old_chapters = serde_json::from_value::<Vec<ChapterDescriptor>>(
            old_book
                .get("chapters")
                .cloned()
                .ok_or_else(|| "Current book chapter directory is missing".to_owned())?,
        )
        .map_err(|error| format!("Cannot read current chapter directory: {error}"))?;
        let old_titles = old_chapters
            .iter()
            .map(|chapter| chapter.title.clone())
            .collect::<Vec<_>>();
        let new_titles = new_chapters
            .iter()
            .map(|chapter| chapter.title.clone())
            .collect::<Vec<_>>();
        let bookmarks_ref = crate::reading_tools::bookmarks_resource(&self.store).await?;
        let progress_ref = self
            .store
            .progress_ref(book_id)
            .map_err(|error| error.to_string())?;
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let mut migrated_bookmarks = 0usize;
        let mut orphaned_bookmarks = 0usize;
        let old_chapter_ids = old_chapters
            .iter()
            .map(|chapter| chapter.id.clone())
            .collect::<Vec<_>>();

        let _sources = self.sources_lock.lock().await;
        let latest_sources = self.read_sources().await?;
        let latest_old_source = latest_sources
            .iter()
            .find(|source| source.id == old_source_id);
        let latest_new_source = latest_sources
            .iter()
            .find(|source| source.id == new_source.id)
            .ok_or_else(|| "Replacement source was removed during replacement".to_owned())?;
        let original_source_present = context["originalSourcePresent"].as_bool().unwrap_or(false);
        let source_presence_unchanged = latest_old_source.is_some() == original_source_present;
        let source_definition_unchanged = match (original_source_present, latest_old_source) {
            (true, Some(source)) => source.source == old_source_definition,
            (false, None) => true,
            _ => false,
        };
        if !source_presence_unchanged
            || !source_definition_unchanged
            || self.source_revision(&old_source_id).await?
                != context["originalSourceRevision"].as_u64().unwrap_or(0)
            || latest_new_source.source != new_source.source
            || !latest_new_source.enabled
        {
            return Err("A source changed during replacement; search again".into());
        }
        let _book = self.book_lock(book_id).await;
        let latest_private = self
            .read_private_json(&private_path)
            .await
            .map_err(|_| format!("Book '{book_id}' was removed during replacement"))?;
        let latest_book = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|_| format!("Book '{book_id}' was removed during replacement"))?;
        if latest_private.get("sourceId").and_then(Value::as_str) != Some(old_source_id.as_str())
            || book_source_fingerprint(
                &old_source_id,
                &old_source_definition,
                &latest_private,
                &latest_book,
            )? != context["catalogFingerprint"].as_str().unwrap_or_default()
        {
            return Err("Book source or catalog changed during replacement; search again".into());
        }

        // Progress may have advanced while the source engine was resolving
        // the new metadata. Re-read it under the commit lock and remap the
        // latest value so a slow network request never rolls the reader back.
        let old_private = latest_private;
        let old_book = latest_book;
        let summary_progress = serde_json::from_value::<ProgressSummary>(
            old_book
                .get("progress")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .unwrap_or_default();
        let current_progress_ref = self
            .store
            .progress_ref(book_id)
            .map_err(|error| error.to_string())?;
        let progress_document = self
            .store
            .read_json_ref(&current_progress_ref)
            .await
            .ok()
            .and_then(|value| serde_json::from_value::<ProgressDocument>(value).ok())
            .filter(|progress| progress.book_id == book_id);
        let old_progress = progress_document
            .filter(|document| document.updated_at_ms > summary_progress.updated_at_ms)
            .map(|document| ProgressSummary {
                chapter_id: document.chapter_id,
                chapter_index: document.chapter_index,
                offset: document.offset,
                updated_at_ms: document.updated_at_ms,
            })
            .unwrap_or(summary_progress);
        let progress_old_index = old_progress
            .chapter_id
            .as_deref()
            .and_then(|id| old_chapters.iter().position(|chapter| chapter.id == id))
            .unwrap_or_else(|| {
                old_progress
                    .chapter_index
                    .min(old_chapters.len().saturating_sub(1))
            });
        let progress_title = old_chapters
            .get(progress_old_index)
            .map(|chapter| chapter.title.as_str())
            .unwrap_or_default();
        let mapped_index = unique_chapter_title_index(progress_title, &old_titles, &new_titles);
        let (progress_index, progress_offset) = match mapped_index {
            Some(index) => (index, old_progress.offset),
            None => (progress_old_index.min(new_chapters.len() - 1), 0),
        };
        let progress = ProgressSummary {
            chapter_id: Some(new_chapters[progress_index].id.clone()),
            chapter_index: progress_index,
            offset: progress_offset,
            updated_at_ms: now_ms().max(old_progress.updated_at_ms.saturating_add(1)),
        };
        let moved_progress = progress_index != old_progress.chapter_index
            || old_progress.chapter_id.as_deref() != progress.chapter_id.as_deref()
            || progress_offset != old_progress.offset;
        let mut next_private = old_private.clone();
        next_private["sourceId"] = json!(new_source.id);
        next_private["book"] = engine_book.clone();
        next_private["chapters"] = json!(raw_chapters);
        next_private["catalogGeneration"] = json!(generation);
        let mut next_book = old_book.clone();
        next_book["title"] = json!(display_metadata.title);
        next_book["author"] = json!(display_metadata.author.unwrap_or(target_author));
        if let Some(cover) = display_metadata.cover_src {
            next_book["coverSrc"] = json!(cover.as_str());
        }
        if let Some(intro) = display_metadata.intro {
            next_book["intro"] = json!(intro);
        }
        if let Some(kind) = display_metadata.kind {
            next_book["kind"] = json!(kind);
        }
        if let Some(word_count) = display_metadata.word_count {
            next_book["wordCount"] = json!(word_count);
        }
        next_book["sourceId"] = json!(display_metadata.source_id);
        next_book["sourceName"] = json!(display_metadata.source_name);
        set_optional_public_field(
            &mut next_book,
            "sourceGroup",
            display_metadata.source_group.map(|group| json!(group)),
        );
        next_book["chapterCount"] = json!(new_chapters.len());
        next_book["latestChapter"] = new_chapters
            .last()
            .map(|chapter| json!(chapter.title))
            .unwrap_or(Value::Null);
        next_book["chapters"] =
            serde_json::to_value(&new_chapters).map_err(|error| error.to_string())?;
        next_book["progress"] =
            serde_json::to_value(&progress).map_err(|error| error.to_string())?;
        let next_progress = serde_json::to_value(ProgressDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            book_id: book_id.to_owned(),
            chapter_id: progress.chapter_id.clone(),
            chapter_index: progress.chapter_index,
            offset: progress.offset,
            updated_at_ms: progress.updated_at_ms,
        })
        .map_err(|error| error.to_string())?;

        self.write_private_json(&private_path, &next_private)
            .await?;
        if let Err(error) = self.store.write_json_ref(&book_ref, &next_book).await {
            let rollback = self.write_private_json(&private_path, &old_private).await;
            return Err(format!(
                "Cannot commit replacement book resource: {error}; private rollback: {}",
                rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error)
            ));
        }
        if let Err(error) = self
            .store
            .write_json_ref(&progress_ref, &next_progress)
            .await
        {
            let book_rollback = self.store.write_json_ref(&book_ref, &old_book).await;
            let private_rollback = self.write_private_json(&private_path, &old_private).await;
            return Err(format!(
                "Cannot commit replacement progress: {error}; book rollback: {}; private rollback: {}",
                book_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error.to_string()),
                private_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error),
            ));
        }
        if let Err(error) = self.upsert_shelf(book_id).await {
            let progress_rollback = self
                .store
                .write_json_ref(
                    &progress_ref,
                    &serde_json::to_value(ProgressDocument {
                        schema_version: CURRENT_SCHEMA_VERSION,
                        book_id: book_id.to_owned(),
                        chapter_id: old_progress.chapter_id.clone(),
                        chapter_index: old_progress.chapter_index,
                        offset: old_progress.offset,
                        updated_at_ms: old_progress.updated_at_ms,
                    })
                    .map_err(|encode_error| encode_error.to_string())?,
                )
                .await;
            let book_rollback = self.store.write_json_ref(&book_ref, &old_book).await;
            let private_rollback = self.write_private_json(&private_path, &old_private).await;
            let shelf_rollback = self.upsert_shelf(book_id).await;
            return Err(format!(
                "Cannot update shelf after source replacement: {error}; progress rollback: {}; book rollback: {}; private rollback: {}; shelf rollback: {}",
                progress_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error.to_string()),
                book_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error.to_string()),
                private_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error),
                shelf_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error),
            ));
        }
        let bookmark_result = self
            .store
            .update_json_ref(&bookmarks_ref, |mut document| {
                let bookmarks = document
                    .get_mut("bookmarks")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| "Bookmark resource has no bookmarks array".to_owned())?;
                for bookmark in bookmarks.iter_mut().filter(|bookmark| {
                    bookmark.get("bookId").and_then(Value::as_str) == Some(book_id)
                }) {
                    let old_title =
                        if bookmark.get("orphaned").and_then(Value::as_bool) == Some(true) {
                            bookmark
                                .get("chapterTitle")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        } else {
                            bookmark
                                .get("chapterIndex")
                                .and_then(Value::as_u64)
                                .and_then(|index| old_titles.get(index as usize))
                                .cloned()
                        };
                    let target_index = old_title.as_deref().and_then(|title| {
                        if bookmark.get("orphaned").and_then(Value::as_bool) == Some(true) {
                            unique_target_title_index(title, &new_titles)
                        } else {
                            unique_chapter_title_index(title, &old_titles, &new_titles)
                        }
                    });
                    if let Some(index) = target_index {
                        bookmark["chapterIndex"] = json!(index);
                        bookmark.as_object_mut().map(|fields| {
                            fields.remove("orphaned");
                            fields.remove("chapterTitle");
                        });
                        migrated_bookmarks = migrated_bookmarks.saturating_add(1);
                    } else {
                        bookmark["orphaned"] = json!(true);
                        bookmark["chapterTitle"] =
                            json!(old_title.unwrap_or_else(|| "Unknown chapter".into()));
                        orphaned_bookmarks = orphaned_bookmarks.saturating_add(1);
                    }
                }
                Ok(document)
            })
            .await;
        if let Err(error) = bookmark_result {
            let old_progress_json = serde_json::to_value(ProgressDocument {
                schema_version: CURRENT_SCHEMA_VERSION,
                book_id: book_id.to_owned(),
                chapter_id: old_progress.chapter_id.clone(),
                chapter_index: old_progress.chapter_index,
                offset: old_progress.offset,
                updated_at_ms: old_progress.updated_at_ms,
            })
            .map_err(|encode_error| encode_error.to_string())?;
            let progress_rollback = self
                .store
                .write_json_ref(&progress_ref, &old_progress_json)
                .await;
            let book_rollback = self.store.write_json_ref(&book_ref, &old_book).await;
            let private_rollback = self.write_private_json(&private_path, &old_private).await;
            let shelf_rollback = self.upsert_shelf(book_id).await;
            return Err(format!(
                "Cannot migrate replacement bookmarks: {error}; progress rollback: {}; book rollback: {}; private rollback: {}; shelf rollback: {}",
                progress_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error.to_string()),
                book_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error.to_string()),
                private_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error),
                shelf_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error),
            ));
        }

        for chapter_id in old_chapter_ids {
            let path = self
                .root
                .join("books")
                .join(book_id)
                .join("chapters")
                .join(format!("{chapter_id}.html"));
            if let Err(error) = tokio::fs::remove_file(path).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("Cannot remove old-source chapter cache {chapter_id}: {error}");
                }
            }
        }
        let shelf_ref = self.store.shelf_ref();
        Ok(json!({
            "book": self.resource_descriptor(&book_ref),
            "shelf": self.resource_descriptor(&shelf_ref),
            "progress": serde_json::to_value(&progress).map_err(|error| error.to_string())?,
            "movedProgress": moved_progress,
            "bookmarks": {
                "resource": self.resource_descriptor(&bookmarks_ref),
                "migratedCount": migrated_bookmarks,
                "orphanedCount": orphaned_bookmarks,
            },
        }))
    }

    pub async fn start_chapter_download(
        &self,
        book_id: &str,
        from_index: usize,
        count: usize,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let book = self
            .store
            .read_json_ref(
                &self
                    .store
                    .book_ref(book_id)
                    .map_err(|error| error.to_string())?,
            )
            .await
            .map_err(|error| error.to_string())?;
        let chapters = book
            .get("chapterCount")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        if from_index >= chapters {
            return Err("Download starts outside the book chapter list".into());
        }
        if count == 0 {
            return Err("Download count must be greater than zero".into());
        }
        let total = count.min(chapters - from_index);
        let now = now_ms();
        let task = AppTask {
            id: format!("task-{}", uuid::Uuid::new_v4().simple()),
            kind: "chapterDownload".into(),
            status: "queued".into(),
            book_id: Some(book_id.to_owned()),
            source_ids: None,
            keyword: None,
            page: 1,
            from_index,
            total,
            completed: 0,
            check_only: false,
            search_id: None,
            result: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let task_id = task.id.clone();
        let response_task_id = task_id.clone();
        let receiver = self.create_task(task).await?;
        let service = self.clone();
        tokio::spawn(async move {
            service.run_task(task_id, receiver).await;
        });
        Ok(
            json!({ "taskId": response_task_id, "task": self.task_summary_by_id(&response_task_id).await?, "resource": self.task_resource_descriptor()? }),
        )
    }

    pub async fn start_catalog_refresh(
        &self,
        book_id: &str,
        check_only: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let _ = self
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await?;
        let now = now_ms();
        let task = AppTask {
            id: format!("task-{}", uuid::Uuid::new_v4().simple()),
            kind: if check_only {
                "checkNewChapters"
            } else {
                "refreshChapters"
            }
            .into(),
            status: "queued".into(),
            book_id: Some(book_id.to_owned()),
            source_ids: None,
            keyword: None,
            page: 1,
            from_index: 0,
            total: 1,
            completed: 0,
            check_only,
            search_id: None,
            result: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let task_id = task.id.clone();
        let response_task_id = task_id.clone();
        let receiver = self.create_task(task).await?;
        let service = self.clone();
        tokio::spawn(async move {
            service.run_task(task_id, receiver).await;
        });
        Ok(
            json!({ "taskId": response_task_id, "task": self.task_summary_by_id(&response_task_id).await?, "resource": self.task_resource_descriptor()? }),
        )
    }

    pub async fn list_tasks(&self) -> Result<Value, String> {
        Ok(json!({ "resource": self.task_resource_descriptor()? }))
    }

    pub async fn tasks_resource(&self) -> Result<Value, String> {
        self.list_tasks().await
    }

    pub async fn refresh_chapters(&self, book_id: &str) -> Result<Value, String> {
        self.start_catalog_refresh(book_id, false).await
    }

    pub async fn check_new_chapters(&self, book_id: &str) -> Result<Value, String> {
        self.start_catalog_refresh(book_id, true).await
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<Value, String> {
        self.control_task(task_id, "pause").await
    }

    pub async fn resume_task(&self, task_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let mut restart = None;
        let task = {
            let mut registry = self.tasks.lock().await;
            let position = registry
                .records
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            let signal = registry.signals.get(task_id).cloned();
            let (task, new_signal) = {
                let task = &mut registry.records[position];
                if matches!(
                    task.status.as_str(),
                    "completed" | "failed" | "cancelled" | "cancelling"
                ) {
                    return Err(format!("Task cannot resume from status '{}'", task.status));
                }
                if let Some(sender) = signal {
                    sender.send_modify(|state| state.paused = false);
                    task.status = "running".into();
                    task.updated_at_ms = now_ms();
                    (task.clone(), None)
                } else if matches!(task.status.as_str(), "paused" | "interrupted" | "queued") {
                    task.status = "queued".into();
                    task.updated_at_ms = now_ms();
                    let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
                    restart = Some(receiver);
                    (task.clone(), Some(sender))
                } else {
                    return Err(format!("Task cannot resume from status '{}'", task.status));
                }
            };
            if let Some(sender) = new_signal {
                registry.signals.insert(task_id.to_owned(), sender);
            }
            self.persist_tasks_locked(&registry).await?;
            task
        };
        self.notify_task(&task).await;
        if let Some(receiver) = restart {
            let service = self.clone();
            let id = task.id.clone();
            tokio::spawn(async move {
                service.run_task(id, receiver).await;
            });
        }
        Ok(json!({ "task": task_summary(&task), "resource": self.task_resource_descriptor()? }))
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<Value, String> {
        self.control_task(task_id, "cancel").await
    }

    async fn control_task(&self, task_id: &str, operation: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let task = {
            let mut registry = self.tasks.lock().await;
            let position = registry
                .records
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            let signal = registry.signals.get(task_id).cloned();
            let task = &mut registry.records[position];
            if matches!(
                task.status.as_str(),
                "completed" | "failed" | "cancelled" | "cancelling"
            ) {
                return Err(format!(
                    "Task cannot {operation} from status '{}'",
                    task.status
                ));
            }
            match operation {
                "pause" => {
                    if let Some(sender) = signal {
                        sender.send_modify(|state| state.paused = true);
                        if task.status == "running" || task.status == "queued" {
                            task.status = "pausing".into();
                        }
                    } else if task.status == "queued" {
                        task.status = "paused".into();
                    }
                }
                "cancel" => {
                    if let Some(sender) = signal {
                        sender.send_modify(|state| state.cancelled = true);
                        if !matches!(task.status.as_str(), "completed" | "failed" | "cancelled") {
                            task.status = "cancelling".into();
                        }
                    } else if !matches!(task.status.as_str(), "completed" | "failed" | "cancelled")
                    {
                        task.status = "cancelled".into();
                    }
                }
                _ => unreachable!(),
            }
            task.updated_at_ms = now_ms();
            let task = task.clone();
            self.persist_tasks_locked(&registry).await?;
            task
        };
        self.notify_task(&task).await;
        Ok(json!({ "task": task_summary(&task), "resource": self.task_resource_descriptor()? }))
    }

    async fn create_task(
        &self,
        task: AppTask,
    ) -> Result<tokio::sync::watch::Receiver<TaskSignal>, String> {
        let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
        {
            let mut registry = self.tasks.lock().await;
            registry.records.push(task.clone());
            registry.signals.insert(task.id.clone(), sender);
            self.persist_tasks_locked(&registry).await?;
        }
        self.notify_task(&task).await;
        Ok(receiver)
    }

    async fn persist_tasks_locked(&self, registry: &TaskRegistry) -> Result<(), String> {
        let reference = self
            .store
            .reading_ref("tasks")
            .map_err(|error| error.to_string())?;
        let value = json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "tasks": registry.records });
        self.store
            .write_json_ref(&reference, &value)
            .await
            .map_err(|error| error.to_string())
    }

    fn task_resource_descriptor(&self) -> Result<Value, String> {
        let resource = self
            .store
            .reading_ref("tasks")
            .map_err(|error| error.to_string())?;
        Ok(self.resource_descriptor(&resource))
    }

    async fn task_summary_by_id(&self, task_id: &str) -> Result<Value, String> {
        let registry = self.tasks.lock().await;
        let task = registry
            .records
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
        Ok(task_summary(task))
    }

    async fn notify_task(&self, task: &AppTask) {
        let event =
            json!({ "task": task_summary(task), "resource": self.task_resource_descriptor().ok() });
        if let Ok(notifier) = self.task_notifier.read() {
            if let Some(notifier) = notifier.as_ref() {
                notifier(event);
            }
        }
    }

    async fn update_task<F>(&self, task_id: &str, update: F) -> Result<AppTask, String>
    where
        F: FnOnce(&mut AppTask),
    {
        let task = {
            let mut registry = self.tasks.lock().await;
            let task = registry
                .records
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            update(task);
            task.updated_at_ms = now_ms();
            let task = task.clone();
            self.persist_tasks_locked(&registry).await?;
            task
        };
        self.notify_task(&task).await;
        Ok(task)
    }

    async fn run_task(
        &self,
        task_id: String,
        mut signal: tokio::sync::watch::Receiver<TaskSignal>,
    ) {
        let outcome = async {
            let operation = self.operation_gate.clone().read_owned().await;
            let task = self.task_summary_record(&task_id).await?;
            if let Err(error) = self
                .update_task(&task_id, |task| task.status = "running".into())
                .await
            {
                drop(operation);
                let message = format!("Cannot mark task running: {error}");
                let _ = self
                    .update_task(&task_id, |task| {
                        task.status = "failed".into();
                        task.error = Some(message.clone());
                    })
                    .await;
                return Err(message);
            }
            drop(operation);
            let result = match task.kind.as_str() {
                "chapterDownload" => self.run_download_task(&task_id, &mut signal).await,
                "search" | "bookSourceCandidates" => {
                    self.run_search_task(&task_id, &mut signal).await
                }
                "refreshChapters" | "checkNewChapters" => {
                    self.run_refresh_task(&task_id, &mut signal).await
                }
                _ => Err(format!("Unsupported task kind '{}'", task.kind)),
            };
            match result {
                Ok(Some(_)) => {
                    let _operation = self.operation_gate.clone().read_owned().await;
                    self.finish_task(&task_id, "completed", None).await
                }
                Ok(None) => Ok(()),
                Err(error) => {
                    let _operation = self.operation_gate.clone().read_owned().await;
                    self.finish_task(&task_id, "failed", Some(error)).await
                }
            }
        }
        .await;
        if let Err(error) = outcome {
            eprintln!("Task {task_id} stopped with an error: {error}");
        }
        let mut registry = self.tasks.lock().await;
        registry.signals.remove(&task_id);
    }

    async fn task_summary_record(&self, task_id: &str) -> Result<AppTask, String> {
        let registry = self.tasks.lock().await;
        registry
            .records
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| format!("Unknown task '{task_id}'"))
    }

    async fn finish_task(
        &self,
        task_id: &str,
        status: &str,
        error: Option<String>,
    ) -> Result<(), String> {
        self.update_task(task_id, |task| {
            task.status = status.to_owned();
            task.error = error;
        })
        .await?;
        Ok(())
    }

    async fn task_checkpoint(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<bool, String> {
        loop {
            let current = *signal.borrow_and_update();
            if current.cancelled {
                if let Ok(task) = self.task_summary_record(task_id).await {
                    if matches!(task.kind.as_str(), "search" | "bookSourceCandidates") {
                        let _operation = self.operation_gate.clone().read_owned().await;
                        self.finish_search_document(&task, true).await?;
                    }
                }
                let _operation = self.operation_gate.clone().read_owned().await;
                self.finish_task(task_id, "cancelled", None).await?;
                return Ok(false);
            }
            if current.paused {
                if self.task_summary_record(task_id).await?.status != "paused" {
                    let _operation = self.operation_gate.clone().read_owned().await;
                    self.update_task(task_id, |task| task.status = "paused".into())
                        .await?;
                }
                if signal.changed().await.is_err() {
                    return Ok(false);
                }
                continue;
            }
            if self.task_summary_record(task_id).await?.status != "running" {
                let _operation = self.operation_gate.clone().read_owned().await;
                self.update_task(task_id, |task| task.status = "running".into())
                    .await?;
            }
            return Ok(true);
        }
    }

    async fn run_download_task(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<Option<String>, String> {
        loop {
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let task = self.task_summary_record(task_id).await?;
            if task.completed >= task.total {
                return Ok(Some(String::new()));
            }
            let book_id = task
                .book_id
                .as_deref()
                .ok_or_else(|| "Download task has no bookId".to_owned())?;
            let index = task.from_index.saturating_add(task.completed);
            let permit = self
                .task_slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|error| error.to_string())?;
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let _operation = self.operation_gate.clone().read_owned().await;
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(_operation);
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            self.prepare_chapters_unlocked(book_id, index, 1).await?;
            self.update_task(task_id, |task| {
                task.completed = task.completed.saturating_add(1)
            })
            .await?;
            drop(permit);
        }
    }

    async fn run_search_task(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<Option<String>, String> {
        loop {
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let task = self.task_summary_record(task_id).await?;
            if task.completed >= task.total {
                let _operation = self.operation_gate.clone().read_owned().await;
                let control = *signal.borrow_and_update();
                if control.paused || control.cancelled {
                    drop(_operation);
                    if !self.task_checkpoint(task_id, signal).await? {
                        return Ok(None);
                    }
                    continue;
                }
                self.finish_search_document(&task, false).await?;
                return Ok(Some(String::new()));
            }
            let permit = self
                .task_slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|error| error.to_string())?;
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let _operation = self.operation_gate.clone().read_owned().await;
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(_operation);
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let sources = self.read_sources().await?;
            let source_id = task
                .source_ids
                .as_ref()
                .and_then(|ids| ids.get(task.completed))
                .ok_or_else(|| "Search task source list is incomplete".to_owned())?;
            let source = sources
                .into_iter()
                .find(|source| &source.id == source_id)
                .ok_or_else(|| format!("Search source '{source_id}' was removed"))?;
            let keyword = task
                .keyword
                .as_deref()
                .ok_or_else(|| "Search task has no keyword".to_owned())?;
            let result = self
                .executor
                .execute(engine_request(
                    "search",
                    &source.source,
                    Some(keyword.to_owned()),
                    Some(task.page as i32),
                    None,
                    None,
                    None,
                ))
                .await;
            drop(permit);
            drop(_operation);
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let _operation = self.operation_gate.clone().read_owned().await;
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(_operation);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            self.append_search_source(&task, &source, result).await?;
            self.update_task(task_id, |task| {
                task.completed = task.completed.saturating_add(1)
            })
            .await?;
        }
    }

    async fn append_search_source(
        &self,
        task: &AppTask,
        source: &SourceRecord,
        response: Result<Value, String>,
    ) -> Result<(), String> {
        let search_id = task
            .search_id
            .as_deref()
            .ok_or_else(|| "Search task has no search resource".to_owned())?;
        let reference = self
            .store
            .search_ref(search_id)
            .map_err(|error| error.to_string())?;
        let mut results = Vec::new();
        let mut errors = Vec::new();
        let replacement_context = if task.kind == "bookSourceCandidates" {
            let context_path =
                Path::new("search-results").join(format!("{search_id}.replacement.json"));
            Some(self.read_private_json(context_path).await?)
        } else {
            None
        };
        match response {
            Ok(value) => {
                let books = value
                    .get("books")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for book in books {
                    let replacement = if let Some(context) = &replacement_context {
                        let Some(requires_identity_confirmation) =
                            candidate_identity_match(context, &book)
                        else {
                            continue;
                        };
                        Some((context, requires_identity_confirmation))
                    } else {
                        None
                    };
                    let result_id = format!("result-{}", uuid::Uuid::new_v4().simple());
                    let source_fingerprint = source_definition_fingerprint(&source.source)?;
                    let private_result = if let Some((context, requires_confirmation)) = replacement
                    {
                        json!({
                            "sourceId": source.id,
                            "book": book,
                            "replacementContext": {
                                "targetBookId": context["targetBookId"],
                                "originalSourceId": context["originalSourceId"],
                                "originalSourcePresent": context["originalSourcePresent"],
                                "originalSourceFingerprint": context["originalSourceFingerprint"],
                                "originalSourceRevision": context["originalSourceRevision"],
                                "catalogFingerprint": context["catalogFingerprint"],
                                "catalogGeneration": context["catalogGeneration"],
                                "createdAtMs": context["createdAtMs"],
                                "expiresAtMs": context["expiresAtMs"],
                                "searchId": search_id,
                                "candidateSourceFingerprint": source_fingerprint,
                                "requiresIdentityConfirmation": requires_confirmation,
                            }
                        })
                    } else {
                        json!({ "sourceId": source.id, "book": book })
                    };
                    self.write_private_json(
                        Path::new("search-results").join(format!("{result_id}.json")),
                        &private_result,
                    )
                    .await?;
                    let mut projected = project_search_result(&result_id, source, &book);
                    if let Some((_, requires_confirmation)) = replacement {
                        projected["requiresIdentityConfirmation"] = json!(requires_confirmation);
                    }
                    results.push(projected);
                }
            }
            Err(error) => errors.push(
                json!({ "sourceId": source.id, "sourceName": source.name, "message": error }),
            ),
        }
        self.store
            .update_json_ref(&reference, move |mut document| {
                let target = document
                    .get_mut("results")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| "Search resource has no results array".to_owned())?;
                target.extend(results);
                let target_errors = document
                    .get_mut("errors")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| "Search resource has no errors array".to_owned())?;
                target_errors.extend(errors);
                Ok(document)
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn finish_search_document(&self, task: &AppTask, cancelled: bool) -> Result<(), String> {
        let search_id = task
            .search_id
            .as_deref()
            .ok_or_else(|| "Search task has no search resource".to_owned())?;
        let reference = self
            .store
            .search_ref(search_id)
            .map_err(|error| error.to_string())?;
        self.store
            .update_json_ref(&reference, move |mut document| {
                document["complete"] = json!(true);
                if cancelled {
                    document["cancelled"] = json!(true);
                }
                Ok(document)
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn run_refresh_task(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<Option<String>, String> {
        loop {
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let task = self.task_summary_record(task_id).await?;
            let book_id = task
                .book_id
                .as_deref()
                .ok_or_else(|| "Refresh task has no bookId".to_owned())?;
            let permit = self
                .task_slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|error| error.to_string())?;
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let _operation = self.operation_gate.clone().read_owned().await;
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(_operation);
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            if task.completed >= task.total {
                return Ok(Some(String::new()));
            }
            let result = self
                .refresh_catalog_unlocked(book_id, !task.check_only)
                .await?;
            self.update_task(task_id, |task| {
                task.completed = 1;
                task.result = Some(result);
            })
            .await?;
            drop(permit);
            return Ok(Some(String::new()));
        }
    }

    async fn refresh_catalog_unlocked(&self, book_id: &str, commit: bool) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;

        // Capture a coherent source/catalog snapshot while following the
        // global lock order (source metadata, then book). The source engine
        // call deliberately happens after both locks are released.
        let (source, engine_book, old_raw, old_chapters, catalog_generation) = {
            let _sources_lock = self.sources_lock.lock().await;
            let source_id = {
                let private = self.read_private_json(&private_path).await?;
                private["sourceId"]
                    .as_str()
                    .ok_or_else(|| "Private book is missing sourceId".to_owned())?
                    .to_owned()
            };
            let source = self
                .read_sources()
                .await?
                .into_iter()
                .find(|source| source.id == source_id)
                .ok_or_else(|| format!("Book source '{source_id}' is no longer imported"))?;
            let _book_lock = self.book_lock(book_id).await;
            let private = self.read_private_json(&private_path).await?;
            if private["sourceId"].as_str() != Some(source.id.as_str()) {
                return Err("Book source changed while refreshing its catalog".to_owned());
            }
            let engine_book = private
                .get("book")
                .cloned()
                .ok_or_else(|| "Private book is missing engine metadata".to_owned())?;
            let book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| "Book was removed while refreshing its catalog".to_owned())?;
            if book.get("id").and_then(Value::as_str) != Some(book_id) {
                return Err("Book resource ID does not match its catalog path".to_owned());
            }
            let raw = private
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| "Private book is missing its source chapter catalog".to_owned())?;
            let chapters = serde_json::from_value::<Vec<ChapterDescriptor>>(
                book.get("chapters")
                    .cloned()
                    .ok_or_else(|| "Book chapter directory is missing".to_owned())?,
            )
            .map_err(|error| format!("Cannot read processed chapter directory: {error}"))?;
            if raw.len() != chapters.len() {
                return Err("Book chapter directory is out of sync; keeping existing data".into());
            }
            let generation = private
                .get("catalogGeneration")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (source, engine_book, raw, chapters, generation)
        };

        let operation = if crate::rss::is_legacy_rss_source(&source.source) {
            "rssChapters"
        } else {
            "chapters"
        };
        let response = self
            .executor
            .execute(engine_request(
                operation,
                &source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?;
        let new_raw = response
            .as_array()
            .cloned()
            .ok_or_else(|| "Source engine returned an invalid chapter list".to_owned())?;

        // Reacquire locks in the same order and reject stale responses. A
        // delete or another refresh may finish while the source engine runs;
        // neither result is allowed to recreate or overwrite newer data.
        let (result, removed_ids) = {
            let _sources_lock = self.sources_lock.lock().await;
            let current_source = self
                .read_sources()
                .await?
                .into_iter()
                .find(|candidate| candidate.id == source.id)
                .ok_or_else(|| "Book source was removed while refreshing its catalog".to_owned())?;
            if current_source.source != source.source {
                return Err("Book source changed while refreshing its catalog".to_owned());
            }
            let _book_lock = self.book_lock(book_id).await;
            let current_private = self
                .read_private_json(&private_path)
                .await
                .map_err(|_| "Book was removed while refreshing its catalog".to_owned())?;
            if current_private["sourceId"].as_str() != Some(source.id.as_str())
                || current_private.get("book") != Some(&engine_book)
            {
                return Err("Book metadata changed while refreshing its catalog".to_owned());
            }
            let current_raw = current_private
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| "Private book is missing its source chapter catalog".to_owned())?;
            if current_raw != old_raw {
                return Err(
                    "A newer chapter catalog was committed; discard this stale refresh".into(),
                );
            }
            let mut current_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| "Book was removed while refreshing its catalog".to_owned())?;
            let current_chapters = serde_json::from_value::<Vec<ChapterDescriptor>>(
                current_book
                    .get("chapters")
                    .cloned()
                    .ok_or_else(|| "Book chapter directory is missing".to_owned())?,
            )
            .map_err(|error| format!("Cannot read processed chapter directory: {error}"))?;
            if current_chapters.len() != old_chapters.len()
                || current_chapters
                    .iter()
                    .zip(&old_chapters)
                    .any(|(current, old)| {
                        current.id != old.id
                            || current.index != old.index
                            || current.title != old.title
                    })
            {
                return Err(
                    "A newer chapter directory was committed; discard this stale refresh".into(),
                );
            }

            let summary_progress = serde_json::from_value::<ProgressSummary>(
                current_book
                    .get("progress")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .map_err(|error| format!("Cannot read saved book progress: {error}"))?;
            let progress_ref = self
                .store
                .progress_ref(book_id)
                .map_err(|error| error.to_string())?;
            let progress_document = self
                .store
                .read_json_ref(&progress_ref)
                .await
                .ok()
                .and_then(|value| serde_json::from_value::<ProgressDocument>(value).ok())
                .filter(|progress| progress.book_id == book_id);
            // save_progress writes the book summary before its mirror. On a
            // timestamp tie the summary therefore wins; a newer mirror wins
            // when recovering from an interrupted older write.
            let progress = progress_document
                .filter(|document| document.updated_at_ms > summary_progress.updated_at_ms)
                .map(|document| ProgressSummary {
                    chapter_id: document.chapter_id,
                    chapter_index: document.chapter_index,
                    offset: document.offset,
                    updated_at_ms: document.updated_at_ms,
                })
                .unwrap_or(summary_progress);
            let previous_progress = serde_json::to_value(ProgressDocument {
                schema_version: CURRENT_SCHEMA_VERSION,
                book_id: book_id.to_owned(),
                chapter_id: progress.chapter_id.clone(),
                chapter_index: progress.chapter_index,
                offset: progress.offset,
                updated_at_ms: progress.updated_at_ms,
            })
            .map_err(|error| error.to_string())?;
            let cached_ids = current_chapters
                .iter()
                .filter(|chapter| chapter.src.is_some())
                .filter(|chapter| {
                    self.root
                        .join("books")
                        .join(book_id)
                        .join("chapters")
                        .join(format!("{}.html", chapter.id))
                        .is_file()
                })
                .map(|chapter| chapter.id.clone())
                .collect::<HashSet<_>>();
            let refreshed_at_ms = now_ms().max(progress.updated_at_ms.saturating_add(1));
            let mut plan = crate::catalog::reconcile_catalog(
                book_id,
                &old_raw,
                &current_chapters,
                &new_raw,
                &progress,
                &cached_ids,
                refreshed_at_ms,
            )?;
            if let Some(generation) = catalog_generation.as_deref() {
                let old_ids = current_chapters
                    .iter()
                    .map(|chapter| chapter.id.as_str())
                    .collect::<HashSet<_>>();
                for (index, chapter) in plan.chapters.iter_mut().enumerate() {
                    if !old_ids.contains(chapter.id.as_str()) {
                        let stable_url = text_at(&new_raw[index], &["url", "chapterUrl"])
                            .unwrap_or_else(|| format!("@{index}"));
                        chapter.id = chapter_id_for_generation(book_id, generation, &stable_url);
                        chapter.src = None;
                    }
                }
                if let Some(chapter) = plan.chapters.get(plan.progress.chapter_index) {
                    plan.progress.chapter_id = Some(chapter.id.clone());
                }
            }
            if !commit {
                return Ok(json!({
                    "bookResourceId": book_ref.as_str(),
                    "addedCount": plan.added_count,
                    "matchedCount": plan.matched_count,
                    "movedProgress": plan.progress_relocated,
                    "progressRelocated": plan.progress_relocated,
                    "committed": false,
                }));
            }

            let original_book = current_book.clone();
            let mut next_private = current_private.clone();
            next_private["chapters"] = json!(new_raw);
            current_book["chapters"] =
                serde_json::to_value(&plan.chapters).map_err(|error| error.to_string())?;
            current_book["chapterCount"] = json!(plan.chapters.len());
            current_book["latestChapter"] = json!(plan.latest_chapter);
            current_book["progress"] =
                serde_json::to_value(&plan.progress).map_err(|error| error.to_string())?;
            let next_progress = ProgressDocument {
                schema_version: CURRENT_SCHEMA_VERSION,
                book_id: book_id.to_owned(),
                chapter_id: plan.progress.chapter_id.clone(),
                chapter_index: plan.progress.chapter_index,
                offset: plan.progress.offset,
                updated_at_ms: plan.progress.updated_at_ms,
            };
            let next_progress =
                serde_json::to_value(next_progress).map_err(|error| error.to_string())?;

            self.write_private_json(&private_path, &next_private)
                .await?;
            if let Err(error) = self.store.write_json_ref(&book_ref, &current_book).await {
                let rollback = self
                    .write_private_json(&private_path, &current_private)
                    .await;
                return Err(match rollback {
                    Ok(()) => format!("Cannot commit refreshed book resource: {error}"),
                    Err(rollback) => format!(
                        "Cannot commit refreshed book resource: {error}; private catalog rollback failed: {rollback}"
                    ),
                });
            }
            if let Err(error) = self
                .store
                .write_json_ref(&progress_ref, &next_progress)
                .await
            {
                let book_rollback = self.store.write_json_ref(&book_ref, &original_book).await;
                let private_rollback = self
                    .write_private_json(&private_path, &current_private)
                    .await;
                return Err(format!(
                    "Cannot commit refreshed progress mirror: {error}; book rollback: {}; private catalog rollback: {}",
                    book_rollback
                        .map(|_| "ok".to_owned())
                        .unwrap_or_else(|error| error.to_string()),
                    private_rollback
                        .map(|_| "ok".to_owned())
                        .unwrap_or_else(|error| error),
                ));
            }
            if let Err(error) = self.upsert_shelf(book_id).await {
                let progress_rollback = self
                    .store
                    .write_json_ref(&progress_ref, &previous_progress)
                    .await;
                let book_rollback = self.store.write_json_ref(&book_ref, &original_book).await;
                let private_rollback = self
                    .write_private_json(&private_path, &current_private)
                    .await;
                return Err(format!(
                    "Cannot update shelf after catalog refresh: {error}; progress rollback: {}; book rollback: {}; private catalog rollback: {}",
                    progress_rollback
                        .map(|_| "ok".to_owned())
                        .unwrap_or_else(|error| error.to_string()),
                    book_rollback
                        .map(|_| "ok".to_owned())
                        .unwrap_or_else(|error| error.to_string()),
                    private_rollback
                        .map(|_| "ok".to_owned())
                        .unwrap_or_else(|error| error),
                ));
            }
            let result = json!({
                "bookResourceId": book_ref.as_str(),
                "addedCount": plan.added_count,
                "matchedCount": plan.matched_count,
                "movedProgress": plan.progress_relocated,
                "progressRelocated": plan.progress_relocated,
                "committed": true,
            });
            (result, plan.removed_chapter_ids)
        };

        if !removed_ids.is_empty() {
            let _book_lock = self.book_lock(book_id).await;
            let current_ids = self
                .store
                .read_json_ref(&book_ref)
                .await
                .ok()
                .and_then(|book| {
                    serde_json::from_value::<Vec<ChapterDescriptor>>(book.get("chapters")?.clone())
                        .ok()
                })
                .map(|chapters| {
                    chapters
                        .into_iter()
                        .map(|chapter| chapter.id)
                        .collect::<HashSet<_>>()
                });
            let Some(current_ids) = current_ids else {
                eprintln!(
                    "Skipping obsolete chapter cache cleanup for {book_id}: current catalog is unavailable"
                );
                return Ok(result);
            };
            for chapter_id in removed_ids {
                // A later refresh can reintroduce an ID before cleanup starts.
                // Recheck under the book lock so that response cannot lose its
                // cached HTML to an earlier refresh's delayed cleanup.
                if current_ids.contains(&chapter_id) {
                    continue;
                }
                let path = self
                    .root
                    .join("books")
                    .join(book_id)
                    .join("chapters")
                    .join(format!("{chapter_id}.html"));
                if let Err(error) = tokio::fs::remove_file(path).await {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        eprintln!("Cannot remove obsolete cached chapter {chapter_id}: {error}");
                    }
                }
            }
        }
        Ok(result)
    }

    pub async fn create_backup(&self, destination: &Path) -> Result<(), String> {
        let _exclusive = self.operation_gate.clone().write_owned().await;
        crate::backup::create_backup(&self.store, destination).await
    }

    pub async fn restore_backup(&self, archive: &Path) -> Result<Value, String> {
        let _restore_serial = self.restore_serial.clone().lock_owned().await;
        self.restore_barrier.send_replace(true);
        let _admission = RestoreAdmissionGuard {
            barrier: self.restore_barrier.clone(),
        };
        let _admission_gate = self.admission_gate.clone().write_owned().await;
        self.cancel_active_tasks().await?;
        let _exclusive = self.operation_gate.clone().write_owned().await;
        crate::backup::restore_backup(&self.store, archive).await?;
        self.pending_pdf_imports.clear()?;
        self.server
            .reload_private_media_mappings()
            .await
            .map_err(|error| error.to_string())?;
        self.reload_tasks_after_restore().await?;
        self.bootstrap().await
    }

    async fn reload_tasks_after_restore(&self) -> Result<(), String> {
        let reference = self
            .store
            .reading_ref("tasks")
            .map_err(|error| error.to_string())?;
        let document = match self.store.read_json_ref(&reference).await {
            Ok(document) => document,
            Err(_) => json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "tasks": [] }),
        };
        let mut records = serde_json::from_value::<Vec<AppTask>>(
            document.get("tasks").cloned().unwrap_or_else(|| json!([])),
        )
        .map_err(|error| format!("Cannot load restored task history: {error}"))?;
        for task in &mut records {
            if matches!(
                task.status.as_str(),
                "queued" | "running" | "pausing" | "cancelling"
            ) {
                task.status = "interrupted".into();
                task.updated_at_ms = now_ms();
            }
            if matches!(task.kind.as_str(), "search" | "bookSourceCandidates")
                && task.search_id.as_deref().is_none_or(|id| {
                    !self
                        .root
                        .join("search")
                        .join(format!("{id}.json"))
                        .is_file()
                })
                && !matches!(task.status.as_str(), "failed" | "cancelled")
            {
                task.status = "failed".into();
                task.error = Some("Search result resources were not included in the backup".into());
                task.updated_at_ms = now_ms();
            }
            if task.kind == "bookSourceCandidates"
                && task.search_id.as_deref().is_none_or(|id| {
                    !self
                        .private_root
                        .join("search-results")
                        .join(format!("{id}.replacement.json"))
                        .is_file()
                })
                && !matches!(task.status.as_str(), "failed" | "cancelled")
            {
                task.status = "failed".into();
                task.error =
                    Some("Replacement candidate context is unavailable after restore".into());
                task.updated_at_ms = now_ms();
            }
        }
        self.store
            .write_json_ref(
                &reference,
                &json!({
                    "schemaVersion": CURRENT_SCHEMA_VERSION,
                    "tasks": records,
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
        let mut registry = self.tasks.lock().await;
        registry.records = records;
        registry.signals.clear();
        Ok(())
    }

    async fn cancel_active_tasks(&self) -> Result<(), String> {
        let changed = {
            let mut registry = self.tasks.lock().await;
            let signals = registry.signals.clone();
            let mut changed = Vec::new();
            for task in &mut registry.records {
                if matches!(task.status.as_str(), "completed" | "failed" | "cancelled") {
                    continue;
                }
                if let Some(sender) = signals.get(&task.id) {
                    sender.send_modify(|signal| signal.cancelled = true);
                    task.status = "cancelling".into();
                } else {
                    task.status = "cancelled".into();
                }
                task.updated_at_ms = now_ms();
                changed.push(task.clone());
            }
            self.persist_tasks_locked(&registry).await?;
            changed
        };
        for task in &changed {
            self.notify_task(task).await;
        }
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if self.tasks.lock().await.signals.is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| {
            "Timed out waiting for active tasks to stop; backup restore was not applied".to_owned()
        })
    }

    pub(crate) async fn store_processed_results(
        &self,
        keyword: &str,
        page: u32,
        source_results: Vec<(SourceRecord, Value)>,
        errors: Vec<Value>,
    ) -> Result<Value, String> {
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        self.store_processed_results_with_search_id(
            &search_id,
            keyword,
            page,
            source_results,
            errors,
        )
        .await
    }

    pub(crate) async fn store_processed_results_with_search_id(
        &self,
        search_id: &str,
        keyword: &str,
        page: u32,
        source_results: Vec<(SourceRecord, Value)>,
        errors: Vec<Value>,
    ) -> Result<Value, String> {
        let mut public_results = Vec::with_capacity(source_results.len());
        for (source, book) in source_results {
            let result_id = format!("result-{}", uuid::Uuid::new_v4().simple());
            self.write_private_json(
                Path::new("search-results").join(format!("{result_id}.json")),
                &json!({ "sourceId": source.id, "book": book }),
            )
            .await?;
            public_results.push(project_search_result(&result_id, &source, &book));
        }
        let resource = self
            .store
            .search_ref(search_id)
            .map_err(|error| error.to_string())?;
        let document = json!({
            "schemaVersion": CURRENT_SCHEMA_VERSION,
            "keyword": keyword,
            "page": page.max(1),
            "results": public_results,
            "errors": errors,
            "complete": true,
        });
        self.store
            .write_json_ref(&resource, &document)
            .await
            .map_err(|error| error.to_string())?;
        let descriptor = self.resource_descriptor(&resource);
        let count = document["results"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default();
        let errors = document["errors"].clone();
        Ok(json!({ "resource": descriptor, "bookCount": count, "errors": errors }))
    }

    pub(crate) async fn source_record(&self, source_id: &str) -> Result<SourceRecord, String> {
        self.find_source(source_id).await
    }

    /// Hold source metadata stable while an RSS request publishes its
    /// category/article/state resources. Removing or changing a subscription
    /// uses the same mutex, so a delayed response cannot recreate its state.
    pub(crate) async fn lock_rss_source_snapshot(
        &self,
        expected: &SourceRecord,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        let guard = self.sources_lock.clone().lock_owned().await;
        let current = self
            .read_sources()
            .await?
            .into_iter()
            .find(|source| source.id == expected.id)
            .ok_or_else(|| "RSS source was removed while the request was running".to_owned())?;
        if !current.enabled
            || current.source != expected.source
            || !crate::source_metadata::is_rss_source_metadata(&current.source)
        {
            return Err("RSS source changed while the request was running".to_owned());
        }
        Ok(guard)
    }

    pub(crate) async fn execute_source_operation(
        &self,
        source_id: &str,
        operation: &str,
        keyword: Option<String>,
        page: Option<u32>,
        book: Option<Value>,
        chapter: Option<Value>,
        next_chapter_url: Option<String>,
    ) -> Result<Value, String> {
        let source = self.find_source(source_id).await?;
        self.executor
            .execute(engine_request(
                operation,
                &source.source,
                keyword,
                page.map(|page| page as i32),
                book,
                chapter,
                next_chapter_url,
            ))
            .await
    }

    pub async fn add_book(&self, result_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(result_id, "resultId")?;
        let cache = self
            .read_private_json(Path::new("search-results").join(format!("{result_id}.json")))
            .await?;
        let source_id = cache
            .get("sourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| "Search result has no source ID".to_owned())?
            .to_owned();
        let raw_book = cache
            .get("book")
            .cloned()
            .ok_or_else(|| "Search result has no book".to_owned())?;
        let source = self.find_source(&source_id).await?;
        let url = text_at(&raw_book, &["bookUrl", "url", "origin"])
            .unwrap_or_else(|| result_id.to_owned());
        let book_id = format!("book-{:016x}", stable_hash(&format!("{source_id}\0{url}")));
        let _book_lock = self.book_lock(&book_id).await;
        let prior_book = self
            .store
            .read_json_ref(
                &self
                    .store
                    .book_ref(&book_id)
                    .map_err(|error| error.to_string())?,
            )
            .await
            .ok();
        let legacy_rss = crate::rss::is_legacy_rss_source(&source.source);
        let info = self
            .executor
            .execute(engine_request(
                if legacy_rss {
                    "rssBookInfo"
                } else {
                    "bookInfo"
                },
                &source.source,
                None,
                None,
                Some(raw_book.clone()),
                None,
                None,
            ))
            .await?;
        let engine_book = if info.is_object() {
            info
        } else {
            raw_book.clone()
        };
        let chapters = self
            .executor
            .execute(engine_request(
                if legacy_rss {
                    "rssChapters"
                } else {
                    "chapters"
                },
                &source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?;
        let raw_chapters = chapters.as_array().cloned().unwrap_or_default();
        // Reject ambiguous first-import catalogs too. The same pure planner
        // used by refresh catches duplicate canonical URLs and generated IDs
        // before any public book or private engine data is written.
        crate::catalog::reconcile_catalog(
            &book_id,
            &[],
            &[],
            &raw_chapters,
            &ProgressSummary::default(),
            &HashSet::new(),
            now_ms(),
        )
        .map_err(|error| format!("Cannot add book with invalid chapter catalog: {error}"))?;
        let display_metadata = crate::book_metadata::project_book_metadata(&engine_book, &source);
        let title = display_metadata.title.clone();
        let author = display_metadata.author.clone().unwrap_or_default();
        let latest = raw_chapters
            .last()
            .and_then(|chapter| text_at(chapter, &["title", "chapterName", "name"]));
        let prior_private = self
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .ok();
        let old_raw_chapters = prior_private
            .as_ref()
            .and_then(|book| book.get("chapters"))
            .and_then(Value::as_array);
        let old_chapters = prior_book
            .as_ref()
            .and_then(|book| book.get("chapters"))
            .and_then(Value::as_array);
        let old_by_url: HashMap<String, Value> = old_raw_chapters
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, raw)| {
                let url = text_at(raw, &["url", "chapterUrl"])?;
                Some((url, old_chapters?.get(index)?.clone()))
            })
            .collect();
        let descriptors = raw_chapters
            .iter()
            .enumerate()
            .map(|(index, chapter)| {
                let raw_url =
                    text_at(chapter, &["url", "chapterUrl"]).unwrap_or_else(|| index.to_string());
                let previous = old_by_url.get(&raw_url);
                let id = previous
                    .and_then(|old| old.get("id").and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_else(|| chapter_id(&book_id, &raw_url));
                let was_cached = previous
                    .filter(|old| old.get("id").and_then(Value::as_str) == Some(&id))
                    .and_then(|old| old.get("src"))
                    .filter(|src| !src.is_null())
                    .and_then(|_| Some(self.store.chapter_ref(&book_id, &id).ok()?))
                    .filter(|_| {
                        self.root
                            .join("books")
                            .join(&book_id)
                            .join("chapters")
                            .join(format!("{id}.html"))
                            .is_file()
                    });
                ChapterDescriptor {
                    id,
                    title: text_at(chapter, &["title", "chapterName", "name"])
                        .unwrap_or_else(|| format!("Chapter {}", index + 1)),
                    index,
                    src: was_cached,
                }
            })
            .collect::<Vec<_>>();
        let progress = prior_book
            .as_ref()
            .and_then(|book| serde_json::from_value(book.get("progress")?.clone()).ok())
            .unwrap_or_default();
        let book_instance_id = prior_private
            .as_ref()
            .and_then(|book| book.get("bookInstanceId"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let book = BookDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: book_id.clone(),
            title,
            can_change_source: true,
            author,
            cover_src: display_metadata.cover_src.clone(),
            intro: display_metadata.intro.clone(),
            kind: display_metadata.kind.clone(),
            word_count: display_metadata.word_count.clone(),
            source_id: Some(display_metadata.source_id.clone()),
            source_name: Some(display_metadata.source_name.clone()),
            source_group: display_metadata.source_group.clone(),
            chapter_count: descriptors.len(),
            latest_chapter: latest,
            progress,
            chapters: descriptors,
        };
        let book_json = serde_json::to_value(book).map_err(|error| error.to_string())?;
        let book_ref = self
            .store
            .book_ref(&book_id)
            .map_err(|error| error.to_string())?;
        self.store
            .write_json_ref(&book_ref, &book_json)
            .await
            .map_err(|error| error.to_string())?;
        self.write_private_json(
            Path::new("books").join(format!("{book_id}.json")),
            &json!({
                "sourceId": source_id,
                "bookInstanceId": book_instance_id,
                "book": engine_book,
                "chapters": raw_chapters
            }),
        )
        .await?;
        self.upsert_shelf(&book_id).await?;
        Ok(json!({
            "book": self.resource_descriptor(&book_ref),
            "shelf": self.resource_descriptor(&self.store.shelf_ref()),
        }))
    }

    pub async fn get_book(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let reference = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let _sources = self.sources_lock.lock().await;
        let sources = self.read_sources().await?;
        let _book = self.book_lock(book_id).await;
        let mut book = self
            .store
            .read_json_ref(&reference)
            .await
            .map_err(|error| error.to_string())?;
        let original_book = book.clone();
        let private = self
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .ok();
        let can_change_source = private.as_ref().is_some_and(can_change_source_from_private);
        if let Some(private) = private.as_ref() {
            if let (Some(source_id), Some(engine_book)) = (
                private.get("sourceId").and_then(Value::as_str),
                private.get("book").filter(|value| value.is_object()),
            ) {
                let source = sources.iter().find(|source| source.id == source_id);
                let fallback_source = SourceRecord {
                    id: source_id.to_owned(),
                    name: book
                        .get("sourceName")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    group: book
                        .get("sourceGroup")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    enabled: false,
                    source: Value::Null,
                };
                let metadata = crate::book_metadata::project_book_metadata(
                    engine_book,
                    source.unwrap_or(&fallback_source),
                );
                apply_cached_book_metadata(&mut book, &metadata, source.is_some());
            }
        }
        if book.get("canChangeSource").and_then(Value::as_bool) != Some(can_change_source) {
            book["canChangeSource"] = json!(can_change_source);
        }
        if book != original_book {
            self.store
                .write_json_ref(&reference, &book)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(self.resource_descriptor(&reference))
    }

    /// Refresh a book's processed details through the configured source
    /// engine. Source and book locks are held only while capturing and
    /// validating snapshots; the KMP/network call runs without either lock.
    pub async fn refresh_book_info(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;

        let (
            source,
            source_revision,
            book_instance_id,
            catalog_generation,
            engine_book,
            raw_chapters,
            public_latest_chapter,
        ) = {
            let _sources = self.sources_lock.lock().await;
            let sources = self.read_sources().await?;
            let first_private = self.read_private_json(&private_path).await?;
            let source_id = first_private
                .get("sourceId")
                .and_then(Value::as_str)
                .filter(|source_id| !source_id.trim().is_empty())
                .ok_or_else(|| "This book is not linked to an online source".to_owned())?;
            let source = sources
                .iter()
                .find(|source| source.id == source_id)
                .cloned()
                .ok_or_else(|| format!("Book source '{source_id}' is no longer imported"))?;
            if !source.enabled {
                return Err("Book source is disabled; enable it before refreshing details".into());
            }
            let source_revision = self.source_revision(&source.id).await?;

            let _book = self.book_lock(book_id).await;
            let private = self
                .read_private_json(&private_path)
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            if private.get("sourceId").and_then(Value::as_str) != Some(source.id.as_str()) {
                return Err("Book source changed before metadata refresh; retry".into());
            }
            if !can_change_source_from_private(&private) {
                return Err("This book has no refreshable online source metadata".into());
            }
            let book_instance_id = private
                .get("bookInstanceId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Private book is missing its instance identity".to_owned())?
                .to_owned();
            let engine_book = private
                .get("book")
                .filter(|value| value.is_object())
                .cloned()
                .ok_or_else(|| "Private book is missing processed engine metadata".to_owned())?;
            let raw_chapters = private
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| "Private book is missing its source chapter catalog".to_owned())?;
            let catalog_generation = private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null);
            let public_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            if public_book.get("id").and_then(Value::as_str) != Some(book_id) {
                return Err("Book resource ID does not match its catalog path".into());
            }
            (
                source,
                source_revision,
                book_instance_id,
                catalog_generation,
                engine_book,
                raw_chapters,
                public_book.get("latestChapter").cloned(),
            )
        };

        let operation = if crate::rss::is_legacy_rss_source(&source.source) {
            "rssBookInfo"
        } else {
            "bookInfo"
        };
        let response = self
            .executor
            .execute(engine_request(
                operation,
                &source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?;
        let refreshed_detail_keys = [
            "name",
            "title",
            "author",
            "coverUrl",
            "cover",
            "coverSrc",
            "intro",
            "introduction",
            "kind",
            "category",
            "wordCount",
            "word_count",
            "lastChapter",
            "latestChapter",
            "latestChapterTitle",
        ];
        let response_fields = response
            .as_object()
            .ok_or_else(|| "Source engine returned invalid book details".to_owned())?;
        let response_metadata = crate::book_metadata::project_book_metadata(&response, &source);
        let has_usable_detail = response_metadata.title != "Untitled"
            || response_metadata.author.is_some()
            || response_metadata.cover_src.is_some()
            || response_metadata.intro.is_some()
            || response_metadata.kind.is_some()
            || response_metadata.word_count.is_some()
            || response_metadata.latest_chapter.is_some();
        if !has_usable_detail {
            return Err("Source engine returned no refreshable book details".into());
        }
        let mut refreshed_fields = engine_book
            .as_object()
            .cloned()
            .ok_or_else(|| "Private book engine metadata is invalid".to_owned())?;
        // Keep the full prior engine object as the base so partial info
        // responses do not discard source-specific navigation/context data.
        // Catalog identity is owned by the existing book and cannot be changed
        // by a metadata refresh; a new URL requires an explicit catalog flow.
        const CATALOG_IDENTITY_KEYS: [&str; 5] = ["bookUrl", "url", "origin", "articleId", "id"];
        for (key, value) in response_fields {
            if CATALOG_IDENTITY_KEYS.contains(&key.as_str()) || value.is_null() {
                continue;
            }
            if refreshed_detail_keys.contains(&key.as_str()) {
                let valid = match key.as_str() {
                    "name" | "title" | "author" | "coverUrl" | "cover" | "coverSrc" | "intro"
                    | "introduction" | "lastChapter" | "latestChapter" | "latestChapterTitle" => {
                        value.as_str().is_some_and(|text| !text.trim().is_empty())
                    }
                    "kind" | "category" => {
                        value.as_str().is_some_and(|text| !text.trim().is_empty())
                            || value.as_array().is_some_and(|values| {
                                values.iter().any(|value| {
                                    value.as_str().is_some_and(|text| !text.trim().is_empty())
                                })
                            })
                    }
                    "wordCount" | "word_count" => {
                        value.as_number().is_some()
                            || value.as_str().is_some_and(|text| !text.trim().is_empty())
                    }
                    _ => true,
                };
                if !valid {
                    continue;
                }
            }
            refreshed_fields.insert(key.clone(), value.clone());
        }
        let mut refreshed_engine_book = Value::Object(refreshed_fields);

        let _sources = self.sources_lock.lock().await;
        let current_source = self
            .read_sources()
            .await?
            .into_iter()
            .find(|candidate| candidate.id == source.id)
            .ok_or_else(|| "Book source was removed during metadata refresh".to_owned())?;
        if !current_source.enabled {
            return Err("Book source was disabled during metadata refresh".into());
        }
        if self.source_revision(&source.id).await? != source_revision
            || current_source.source != source.source
        {
            return Err("Book source changed during metadata refresh; refresh again".into());
        }

        let _book = self.book_lock(book_id).await;
        let old_private = self
            .read_private_json(&private_path)
            .await
            .map_err(|_| format!("Book '{book_id}' was removed during metadata refresh"))?;
        if old_private.get("sourceId").and_then(Value::as_str) != Some(source.id.as_str())
            || old_private.get("bookInstanceId").and_then(Value::as_str)
                != Some(book_instance_id.as_str())
            || old_private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null)
                != catalog_generation
            || old_private.get("book") != Some(&engine_book)
        {
            return Err("Book changed during metadata refresh; refresh again".into());
        }
        let old_book = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|_| format!("Book '{book_id}' was removed during metadata refresh"))?;
        if old_book.get("id").and_then(Value::as_str) != Some(book_id) {
            return Err("Book resource ID does not match its catalog path".into());
        }
        let catalog_changed_while_refreshing = old_private
            .get("chapters")
            .and_then(Value::as_array)
            .is_none_or(|chapters| chapters != &raw_chapters)
            || old_book.get("latestChapter").cloned() != public_latest_chapter;
        if catalog_changed_while_refreshing {
            if let Some(refreshed_fields) = refreshed_engine_book.as_object_mut() {
                for key in ["lastChapter", "latestChapter", "latestChapterTitle"] {
                    refreshed_fields.remove(key);
                }
            }
        }
        let display_metadata =
            crate::book_metadata::project_book_metadata(&refreshed_engine_book, &current_source);

        let mut next_private = old_private.clone();
        next_private["book"] = refreshed_engine_book;
        let mut next_book = old_book.clone();
        apply_refreshed_book_metadata(&mut next_book, &display_metadata);
        let shelf_ref = self.store.shelf_ref();

        self.write_private_json(&private_path, &next_private)
            .await?;
        if let Err(error) = self.store.write_json_ref(&book_ref, &next_book).await {
            let rollback = self.write_private_json(&private_path, &old_private).await;
            return Err(format!(
                "Cannot save refreshed book details: {error}; private rollback: {}",
                rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error)
            ));
        }
        if let Err(error) = self.upsert_shelf(book_id).await {
            let book_rollback = self.store.write_json_ref(&book_ref, &old_book).await;
            let private_rollback = self.write_private_json(&private_path, &old_private).await;
            return Err(format!(
                "Cannot update shelf after metadata refresh: {error}; book rollback: {}; private rollback: {}",
                book_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error.to_string()),
                private_rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error),
            ));
        }

        Ok(json!({
            "book": self.resource_descriptor(&book_ref),
            "shelf": self.resource_descriptor(&shelf_ref),
        }))
    }

    pub async fn prepare_chapters(
        &self,
        book_id: &str,
        from_index: usize,
        count: usize,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        self.prepare_chapters_unlocked(book_id, from_index, count)
            .await
    }

    async fn prepare_chapters_unlocked(
        &self,
        book_id: &str,
        from_index: usize,
        count: usize,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let count = count.clamp(1, 50);
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let (source_id, engine_book, target_chapter_ids) = {
            let _book_lock = self.book_lock(book_id).await;
            let private = self
                .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                .await?;
            let source_id = private["sourceId"]
                .as_str()
                .ok_or_else(|| "Book source ID is missing".to_owned())?
                .to_owned();
            let engine_book = private
                .get("book")
                .cloned()
                .ok_or_else(|| "Book engine data is missing".to_owned())?;
            let public_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|error| error.to_string())?;
            let raw_chapters = private
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
            let descriptors = public_book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            if raw_chapters.len() != descriptors.len() {
                return Err(
                    "Book chapter directory is out of sync; refresh it before reading".into(),
                );
            }
            let end = from_index.saturating_add(count).min(raw_chapters.len());
            let target_chapter_ids = descriptors
                .iter()
                .skip(from_index.min(end))
                .take(end.saturating_sub(from_index.min(end)))
                .map(|chapter| {
                    chapter["id"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "Chapter ID missing".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            (source_id, engine_book, target_chapter_ids)
        };
        let mut prepared = 0usize;
        let defaults = reader_defaults(
            self.store
                .read_json_ref(&self.store.settings_ref())
                .await
                .ok(),
        );
        for chapter_id in target_chapter_ids {
            // Requests for the same uncached chapter share this lock. It stays
            // held over the source call while the book lock remains available
            // to progress saves and unrelated chapter cache commits.
            let _chapter_lock = self.chapter_lock(book_id, &chapter_id).await;
            let (raw_chapter, next_chapter_url, already_ready) = {
                let _book_lock = self.book_lock(book_id).await;
                let private = self
                    .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                    .await?;
                if private["sourceId"].as_str() != Some(source_id.as_str()) {
                    return Err("Book source changed while preparing a chapter".to_owned());
                }
                if private.get("book") != Some(&engine_book) {
                    return Err("Book metadata changed while preparing a chapter".to_owned());
                }
                let public_book = self
                    .store
                    .read_json_ref(&book_ref)
                    .await
                    .map_err(|_| "Book was removed while preparing a chapter".to_owned())?;
                let descriptors = public_book
                    .get("chapters")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
                let index = descriptors
                    .iter()
                    .position(|chapter| chapter["id"].as_str() == Some(chapter_id.as_str()))
                    .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
                let raw_chapters = private
                    .get("chapters")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
                let raw_chapter = raw_chapters
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
                let next_chapter_url = raw_chapters
                    .get(index + 1)
                    .and_then(|chapter| text_at(chapter, &["url", "chapterUrl"]));
                let chapter_path = self
                    .root
                    .join("books")
                    .join(book_id)
                    .join("chapters")
                    .join(format!("{chapter_id}.html"));
                let already_ready = tokio::fs::metadata(&chapter_path).await.is_ok();
                if already_ready {
                    let chapter_ref = self
                        .store
                        .chapter_ref(book_id, &chapter_id)
                        .map_err(|error| error.to_string())?;
                    if descriptors[index]["src"].as_str() != Some(chapter_ref.as_str()) {
                        let mut current = public_book;
                        current["chapters"][index]["src"] = json!(chapter_ref.as_str());
                        self.store
                            .write_json_ref(&book_ref, &current)
                            .await
                            .map_err(|error| error.to_string())?;
                    }
                }
                (raw_chapter, next_chapter_url, already_ready)
            };
            if already_ready {
                continue;
            }

            // Clone the current source metadata for this request. Before
            // publishing the result, verify that the source has not been
            // replaced while the executor was working.
            let source = self.find_source(&source_id).await?;
            let content = self
                .executor
                .execute(engine_request(
                    if crate::rss::is_legacy_rss_source(&source.source) {
                        "rssContent"
                    } else {
                        "content"
                    },
                    &source.source,
                    None,
                    None,
                    Some(engine_book.clone()),
                    Some(raw_chapter.clone()),
                    next_chapter_url,
                ))
                .await?;
            let content = content
                .as_str()
                .ok_or_else(|| "Source engine returned non-text chapter content".to_owned())?;
            let _sources_lock = self.sources_lock.lock().await;
            let current_source = self
                .read_sources()
                .await?
                .into_iter()
                .find(|candidate| candidate.id == source_id)
                .ok_or_else(|| "Book source was removed while preparing a chapter".to_owned())?;
            if current_source.source != source.source {
                return Err(
                    "Book source changed while preparing a chapter; retry the request".into(),
                );
            }

            let _book_lock = self.book_lock(book_id).await;
            let current_private = self
                .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                .await
                .map_err(|_| "Book was removed while preparing a chapter".to_owned())?;
            if current_private["sourceId"].as_str() != Some(source_id.as_str()) {
                return Err("Book source changed while preparing a chapter".to_owned());
            }
            if current_private.get("book") != Some(&engine_book) {
                return Err("Book metadata changed while preparing a chapter".to_owned());
            }
            let mut current_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| "Book was removed while preparing a chapter".to_owned())?;
            let descriptors = current_book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            let index = descriptors
                .iter()
                .position(|chapter| chapter["id"].as_str() == Some(chapter_id.as_str()))
                .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
            let current_raw_chapters = current_private
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
            let current_raw = current_raw_chapters
                .get(index)
                .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
            if current_raw != &raw_chapter {
                return Err("Book catalog changed while preparing a chapter".to_owned());
            }

            let chapter_path = self
                .root
                .join("books")
                .join(book_id)
                .join("chapters")
                .join(format!("{chapter_id}.html"));
            let reference = if tokio::fs::metadata(&chapter_path).await.is_ok() {
                self.store
                    .chapter_ref(book_id, &chapter_id)
                    .map_err(|error| error.to_string())?
            } else if looks_like_html(content) {
                self.store
                    .write_chapter_html(book_id, &chapter_id, content, &defaults)
                    .await
                    .map_err(|error| error.to_string())?
            } else {
                self.store
                    .write_chapter_text(book_id, &chapter_id, content, &defaults)
                    .await
                    .map_err(|error| error.to_string())?
            };
            if let Some(chapter) = current_book
                .get_mut("chapters")
                .and_then(Value::as_array_mut)
                .and_then(|chapters| chapters.get_mut(index))
            {
                chapter["src"] = json!(reference.as_str());
            }
            self.store
                .write_json_ref(&book_ref, &current_book)
                .await
                .map_err(|error| error.to_string())?;
            prepared += 1;
        }
        Ok(
            json!({ "book": self.resource_descriptor(&book_ref), "prepared": prepared, "bookId": book_id, "fromIndex": from_index }),
        )
    }

    pub async fn remove_book(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let _book_lock = self.book_lock(book_id).await;
        self.store
            .remove_book_resources(book_id)
            .await
            .map_err(|error| error.to_string())?;
        crate::reading_tools::remove_bookmarks_for_book(&self.store, book_id).await?;
        let _ = tokio::fs::remove_file(
            self.private_root
                .join("books")
                .join(format!("{book_id}.json")),
        )
        .await;
        let shelf_ref = self.store.shelf_ref();
        self.store
            .update_json_ref(&shelf_ref, |mut shelf| {
                let books = shelf
                    .get_mut("books")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| "Invalid shelf JSON: books must be an array".to_owned())?;
                books.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(book_id));
                crate::reading_tools::apply_shelf_sort(&mut shelf)?;
                Ok(shelf)
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({ "shelf": self.resource_descriptor(&self.store.shelf_ref()) }))
    }

    pub async fn save_progress(&self, book_id: &str, progress: Value) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let _book_lock = self.book_lock(book_id).await;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let chapter_id = match progress.get("chapterId") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => return Err("Progress chapterId must be a string or null".into()),
        };
        let chapter_index = strict_unsigned_field(&progress, "chapterIndex")?
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| "Progress chapterIndex must be a non-negative integer".to_owned())?;
        let offset = strict_unsigned_field(&progress, "offset")?
            .ok_or_else(|| "Progress offset must be a non-negative integer".to_owned())?;
        let updated_at_ms = strict_unsigned_field(&progress, "updatedAtMs")?.unwrap_or_else(now_ms);
        if let Some(chapter_id) = chapter_id.as_deref() {
            validate_id(chapter_id, "chapterId")?;
        }
        let progress_summary = json!({
            "chapterId": chapter_id,
            "chapterIndex": chapter_index,
            "offset": offset,
            "updatedAtMs": updated_at_ms,
        });
        let chapter_id_for_book = chapter_id.clone();
        self.store
            .update_json_ref(&book_ref, move |mut book| {
                let chapter_count = book
                    .get("chapterCount")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize;
                if chapter_index >= chapter_count {
                    return Err("Progress chapterIndex is outside the book chapter list".into());
                }
                if let Some(chapter_id) = chapter_id_for_book.as_deref() {
                    let belongs_to_book = book
                        .get("chapters")
                        .and_then(Value::as_array)
                        .and_then(|chapters| chapters.get(chapter_index))
                        .and_then(|chapter| chapter.get("id"))
                        .and_then(Value::as_str)
                        == Some(chapter_id);
                    if !belongs_to_book {
                        return Err("Progress chapterId does not match chapterIndex".into());
                    }
                }
                book["progress"] = progress_summary;
                Ok(book)
            })
            .await
            .map_err(|error| error.to_string())?;
        let record = ProgressDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            book_id: book_id.to_owned(),
            chapter_id,
            chapter_index,
            offset,
            updated_at_ms,
        };
        let progress_ref = self
            .store
            .progress_ref(book_id)
            .map_err(|error| error.to_string())?;
        let value = serde_json::to_value(record).map_err(|error| error.to_string())?;
        self.store
            .write_json_ref(&progress_ref, &value)
            .await
            .map_err(|error| error.to_string())?;
        self.upsert_shelf(book_id).await?;
        Ok(self.resource_descriptor(&book_ref))
    }

    pub async fn save_settings(&self, settings: Value) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let mut document = if settings.get("reader").is_some() {
            settings
        } else {
            json!({ "reader": settings })
        };
        if !document.is_object() {
            return Err("Settings must be a JSON object".into());
        }
        let reader = document
            .get("reader")
            .and_then(Value::as_object)
            .ok_or_else(|| "Settings reader must be an object".to_owned())?;
        if reader.contains_key("fontSizePx") {
            validate_number_range(reader, "fontSizePx", 12.0, 36.0)?;
        } else {
            // Accept the pre-canonical field when reading older settings, then
            // write only `fontSizePx` below.
            validate_number_range(reader, "fontSize", 12.0, 36.0)?;
        }
        validate_number_range(reader, "lineHeight", 1.2, 2.8)?;
        validate_integer_range(reader, "preloadCount", 1, 20)?;
        if let Some(theme) = reader.get("theme") {
            match theme.as_str() {
                Some("paper" | "sepia" | "dark" | "system" | "light") => {}
                _ => return Err("Settings theme must be paper, sepia, dark, or system".into()),
            }
        }
        let reader = document
            .get_mut("reader")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "Settings reader must be an object".to_owned())?;
        if !reader.contains_key("fontSizePx") {
            if let Some(legacy_size) = reader.remove("fontSize") {
                reader.insert("fontSizePx".to_owned(), legacy_size);
            }
        } else {
            reader.remove("fontSize");
        }
        if reader.get("theme").and_then(Value::as_str) == Some("light") {
            reader.insert("theme".to_owned(), json!("system"));
        }
        // Replacement rules have their own resource and command surface. Keep
        // this legacy settings field empty so there is only one stored copy.
        reader.insert("replacements".to_owned(), json!([]));
        document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
        self.store
            .write_json_ref(&self.store.settings_ref(), &document)
            .await
            .map_err(|error| error.to_string())?;
        Ok(self.resource_descriptor(&self.store.settings_ref()))
    }

    pub async fn import_local_book(
        &self,
        selected_path: &Path,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        self.import_local_book_unlocked(selected_path, options)
            .await
    }

    async fn import_local_book_unlocked(
        &self,
        selected_path: &Path,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<Value, String> {
        let defaults = reader_defaults(
            self.store
                .read_json_ref(&self.store.settings_ref())
                .await
                .ok(),
        );
        let book =
            crate::local_books::import_local_book(&self.store, selected_path, &defaults, &options)
                .await?;
        self.upsert_shelf(&book.id).await?;
        let book_ref = self
            .store
            .book_ref(&book.id)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "book": self.resource_descriptor(&book_ref),
            "shelf": self.resource_descriptor(&self.store.shelf_ref()),
        }))
    }

    /// Import a selected local file using the same recoverable encrypted-PDF
    /// challenge registry that the native picker commands use. The method is
    /// also available to headless browser harnesses, which supply a fixture
    /// path instead of opening a native picker.
    pub async fn import_local_book_with_challenge(
        &self,
        picked_file: PickerFile,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let pdf_password_supplied = options.pdf_password.is_some();
        let is_pdf = picked_file
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
        match self
            .import_local_book_unlocked(&picked_file.path, options)
            .await
        {
            Ok(result) => Ok(result),
            Err(error)
                if is_pdf && !pdf_password_supplied && error.contains("provide pdfPassword") =>
            {
                let import_token = self
                    .pending_pdf_imports
                    .insert_at(picked_file, Instant::now())?;
                schedule_pending_pdf_cleanup(
                    self.pending_pdf_imports.clone(),
                    import_token.clone(),
                );
                Ok(json!({ "passwordRequired": true, "importToken": import_token }))
            }
            Err(error) => Err(error),
        }
    }

    /// Retry a pending protected PDF import. Recoverable errors put the same
    /// temporary file back in the registry, preserving its original expiry.
    pub async fn retry_pending_pdf_import(
        &self,
        import_token: &str,
        password: String,
    ) -> Result<Value, String> {
        validate_id(import_token, "importToken")?;
        let _operation = self.operation_read().await;
        let Some(pending) = self
            .pending_pdf_imports
            .take_at(import_token, Instant::now())?
        else {
            return Err("PDF import expired; select the file again".to_owned());
        };
        if !tokio::fs::try_exists(&pending.file.path)
            .await
            .unwrap_or(false)
        {
            return Err(
                "PDF import file expired after app data was restored; select the file again"
                    .to_owned(),
            );
        }
        let options = crate::local_books::LocalImportOptions {
            pdf_password: Some(password),
            ..Default::default()
        };
        match self
            .import_local_book_unlocked(&pending.file.path, options)
            .await
        {
            Ok(result) => Ok(result),
            Err(error) => {
                self.pending_pdf_imports.reinsert_at(
                    import_token.to_owned(),
                    pending,
                    Instant::now(),
                )?;
                Err(error)
            }
        }
    }

    pub async fn cancel_pending_pdf_import(&self, import_token: &str) -> Result<bool, String> {
        validate_id(import_token, "importToken")?;
        let _operation = self.operation_read().await;
        self.pending_pdf_imports
            .cancel_at(import_token, Instant::now())
    }

    async fn source_metadata(&self) -> Result<Vec<SourceMetadata>, String> {
        Ok(metadata(&self.read_sources().await?))
    }

    async fn find_source(&self, id: &str) -> Result<SourceRecord, String> {
        self.read_sources()
            .await?
            .into_iter()
            .find(|source| source.id == id)
            .ok_or_else(|| format!("Book source '{id}' is no longer imported"))
    }

    async fn read_sources(&self) -> Result<Vec<SourceRecord>, String> {
        let path = self.private_root.join("sources.json");
        match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("Cannot read private source data: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("Cannot read private source data: {error}")),
        }
    }

    async fn source_revision(&self, source_id: &str) -> Result<u64, String> {
        let path = self.private_root.join("source-revisions.json");
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(format!("Cannot read source revisions: {error}")),
        };
        let document: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Cannot parse source revisions: {error}"))?;
        let revisions = document
            .get("revisions")
            .and_then(Value::as_object)
            .ok_or_else(|| "Source revision document is invalid".to_owned())?;
        match revisions.get(source_id) {
            None => Ok(0),
            Some(value) => value
                .as_u64()
                .ok_or_else(|| "Source revision value is invalid".to_owned()),
        }
    }

    async fn write_sources(&self, sources: &[SourceRecord]) -> Result<(), String> {
        let previous = self.read_sources().await?;
        let previous_by_id = previous
            .iter()
            .map(|record| {
                serde_json::to_value(record)
                    .map(|value| (record.id.as_str(), value))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let next_by_id = sources
            .iter()
            .map(|record| {
                serde_json::to_value(record)
                    .map(|value| (record.id.as_str(), value))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let mut revisions = self.read_source_revisions().await?;
        let changed = previous_by_id
            .keys()
            .chain(next_by_id.keys())
            .copied()
            .collect::<HashSet<_>>();
        for source_id in changed {
            if previous_by_id.get(source_id) != next_by_id.get(source_id) {
                let revision = revisions.entry(source_id.to_owned()).or_default();
                *revision = revision.saturating_add(1);
            }
        }
        self.write_private_json(
            Path::new("source-revisions.json"),
            &json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "revisions": revisions }),
        )
        .await?;
        self.write_private_json(Path::new("sources.json"), &json!(sources))
            .await
    }

    async fn read_source_revisions(&self) -> Result<HashMap<String, u64>, String> {
        let path = self.private_root.join("source-revisions.json");
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(error) => return Err(format!("Cannot read source revisions: {error}")),
        };
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RevisionsDocument {
            schema_version: u32,
            revisions: HashMap<String, u64>,
        }
        let document: RevisionsDocument = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Cannot parse source revisions: {error}"))?;
        if document.schema_version != CURRENT_SCHEMA_VERSION {
            return Err("Unsupported source revision schema".into());
        }
        Ok(document.revisions)
    }

    pub(crate) async fn read_private_json(
        &self,
        relative: impl AsRef<Path>,
    ) -> Result<Value, String> {
        let path = self.private_root.join(relative);
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| format!("Cannot read private app data: {error}"))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("Cannot parse private app data: {error}"))
    }

    pub(crate) async fn write_private_json(
        &self,
        relative: impl AsRef<Path>,
        value: &Value,
    ) -> Result<(), String> {
        let path = self.private_root.join(relative);
        let parent = path
            .parent()
            .ok_or_else(|| "Private app data path has no parent".to_owned())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Cannot create private app data directory: {error}"))?;
        let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
        let root = self.private_root.clone();
        let output = path.clone();
        tokio::task::spawn_blocking(move || {
            if !output.starts_with(root) {
                return Err("Private app data path escaped its root".to_owned());
            }
            atomicwrites::AtomicFile::new(&output, atomicwrites::AllowOverwrite)
                .write(|file| file.write_all(&bytes))
                .map_err(|error| format!("Cannot commit private app data: {error}"))?;
            Ok(())
        })
        .await
        .map_err(|error| format!("Private app data worker failed: {error}"))??;
        Ok(())
    }

    async fn upsert_shelf(&self, book_id: &str) -> Result<(), String> {
        let shelf_ref = self.store.shelf_ref();
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let book = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|error| error.to_string())?;
        let mut entry = json!({
            "id": book["id"], "title": book["title"], "author": book["author"],
            "coverSrc": book["coverSrc"], "chapterCount": book["chapterCount"],
            "latestChapter": book["latestChapter"], "progress": book["progress"], "groups": [],
        });
        self.store
            .update_json_ref(&shelf_ref, move |mut shelf| {
                let books = shelf
                    .get_mut("books")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| "Invalid shelf JSON: books must be an array".to_owned())?;
                if let Some(existing) = books
                    .iter_mut()
                    .find(|existing| existing.get("id").and_then(Value::as_str) == Some(book_id))
                {
                    if let Some(groups) = existing.get("groups").cloned() {
                        entry["groups"] = groups;
                    }
                    *existing = entry;
                } else {
                    books.push(entry);
                }
                crate::reading_tools::apply_shelf_sort(&mut shelf)?;
                Ok(shelf)
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn book_lock(&self, book_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.book_locks.lock().await;
            locks
                .entry(book_id.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    async fn chapter_lock(
        &self,
        book_id: &str,
        chapter_id: &str,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let key = format!("{book_id}\0{chapter_id}");
        let lock = {
            let mut locks = self.chapter_locks.lock().await;
            // The map keeps only weak references. Every in-flight waiter or
            // OwnedMutexGuard owns a strong reference, so duplicate requests
            // still share the same mutex while completed chapter IDs do not
            // accumulate for the lifetime of a long reading session.
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }
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

fn engine_request(
    operation: &str,
    source: &Value,
    keyword: Option<String>,
    page: Option<i32>,
    book: Option<Value>,
    chapter: Option<Value>,
    next_chapter_url: Option<String>,
) -> SourceEngineRequest {
    SourceEngineRequest {
        operation: operation.to_owned(),
        source: source.clone(),
        keyword,
        page,
        book,
        chapter,
        next_chapter_url,
    }
}

fn chapter_id(book_id: &str, stable_url: &str) -> String {
    format!(
        "chapter-{:016x}",
        stable_hash(&format!("{book_id}\0{stable_url}"))
    )
}

fn chapter_id_for_generation(book_id: &str, generation: &str, stable_url: &str) -> String {
    format!(
        "chapter-{:016x}",
        stable_hash(&format!("{book_id}\0{generation}\0{stable_url}"))
    )
}

fn normalize_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn candidate_identity_match(context: &Value, candidate: &Value) -> Option<bool> {
    let expected_title = context.get("targetTitle")?.as_str()?;
    let candidate_title = text_at(candidate, &["name", "title"])?;
    let expected_title = normalize_identity(expected_title);
    if expected_title.is_empty() || normalize_identity(&candidate_title) != expected_title {
        return None;
    }
    let expected_author = context
        .get("targetAuthor")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let candidate_author = text_at(candidate, &["author"]).unwrap_or_default();
    if !expected_author.trim().is_empty()
        && !candidate_author.trim().is_empty()
        && normalize_identity(expected_author) != normalize_identity(&candidate_author)
    {
        return None;
    }
    Some(expected_author.trim().is_empty() || candidate_author.trim().is_empty())
}

fn unique_target_title_index(title: &str, targets: &[String]) -> Option<usize> {
    let normalized = normalize_identity(title);
    if normalized.is_empty() {
        return None;
    }
    let mut matches = targets
        .iter()
        .enumerate()
        .filter(|(_, candidate)| normalize_identity(candidate) == normalized)
        .map(|(index, _)| index);
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn unique_chapter_title_index(title: &str, old: &[String], new: &[String]) -> Option<usize> {
    let normalized = normalize_identity(title);
    if normalized.is_empty() {
        return None;
    }
    let old_matches = old
        .iter()
        .filter(|candidate| normalize_identity(candidate) == normalized)
        .count();
    (old_matches == 1)
        .then(|| unique_target_title_index(title, new))
        .flatten()
}

fn source_definition_fingerprint(source: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(source)
        .map_err(|error| format!("Cannot fingerprint source definition: {error}"))?;
    Ok(sha256_hex(&bytes))
}

fn book_source_fingerprint(
    source_id: &str,
    source_definition: &Value,
    private: &Value,
    book: &Value,
) -> Result<String, String> {
    let chapters = book
        .get("chapters")
        .and_then(Value::as_array)
        .ok_or_else(|| "Book chapter directory is missing".to_owned())?
        .iter()
        .map(|chapter| {
            json!({
                "id": chapter.get("id"),
                "title": chapter.get("title"),
                "index": chapter.get("index"),
            })
        })
        .collect::<Vec<_>>();
    let identity = json!({
        "sourceId": source_id,
        "sourceDefinition": source_definition,
        "bookInstanceId": private.get("bookInstanceId"),
        "catalogGeneration": private.get("catalogGeneration"),
        "engineBook": private.get("book"),
        "rawChapters": private.get("chapters"),
        "title": book.get("title"),
        "author": book.get("author"),
        "chapters": chapters,
    });
    let bytes = serde_json::to_vec(&identity)
        .map_err(|error| format!("Cannot fingerprint book catalog: {error}"))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn task_summary(task: &AppTask) -> Value {
    let mut summary = json!({
        "id": task.id,
        "kind": task.kind,
        "status": task.status,
        "completed": task.completed,
        "total": task.total,
        "createdAtMs": task.created_at_ms,
        "updatedAtMs": task.updated_at_ms,
    });
    if let Some(book_id) = &task.book_id {
        summary["bookId"] = json!(book_id);
    }
    if task.kind == "search" {
        if let Some(search_id) = &task.search_id {
            summary["searchId"] = json!(search_id);
        }
    }
    if let Some(error) = &task.error {
        summary["error"] = json!(error);
    }
    if let Some(result) = &task.result {
        summary["result"] = result.clone();
    }
    summary
}

fn extract_sources(value: Value) -> Result<Vec<Value>, String> {
    if let Some(array) = value.as_array() {
        return Ok(array.clone());
    }
    let Some(object) = value.as_object() else {
        return Err("Source JSON must be an object or array".into());
    };
    for key in ["bookSource", "sources", "data"] {
        if let Some(array) = object.get(key).and_then(Value::as_array) {
            return Ok(array.clone());
        }
    }
    Ok(vec![value])
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetadata {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    pub enabled: bool,
    pub is_rss: bool,
}

fn metadata(records: &[SourceRecord]) -> Vec<SourceMetadata> {
    records
        .iter()
        .map(|record| SourceMetadata {
            id: record.id.clone(),
            name: record.name.clone(),
            group: record.group.clone(),
            enabled: record.enabled,
            is_rss: crate::source_metadata::is_rss_source_metadata(&record.source),
        })
        .collect()
}

fn can_change_source_from_private(private: &Value) -> bool {
    private
        .get("sourceId")
        .and_then(Value::as_str)
        .is_some_and(|source_id| !source_id.trim().is_empty())
        && private.get("book").is_some_and(Value::is_object)
        && private.get("chapters").and_then(Value::as_array).is_some()
}

fn set_optional_public_field(document: &mut Value, key: &str, value: Option<Value>) {
    if let Some(fields) = document.as_object_mut() {
        if let Some(value) = value {
            fields.insert(key.to_owned(), value);
        } else {
            fields.remove(key);
        }
    }
}

fn apply_cached_book_metadata(
    book: &mut Value,
    metadata: &crate::book_metadata::ProcessedBookMetadata,
    source_is_imported: bool,
) {
    if metadata.title != "Untitled" {
        book["title"] = json!(metadata.title.as_str());
    }
    if let Some(author) = metadata.author.as_deref() {
        book["author"] = json!(author);
    }
    if let Some(cover) = metadata.cover_src.as_ref() {
        book["coverSrc"] = json!(cover.as_str());
    }
    if let Some(intro) = metadata.intro.as_deref() {
        book["intro"] = json!(intro);
    }
    if let Some(kind) = metadata.kind.as_deref() {
        book["kind"] = json!(kind);
    }
    if let Some(word_count) = metadata.word_count.as_deref() {
        book["wordCount"] = json!(word_count);
    }
    if !metadata.source_id.is_empty() {
        book["sourceId"] = json!(metadata.source_id.as_str());
    }
    if !metadata.source_name.is_empty() {
        book["sourceName"] = json!(metadata.source_name.as_str());
    }
    if source_is_imported {
        set_optional_public_field(
            book,
            "sourceGroup",
            metadata.source_group.as_deref().map(|group| json!(group)),
        );
    } else if let Some(group) = metadata.source_group.as_deref() {
        book["sourceGroup"] = json!(group);
    }
}

fn apply_refreshed_book_metadata(
    book: &mut Value,
    metadata: &crate::book_metadata::ProcessedBookMetadata,
) {
    if metadata.title != "Untitled" {
        book["title"] = json!(metadata.title.as_str());
    }
    if let Some(author) = metadata.author.as_deref() {
        book["author"] = json!(author);
    }
    if let Some(cover) = metadata.cover_src.as_ref() {
        book["coverSrc"] = json!(cover.as_str());
    }
    if let Some(intro) = metadata.intro.as_deref() {
        book["intro"] = json!(intro);
    }
    if let Some(kind) = metadata.kind.as_deref() {
        book["kind"] = json!(kind);
    }
    if let Some(word_count) = metadata.word_count.as_deref() {
        book["wordCount"] = json!(word_count);
    }
    if let Some(latest_chapter) = metadata.latest_chapter.as_deref() {
        book["latestChapter"] = json!(latest_chapter);
    }
    if !metadata.source_id.is_empty() {
        book["sourceId"] = json!(metadata.source_id.as_str());
    }
    if !metadata.source_name.is_empty() {
        book["sourceName"] = json!(metadata.source_name.as_str());
    }
    set_optional_public_field(
        book,
        "sourceGroup",
        metadata.source_group.as_deref().map(|group| json!(group)),
    );
}

pub(crate) fn project_search_result(result_id: &str, source: &SourceRecord, book: &Value) -> Value {
    let mut result = Map::new();
    result.insert("resultId".into(), json!(result_id));
    result.insert("sourceId".into(), json!(source.id));
    result.insert("sourceName".into(), json!(source.name));
    for (target, keys) in [
        ("title", &["name", "title"][..]),
        ("author", &["author"][..]),
        ("coverSrc", &["coverUrl", "cover", "coverSrc"][..]),
        ("intro", &["intro", "introduction"][..]),
        ("latestChapter", &["lastChapter", "latestChapter"][..]),
        ("bookUrl", &["bookUrl"][..]),
    ] {
        if let Some(value) = keys
            .iter()
            .find_map(|key| book.get(*key).filter(|v| !v.is_null()))
        {
            result.insert(target.to_owned(), value.clone());
        }
    }
    result.entry("title").or_insert_with(|| json!("Untitled"));
    Value::Object(result)
}

fn reader_defaults(settings: Option<Value>) -> ReaderDefaults {
    let Some(mut value) = settings else {
        return ReaderDefaults::default();
    };
    if value.get("reader").is_some() {
        value = value["reader"].clone();
    }
    let mut defaults = ReaderDefaults::default();
    if let Some(size) = value
        .get("fontSizePx")
        .or_else(|| value.get("fontSize"))
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
    {
        defaults.font_size_px = (size as f32).clamp(12.0, 36.0);
    }
    if let Some(line_height) = value
        .get("lineHeight")
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
    {
        defaults.line_height = (line_height as f32).clamp(1.2, 2.8);
    }
    if let Some(family) = value.get("fontFamily").and_then(Value::as_str) {
        defaults.font_family = match family {
            "serif" => "serif",
            "sans" => "sans-serif",
            "system" => "system-ui, sans-serif",
            "mono" => "monospace",
            _ => defaults.font_family.as_str(),
        }
        .to_owned();
    }
    if let Some(color) = value.get("textColor").and_then(Value::as_str) {
        defaults.text_color = reader_color(color, &defaults.text_color);
    }
    if let Some(color) = value.get("backgroundColor").and_then(Value::as_str) {
        defaults.background_color = reader_color(color, &defaults.background_color);
    }
    if let Some(align) = value.get("textAlign").and_then(Value::as_str) {
        if matches!(align, "left" | "right" | "center" | "justify") {
            defaults.text_align = align.to_owned();
        }
    }
    if let Some(preload_count) = value.get("preloadCount").and_then(Value::as_u64) {
        if (1..=20).contains(&preload_count) {
            defaults.preload_count = preload_count as usize;
        }
    }
    if let Some(theme) = value.get("theme").and_then(Value::as_str) {
        defaults.theme = match theme {
            "sepia" => ReaderTheme::Sepia,
            "dark" => ReaderTheme::Dark,
            "system" | "light" => ReaderTheme::System,
            _ => ReaderTheme::Paper,
        };
    }
    defaults
}

fn reader_color(value: &str, fallback: &str) -> String {
    let hex = value.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 6) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return format!("#{hex}");
    }
    match value.to_ascii_lowercase().as_str() {
        "black" | "white" | "red" | "green" | "blue" | "gray" | "grey" | "transparent" => {
            value.to_ascii_lowercase()
        }
        _ => fallback.to_owned(),
    }
}

fn text_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    })
}

fn bool_at(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn strict_unsigned_field(value: &Value, field: &str) -> Result<Option<u64>, String> {
    match value.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("Progress {field} must be a non-negative integer")),
    }
}

fn validate_number_range(
    object: &Map<String, Value>,
    field: &str,
    min: f64,
    max: f64,
) -> Result<(), String> {
    if let Some(value) = object.get(field) {
        let Some(number) = value.as_f64() else {
            return Err(format!("Settings {field} must be a number"));
        };
        if !number.is_finite() || number < min || number > max {
            return Err(format!("Settings {field} must be between {min} and {max}"));
        }
    }
    Ok(())
}

fn validate_integer_range(
    object: &Map<String, Value>,
    field: &str,
    min: u64,
    max: u64,
) -> Result<(), String> {
    if let Some(value) = object.get(field) {
        let Some(number) = value.as_u64() else {
            return Err(format!("Settings {field} must be a non-negative integer"));
        };
        if number < min || number > max {
            return Err(format!("Settings {field} must be between {min} and {max}"));
        }
    }
    Ok(())
}

fn looks_like_html(content: &str) -> bool {
    let trimmed = content.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<body")
        || trimmed.starts_with("<div")
        || trimmed.starts_with("<p")
        || trimmed.starts_with("<section")
}

fn validate_id(id: &str, field: &str) -> Result<(), String> {
    if id.len() > 128
        || id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("Invalid {field}"));
    }
    Ok(())
}

fn stable_hash(text: &str) -> u64 {
    text.as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn content_type(resource: &str) -> &'static str {
    if resource.ends_with(".html") {
        "text/html; charset=utf-8"
    } else {
        "application/json; charset=utf-8"
    }
}

#[cfg(any(test, feature = "desktop", feature = "mobile-runtime"))]
fn picker_display_name(uri: &str, metadata_name: Option<&str>) -> Option<String> {
    let encoded_name = metadata_name.or_else(|| {
        uri.split(|character| character == '?' || character == '#')
            .next()?
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|segment| !segment.is_empty())
    })?;
    let decoded = percent_encoding::percent_decode_str(encoded_name).decode_utf8_lossy();
    let name = Path::new(decoded.as_ref())
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)?;
    // Android providers commonly expose a stable opaque document ID as the
    // last `content://` URI segment. Do not mistake it for a display name.
    if metadata_name.is_none()
        && uri.starts_with("content:")
        && Path::new(&name).extension().is_none()
    {
        return None;
    }
    Some(name)
}

/// Determine Android `content://` file types without trusting the opaque URI
/// suffix. Dialog/fs metadata does not expose the provider's MIME type, so use
/// a reported filename first and inspect bounded file signatures/ZIP entries
/// when the provider returns only an opaque document id.
#[cfg(any(test, feature = "desktop", feature = "mobile-runtime"))]
fn infer_picker_extension(
    _uri: &str,
    metadata_name: Option<&str>,
    file_path: &Path,
    allowed_extensions: &[&str],
) -> Result<String, String> {
    // Only trust a provider-supplied filename, never an opaque content URI
    // suffix. Tauri's current dialog API does not expose MIME metadata, so
    // Android content URIs fall through to actual bytes/ZIP structure.
    if let Some(name) = metadata_name {
        if let Some(extension) = Path::new(&name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
        {
            if allowed_extensions.contains(&extension.as_str()) {
                return Ok(extension);
            }
            return Err(format!(
                "Selected file extension .{extension} is not supported"
            ));
        }
    }

    let prefix = read_file_prefix(file_path, 4096)?;
    if allowed_extensions.contains(&"pdf")
        && prefix
            .get(..prefix.len().min(1024))
            .is_some_and(|header| header.windows(5).any(|window| window == b"%PDF-"))
    {
        return Ok("pdf".to_owned());
    }

    if prefix.starts_with(b"PK") {
        let mut archive = zip::ZipArchive::new(
            std::fs::File::open(file_path)
                .map_err(|error| format!("Cannot inspect the selected ZIP archive: {error}"))?,
        )
        .map_err(|_| "Cannot identify the selected ZIP-based book".to_owned())?;
        if archive.len() > 100_000 {
            return Err("Selected archive contains too many entries".to_owned());
        }
        let has_container = archive
            .file_names()
            .any(|name| name == "META-INF/container.xml");
        let mime_type = match archive.by_name("mimetype") {
            Ok(member) if member.size() <= 128 => {
                use std::io::Read;
                let mut content = Vec::new();
                member
                    .take(128)
                    .read_to_end(&mut content)
                    .map_err(|_| "Cannot inspect the selected EPUB archive".to_owned())?;
                String::from_utf8_lossy(&content).trim().to_owned()
            }
            _ => String::new(),
        };
        if has_container
            && mime_type == "application/epub+zip"
            && allowed_extensions.contains(&"epub")
        {
            return Ok("epub".to_owned());
        }
        let contains_comic_image = archive.file_names().any(|name| {
            matches!(
                Path::new(name)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
            )
        });
        if contains_comic_image && allowed_extensions.contains(&"cbz") {
            return Ok("cbz".to_owned());
        }
        return Err("The selected ZIP file is neither a supported EPUB nor a CBZ".to_owned());
    }

    if allowed_extensions.contains(&"json") && looks_like_picker_json(&prefix) {
        return Ok("json".to_owned());
    }
    if allowed_extensions.contains(&"txt") && looks_like_picker_text(&prefix) {
        return Ok("txt".to_owned());
    }
    Err("Cannot determine the selected file type; choose a TXT, EPUB, CBZ, or PDF file".to_owned())
}

#[cfg(any(test, feature = "desktop", feature = "mobile-runtime"))]
fn read_file_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Cannot inspect the selected file: {error}"))?;
    let mut prefix = Vec::with_capacity(limit);
    file.take(limit as u64)
        .read_to_end(&mut prefix)
        .map_err(|error| format!("Cannot inspect the selected file: {error}"))?;
    Ok(prefix)
}

#[cfg(any(test, feature = "desktop", feature = "mobile-runtime"))]
fn copy_stream_limited<R: std::io::Read, W: std::io::Write>(
    reader: R,
    writer: &mut W,
    limit: u64,
) -> std::io::Result<u64> {
    let mut limited_reader = std::io::Read::take(reader, limit.saturating_add(1));
    let copied = std::io::copy(&mut limited_reader, writer)?;
    if copied > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "selected file exceeds the configured import limit",
        ));
    }
    Ok(copied)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
fn safe_picker_file_name(display_name: Option<&str>, extension: &str) -> String {
    let stem = display_name
        .and_then(|name| Path::new(name).file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("Imported book");
    let mut safe_stem = stem
        .chars()
        .filter_map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | ' ') {
                Some(if character == ' ' { '_' } else { character })
            } else {
                None
            }
        })
        .take(100)
        .collect::<String>();
    while safe_stem.starts_with('.') {
        safe_stem.remove(0);
    }
    if safe_stem.is_empty() {
        safe_stem = "Imported_book".to_owned();
    }
    format!("{safe_stem}.{extension}")
}

#[cfg(any(test, feature = "desktop", feature = "mobile-runtime"))]
fn looks_like_picker_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.contains(&0) {
        return false;
    }
    let sample = &bytes[..bytes.len().min(4096)];
    let control_count = sample
        .iter()
        .filter(|byte| **byte < 0x20 && !matches!(**byte, b'\t' | b'\n' | b'\r' | 0x0c))
        .count();
    control_count * 100 <= sample.len()
}

#[cfg(any(test, feature = "desktop", feature = "mobile-runtime"))]
fn looks_like_picker_json(bytes: &[u8]) -> bool {
    if !looks_like_picker_text(bytes) {
        return false;
    }
    let text = String::from_utf8_lossy(bytes);
    matches!(
        text.trim_start()
            .trim_start_matches('\u{feff}')
            .chars()
            .next(),
        Some('[' | '{')
    )
}

#[cfg(test)]
mod picker_helper_tests {
    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary picker fixture directory")
    }

    fn write_file(directory: &tempfile::TempDir, bytes: &[u8]) -> PathBuf {
        let path = directory.path().join("opaque-document-id");
        std::fs::write(&path, bytes).expect("write picker fixture");
        path
    }

    fn write_zip(directory: &tempfile::TempDir, files: &[(&str, &[u8])]) -> PathBuf {
        use std::io::Write;
        let path = directory.path().join("opaque-zip-id");
        let output = std::fs::File::create(&path).expect("create ZIP fixture");
        let mut archive = zip::ZipWriter::new(output);
        for (name, body) in files {
            archive
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .expect("start ZIP member");
            archive.write_all(body).expect("write ZIP member");
        }
        archive.finish().expect("finish ZIP fixture");
        path
    }

    const BOOK_EXTENSIONS: &[&str] = &["txt", "epub", "cbz", "pdf"];

    #[test]
    fn opaque_content_uris_are_classified_from_file_signatures_and_zip_members() {
        let directory = temp_dir();
        let pdf = write_file(&directory, b"%PDF-1.7\nfixture");
        assert_eq!(
            infer_picker_extension(
                "content://provider/document/opaque-1",
                None,
                &pdf,
                BOOK_EXTENSIONS
            )
            .expect("PDF signature"),
            "pdf"
        );

        let epub = write_zip(
            &directory,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", b"<container/>"),
            ],
        );
        assert_eq!(
            infer_picker_extension(
                "content://provider/document/opaque-2",
                None,
                &epub,
                BOOK_EXTENSIONS
            )
            .expect("EPUB ZIP entries"),
            "epub"
        );

        let cbz = write_zip(&directory, &[("001/cover.png", b"not parsed here")]);
        assert_eq!(
            infer_picker_extension(
                "content://provider/document/opaque-3",
                None,
                &cbz,
                BOOK_EXTENSIONS
            )
            .expect("comic image entry"),
            "cbz"
        );

        let txt = write_file(&directory, "chapter one\n正文".as_bytes());
        assert_eq!(
            infer_picker_extension(
                "content://provider/document/opaque-4",
                None,
                &txt,
                BOOK_EXTENSIONS
            )
            .expect("plain text fallback"),
            "txt"
        );
        assert!(picker_display_name("content://provider/document/opaque-4", None).is_none());
    }

    #[test]
    fn picker_keeps_a_reported_supported_extension_and_rejects_unknown_zip() {
        let directory = temp_dir();
        let fake_pdf = write_file(&directory, b"not a PDF");
        assert_eq!(
            infer_picker_extension(
                "content://provider/document/file.pdf",
                Some("My Book.PDF"),
                &fake_pdf,
                BOOK_EXTENSIONS,
            )
            .expect("provider filename extension"),
            "pdf"
        );

        assert_eq!(
            infer_picker_extension(
                "content://provider/document/opaque-id.pdf",
                None,
                &fake_pdf,
                BOOK_EXTENSIONS,
            )
            .expect("content type comes from file bytes, not URI suffix"),
            "txt"
        );

        let unknown_zip = write_zip(&directory, &[("readme.txt", b"zip archive")]);
        assert!(
            infer_picker_extension(
                "content://provider/document/opaque-zip",
                None,
                &unknown_zip,
                BOOK_EXTENSIONS,
            )
            .is_err()
        );
    }

    #[test]
    fn staging_copy_stops_at_the_configured_limit_plus_one_byte() {
        let mut exact_copy = Vec::new();
        assert_eq!(
            copy_stream_limited(std::io::Cursor::new(b"12345678"), &mut exact_copy, 8)
                .expect("copy exactly at limit"),
            8
        );
        assert_eq!(exact_copy, b"12345678");

        let mut over_limit_copy = Vec::new();
        let error =
            copy_stream_limited(std::io::Cursor::new(b"1234567890"), &mut over_limit_copy, 8)
                .expect_err("reject one byte over limit");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(over_limit_copy.len(), 9);
    }
}

#[cfg(test)]
mod pending_pdf_registry_tests {
    use super::{ApplicationService, PendingPdfImportRegistry, PickerFile};
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };

    fn temporary_picker_file(
        root: &tempfile::TempDir,
        directory_name: &str,
    ) -> (PickerFile, PathBuf, PathBuf) {
        let stage = root.path().join(directory_name);
        std::fs::create_dir(&stage).expect("create staged import directory");
        let file = stage.join("document.pdf");
        std::fs::write(&file, b"encrypted-pdf-fixture").expect("create staged PDF");
        (
            PickerFile {
                path: file.clone(),
                temporary: true,
                cleanup_dir: Some(stage.clone()),
            },
            file,
            stage,
        )
    }

    #[test]
    fn cancelling_pending_pdf_removes_the_staged_file_and_directory() {
        let root = tempfile::tempdir().expect("temporary registry fixture");
        let registry = PendingPdfImportRegistry::new(Duration::from_secs(300));
        let now = Instant::now();
        let (file, path, stage) = temporary_picker_file(&root, "cancel-import");
        let token = registry.insert_at(file, now).expect("insert pending PDF");

        assert!(
            registry
                .cancel_at(&token, now + Duration::from_secs(1))
                .expect("cancel pending PDF")
        );
        assert!(!path.exists());
        assert!(!stage.exists());
        assert!(
            !registry
                .cancel_at(&token, now + Duration::from_secs(1))
                .expect("repeat cancellation is harmless")
        );
    }

    #[test]
    fn recoverable_password_failure_can_reinsert_the_same_pending_file() {
        let root = tempfile::tempdir().expect("temporary registry fixture");
        let registry = PendingPdfImportRegistry::new(Duration::from_secs(300));
        let now = Instant::now();
        let (file, path, _) = temporary_picker_file(&root, "retry-import");
        let token = registry.insert_at(file, now).expect("insert pending PDF");

        // A bad password follows the Tauri flow: take for one attempt, then put
        // the same file back so the user can retry with another password.
        let pending = registry
            .take_at(&token, now + Duration::from_secs(20))
            .expect("take for password attempt")
            .expect("pending file exists");
        assert!(path.exists());
        registry
            .reinsert_at(token.clone(), pending, now + Duration::from_secs(21))
            .expect("restore retry state");

        let retry = registry
            .take_at(&token, now + Duration::from_secs(22))
            .expect("take for retry")
            .expect("retry entry is present");
        assert!(path.exists());
        drop(retry);
        assert!(!path.exists());
    }

    #[test]
    fn pending_pdf_expires_and_cleans_up_without_waiting_for_the_ttl() {
        let root = tempfile::tempdir().expect("temporary registry fixture");
        let registry = PendingPdfImportRegistry::new(Duration::from_secs(300));
        let now = Instant::now();
        let (file, path, stage) = temporary_picker_file(&root, "expire-import");
        let token = registry.insert_at(file, now).expect("insert pending PDF");

        assert!(
            registry
                .expire_at(&token, now + Duration::from_secs(299))
                .expect("check before expiry")
                == false
        );
        assert!(path.exists());
        assert!(
            registry
                .expire_at(&token, now + Duration::from_secs(300))
                .expect("expire pending PDF")
        );
        assert!(!path.exists());
        assert!(!stage.exists());
    }

    #[test]
    fn replacing_the_active_challenge_invalidates_and_cleans_the_previous_file() {
        let root = tempfile::tempdir().expect("temporary registry fixture");
        let registry = PendingPdfImportRegistry::new(Duration::from_secs(300));
        let now = Instant::now();
        let (first, first_path, first_stage) = temporary_picker_file(&root, "first-import");
        let first_token = registry
            .insert_at(first, now)
            .expect("insert first pending PDF");
        let (second, second_path, second_stage) = temporary_picker_file(&root, "second-import");
        let second_token = registry
            .insert_at(second, now + Duration::from_secs(1))
            .expect("replace pending PDF");

        assert_ne!(first_token, second_token);
        assert!(!first_path.exists());
        assert!(!first_stage.exists());
        assert!(second_path.exists());
        assert!(second_stage.exists());
        assert!(
            registry
                .take_at(&first_token, now + Duration::from_secs(2))
                .expect("check stale challenge")
                .is_none()
        );
        drop(
            registry
                .take_at(&second_token, now + Duration::from_secs(2))
                .expect("take current challenge")
                .expect("current challenge remains active"),
        );
        assert!(!second_path.exists());
    }

    #[test]
    fn clearing_pending_imports_drops_the_current_temporary_file() {
        let root = tempfile::tempdir().expect("temporary registry fixture");
        let registry = PendingPdfImportRegistry::new(Duration::from_secs(300));
        let (file, path, stage) = temporary_picker_file(&root, "restore-import");
        let token = registry
            .insert_at(file, Instant::now())
            .expect("insert pending PDF");

        registry.clear().expect("clear after restore");
        assert!(!path.exists());
        assert!(!stage.exists());
        assert!(
            registry
                .take_at(&token, Instant::now())
                .expect("read cleared registry")
                .is_none()
        );
    }

    #[tokio::test]
    async fn protected_pdf_challenge_is_singleton_and_allows_a_password_retry() {
        let root = tempfile::tempdir().expect("temporary application data");
        let service = ApplicationService::open(root.path(), None)
            .await
            .expect("open app service");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/local-books/two-page-encrypted.pdf");
        let challenge = service
            .import_local_book_with_challenge(
                PickerFile::from_existing_path(&fixture),
                crate::local_books::LocalImportOptions::default(),
            )
            .await
            .expect("return password challenge");
        assert_eq!(challenge["passwordRequired"], true);
        let token = challenge["importToken"]
            .as_str()
            .expect("opaque challenge token")
            .to_owned();

        let replacement = service
            .import_local_book_with_challenge(
                PickerFile::from_existing_path(&fixture),
                crate::local_books::LocalImportOptions::default(),
            )
            .await
            .expect("replace existing password challenge");
        let replacement_token = replacement["importToken"]
            .as_str()
            .expect("replacement challenge token")
            .to_owned();
        assert_ne!(token, replacement_token);
        assert!(
            service
                .retry_pending_pdf_import(&token, "fixture-pass".to_owned())
                .await
                .unwrap_err()
                .contains("expired")
        );

        let wrong_password = service
            .retry_pending_pdf_import(&replacement_token, "wrong-pass".to_owned())
            .await
            .expect_err("wrong password should allow a retry");
        assert!(wrong_password.contains("password is incorrect"));

        let imported = service
            .retry_pending_pdf_import(&replacement_token, "fixture-pass".to_owned())
            .await
            .expect("retry with the correct password");
        assert!(imported["book"]["src"].is_string());
        assert!(
            service
                .retry_pending_pdf_import(&replacement_token, "fixture-pass".to_owned())
                .await
                .unwrap_err()
                .contains("expired")
        );
    }

    #[tokio::test]
    async fn successful_restore_invalidates_and_cleans_pending_picker_imports() {
        let temporary = tempfile::tempdir().expect("temporary application data");
        let app_root = temporary.path().join("app-data");
        let archive = temporary.path().join("snapshot.zip");
        let service = ApplicationService::open(&app_root, None)
            .await
            .expect("open app service");
        service
            .create_backup(&archive)
            .await
            .expect("create restore fixture");

        let staging = app_root.join("private-data/picker-imports/active");
        std::fs::create_dir_all(&staging).expect("create pending picker stage");
        let staged_file = staging.join("encrypted.pdf");
        std::fs::write(&staged_file, b"encrypted fixture").expect("write pending PDF");
        let token = service
            .pending_pdf_imports
            .insert_at(
                PickerFile {
                    path: staged_file.clone(),
                    temporary: true,
                    cleanup_dir: Some(staging.clone()),
                },
                Instant::now(),
            )
            .expect("register pending PDF");

        service
            .restore_backup(&archive)
            .await
            .expect("restore snapshot");
        assert!(!staged_file.exists());
        assert!(!staging.exists());
        assert!(
            service
                .retry_pending_pdf_import(&token, "fixture-pass".to_owned())
                .await
                .unwrap_err()
                .contains("expired")
        );
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod tauri_commands {
    use super::*;
    use tauri::{Emitter, Manager, State};
    use tauri_plugin_dialog::{DialogExt, FilePath};

    async fn pick_file(
        app: &tauri::AppHandle,
        label: &'static str,
        extensions: &'static [&'static str],
    ) -> Result<Option<PickerFile>, String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .add_filter(label, extensions)
            .pick_file(move |selected| {
                let _ = sender.send(selected);
            });
        let Some(selected) = receiver
            .await
            .map_err(|error| format!("File picker failed: {error}"))?
        else {
            return Ok(None);
        };
        selected_to_local(app, selected, extensions).await.map(Some)
    }

    async fn pick_save_file(
        app: &tauri::AppHandle,
        label: &'static str,
        extensions: &'static [&'static str],
    ) -> Result<Option<FilePath>, String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .add_filter(label, extensions)
            .save_file(move |selected| {
                let _ = sender.send(selected);
            });
        let Some(selected) = receiver
            .await
            .map_err(|error| format!("Save picker failed: {error}"))?
        else {
            return Ok(None);
        };
        Ok(Some(selected))
    }

    async fn selected_to_local(
        app: &tauri::AppHandle,
        selected: FilePath,
        allowed_extensions: &[&str],
    ) -> Result<PickerFile, String> {
        let selected_uri = selected.to_string();
        let display_name = picker_display_name(&selected_uri, None);

        // Desktop paths can be used directly. On Android/iOS, always go
        // through the fs plugin so content URIs and security-scoped URLs are
        // opened while the native permission is active.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if let Ok(path) = selected.clone().into_path() {
            validate_picker_extension(&path, allowed_extensions)?;
            validate_picker_size(&path, allowed_extensions)?;
            return Ok(PickerFile {
                path,
                temporary: false,
                cleanup_dir: None,
            });
        }

        use tauri_plugin_fs::FsExt;
        let mut options = tauri_plugin_fs::OpenOptions::new();
        options.read(true);
        let source = app
            .fs()
            .open(selected.clone(), options)
            .map_err(|error| format!("Cannot read selected file: {error}"))?;
        if source
            .metadata()
            .ok()
            .is_some_and(|metadata| metadata.len() > MAX_PICKER_FILE_BYTES)
        {
            return Err(format!(
                "Selected file exceeds the {} MiB import limit",
                MAX_PICKER_FILE_BYTES / (1024 * 1024)
            ));
        }
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("private-data")
            .join("picker-imports");
        let staging_directory = directory.join(uuid::Uuid::new_v4().simple().to_string());
        tokio::fs::create_dir_all(&staging_directory)
            .await
            .map_err(|error| error.to_string())?;
        let raw_path = staging_directory.join("source.bin");
        let copy_path = raw_path.clone();
        let copy_result = tokio::task::spawn_blocking(move || {
            use std::io;
            let mut destination = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&copy_path)?;
            let copied = copy_stream_limited(source, &mut destination, MAX_PICKER_FILE_BYTES)?;
            destination.sync_all()?;
            Ok::<u64, io::Error>(copied)
        })
        .await;
        let copy_result = match copy_result {
            Ok(result) => result,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&staging_directory).await;
                return Err(format!("Cannot stage selected file: {error}"));
            }
        };
        let file_size = match copy_result {
            Ok(size) => size,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&staging_directory).await;
                if error.kind() == std::io::ErrorKind::InvalidData {
                    return Err(format!(
                        "Selected file exceeds the {} MiB import limit",
                        MAX_PICKER_FILE_BYTES / (1024 * 1024)
                    ));
                }
                return Err(format!("Cannot stage selected file: {error}"));
            }
        };
        let extension =
            match infer_picker_extension(&selected_uri, None, &raw_path, allowed_extensions) {
                Ok(extension) => extension,
                Err(error) => {
                    let _ = tokio::fs::remove_dir_all(&staging_directory).await;
                    return Err(error);
                }
            };
        if extension == "json" && file_size > MAX_PICKER_SOURCE_JSON_BYTES {
            let _ = tokio::fs::remove_dir_all(&staging_directory).await;
            return Err(format!(
                "Selected source file exceeds the {} MiB import limit",
                MAX_PICKER_SOURCE_JSON_BYTES / (1024 * 1024)
            ));
        }
        let path =
            staging_directory.join(safe_picker_file_name(display_name.as_deref(), &extension));
        if let Err(error) = tokio::fs::rename(&raw_path, &path).await {
            let _ = tokio::fs::remove_dir_all(&staging_directory).await;
            return Err(format!("Cannot finalize selected file: {error}"));
        }
        Ok(PickerFile {
            path,
            temporary: true,
            cleanup_dir: Some(staging_directory),
        })
    }

    fn validate_picker_size(path: &Path, allowed_extensions: &[&str]) -> Result<(), String> {
        let size = std::fs::metadata(path)
            .map_err(|error| format!("Cannot inspect selected file: {error}"))?
            .len();
        if size > MAX_PICKER_FILE_BYTES {
            return Err(format!(
                "Selected file exceeds the {} MiB import limit",
                MAX_PICKER_FILE_BYTES / (1024 * 1024)
            ));
        }
        if allowed_extensions.contains(&"json")
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && size > MAX_PICKER_SOURCE_JSON_BYTES
        {
            return Err(format!(
                "Selected source file exceeds the {} MiB import limit",
                MAX_PICKER_SOURCE_JSON_BYTES / (1024 * 1024)
            ));
        }
        Ok(())
    }

    fn validate_picker_extension(path: &Path, allowed: &[&str]) -> Result<(), String> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .ok_or_else(|| "Selected file has no supported extension".to_owned())?;
        if allowed.contains(&extension.as_str()) {
            Ok(())
        } else {
            Err(format!(
                "Selected file extension .{extension} is not supported"
            ))
        }
    }

    fn write_selected_file(
        app: &tauri::AppHandle,
        selected: FilePath,
        bytes: &[u8],
    ) -> Result<(), String> {
        if let Ok(path) = selected.clone().into_path() {
            return std::fs::write(path, bytes)
                .map_err(|error| format!("Cannot write selected file: {error}"));
        }
        use tauri_plugin_fs::FsExt;
        let mut options = tauri_plugin_fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        let mut file = app
            .fs()
            .open(selected, options)
            .map_err(|error| format!("Cannot open selected destination: {error}"))?;
        std::io::Write::write_all(&mut file, bytes)
            .map_err(|error| format!("Cannot write selected destination: {error}"))
    }

    #[tauri::command]
    pub async fn app_bootstrap(service: State<'_, ApplicationService>) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        service.bootstrap().await
    }

    #[tauri::command]
    pub async fn list_sources(service: State<'_, ApplicationService>) -> Result<Value, String> {
        service.list_sources().await
    }

    #[tauri::command]
    pub async fn get_search_history(
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        service.get_search_history().await
    }

    #[tauri::command]
    pub async fn delete_search_history(
        app: tauri::AppHandle,
        query: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource = service.delete_search_history(&query).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "searchHistory", "resource": resource }),
        );
        Ok(resource)
    }

    #[tauri::command]
    pub async fn clear_search_history(
        app: tauri::AppHandle,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource = service.clear_search_history().await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "searchHistory", "resource": resource }),
        );
        Ok(resource)
    }

    #[tauri::command]
    pub async fn import_sources(
        app: tauri::AppHandle,
        source_json: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.import_sources(&source_json).await?;
        let _ = app.emit("sources-updated", result["sources"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn import_sources_from_picker(
        app: tauri::AppHandle,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let Some(path) = pick_file(&app, "JSON", &["json"]).await? else {
            return Ok(
                json!({ "cancelled": true, "sources": service.list_sources().await?["sources"] }),
            );
        };
        let source_json = tokio::fs::read_to_string(&path.path)
            .await
            .map_err(|error| format!("Cannot read selected source file: {error}"))?;
        let result = service.import_sources(&source_json).await?;
        let _ = app.emit("sources-updated", result["sources"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn remove_sources(
        app: tauri::AppHandle,
        source_ids: Vec<String>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.remove_sources(&source_ids).await?;
        let _ = app.emit("sources-updated", result["sources"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn update_source(
        app: tauri::AppHandle,
        source_id: String,
        patch: Value,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.update_source(&source_id, patch).await?;
        let _ = app.emit("sources-updated", result["sources"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn search_books(
        app: tauri::AppHandle,
        source_ids: Vec<String>,
        keyword: String,
        page: Option<u32>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let keyword_for_event = keyword.clone();
        let result = service
            .search_books(&source_ids, &keyword, page.unwrap_or(1), |progress| {
                let _ = app.emit("search-progress", progress);
            })
            .await?;
        let history = service.get_search_history().await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "searchHistory", "resource": history }),
        );
        let _ = app.emit(
            "search-complete",
            json!({
                "keyword": keyword_for_event,
                "resource": result["resource"],
                "bookCount": result["bookCount"],
                "errors": result["errors"],
            }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn start_search(
        app: tauri::AppHandle,
        source_ids: Vec<String>,
        keyword: String,
        page: Option<u32>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service
            .start_search(&source_ids, &keyword, page.unwrap_or(1))
            .await?;
        let _ = app.emit("search-started", result.clone());
        let history = service.get_search_history().await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "searchHistory", "resource": history }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn search_book_source_candidates(
        app: tauri::AppHandle,
        book_id: String,
        source_ids: Vec<String>,
        keyword: Option<String>,
        page: Option<u32>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service
            .search_book_source_candidates(
                &book_id,
                &source_ids,
                keyword.as_deref(),
                page.unwrap_or(1),
            )
            .await?;
        let _ = app.emit("search-started", result.clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn tasks_resource(service: State<'_, ApplicationService>) -> Result<Value, String> {
        service.tasks_resource().await
    }

    #[tauri::command]
    pub async fn start_chapter_download(
        app: tauri::AppHandle,
        book_id: String,
        from_index: usize,
        count: usize,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service
            .start_chapter_download(&book_id, from_index, count)
            .await?;
        let _ = app.emit(
            "task-updated",
            json!({ "task": result["task"], "resource": result["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn refresh_chapters(
        app: tauri::AppHandle,
        book_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.refresh_chapters(&book_id).await?;
        let _ = app.emit(
            "task-updated",
            json!({ "task": result["task"], "resource": result["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn check_new_chapters(
        app: tauri::AppHandle,
        book_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.check_new_chapters(&book_id).await?;
        let _ = app.emit(
            "task-updated",
            json!({ "task": result["task"], "resource": result["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn pause_task(
        task_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        service.pause_task(&task_id).await
    }

    #[tauri::command]
    pub async fn resume_task(
        task_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        service.resume_task(&task_id).await
    }

    #[tauri::command]
    pub async fn cancel_task(
        task_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        service.cancel_task(&task_id).await
    }

    #[tauri::command]
    pub async fn add_book(
        app: tauri::AppHandle,
        result_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.add_book(&result_id).await?;
        let _ = app.emit("book-added", result["book"].clone());
        let _ = app.emit("shelf-updated", result["shelf"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn change_book_source(
        app: tauri::AppHandle,
        book_id: String,
        result_id: String,
        confirm_missing_author: bool,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service
            .change_book_source(&book_id, &result_id, confirm_missing_author)
            .await?;
        let _ = app.emit("book-source-changed", result.clone());
        let _ = app.emit("shelf-updated", result["shelf"].clone());
        let _ = app.emit(
            "progress-saved",
            json!({ "bookId": book_id, "book": result["book"] }),
        );
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "bookmarks", "resource": result["bookmarks"]["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn import_book_from_picker(
        app: tauri::AppHandle,
        options: crate::local_books::LocalImportOptions,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let Some(path) = pick_file(&app, "Books", &["txt", "epub", "cbz", "pdf"]).await? else {
            return Ok(json!({ "cancelled": true }));
        };
        let result = service
            .import_local_book_with_challenge(path, options)
            .await?;
        if result.get("passwordRequired").and_then(Value::as_bool) == Some(true) {
            return Ok(result);
        }
        let _ = app.emit("book-added", result["book"].clone());
        let _ = app.emit("shelf-updated", result["shelf"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn import_protected_pdf(
        app: tauri::AppHandle,
        import_token: String,
        password: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service
            .retry_pending_pdf_import(&import_token, password)
            .await?;
        let _ = app.emit("book-added", result["book"].clone());
        let _ = app.emit("shelf-updated", result["shelf"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn cancel_pending_pdf_import(
        import_token: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _ = service.cancel_pending_pdf_import(&import_token).await?;
        Ok(json!({ "cancelled": true }))
    }

    #[tauri::command]
    pub async fn get_book(
        book_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        service.get_book(&book_id).await
    }

    #[tauri::command]
    pub async fn refresh_book_info(
        app: tauri::AppHandle,
        book_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.refresh_book_info(&book_id).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "book", "resource": result["book"] }),
        );
        let _ = app.emit("shelf-updated", result["shelf"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn prepare_chapters(
        app: tauri::AppHandle,
        book_id: String,
        from_index: usize,
        count: usize,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service
            .prepare_chapters(&book_id, from_index, count)
            .await?;
        let _ = app.emit(
            "chapters-prepared",
            json!({
                "bookId": book_id,
                "fromIndex": from_index,
                "prepared": result["prepared"],
                "book": result["book"],
            }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn remove_book(
        app: tauri::AppHandle,
        book_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.remove_book(&book_id).await?;
        let _ = app.emit("shelf-updated", result["shelf"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn save_progress(
        app: tauri::AppHandle,
        book_id: String,
        progress: Value,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.save_progress(&book_id, progress).await?;
        let _ = app.emit(
            "progress-saved",
            json!({ "bookId": book_id, "book": result }),
        );
        let _ = app.emit(
            "shelf-updated",
            service.resource_descriptor(&service.resource_store().shelf_ref()),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn save_settings(
        app: tauri::AppHandle,
        settings: Value,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.save_settings(settings).await?;
        let _ = app.emit("settings-updated", result.clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn get_home_config(service: State<'_, ApplicationService>) -> Result<Value, String> {
        service.get_home_config().await
    }

    #[tauri::command]
    pub async fn save_home_config(
        app: tauri::AppHandle,
        config: crate::discovery::home_config::HomeConfigDocument,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource = service.save_home_config(config).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "homeConfig", "resource": resource }),
        );
        Ok(resource)
    }

    #[tauri::command]
    pub async fn get_rss_state(service: State<'_, ApplicationService>) -> Result<Value, String> {
        service.get_rss_state().await
    }

    #[tauri::command]
    pub async fn set_rss_filter(
        app: tauri::AppHandle,
        source_id: String,
        filter: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource = service.set_rss_filter(&source_id, &filter).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssState", "resource": resource }),
        );
        Ok(resource)
    }

    #[tauri::command]
    pub async fn set_rss_article_state(
        app: tauri::AppHandle,
        source_id: String,
        article_id: String,
        is_read: Option<bool>,
        is_favorite: Option<bool>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource = service
            .set_rss_article_state(&source_id, &article_id, is_read, is_favorite)
            .await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssState", "resource": resource }),
        );
        Ok(resource)
    }

    #[tauri::command]
    pub async fn unsubscribe_rss(
        app: tauri::AppHandle,
        source_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let result = service.unsubscribe_rss(&source_id).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssState", "resource": result["resource"] }),
        );
        let _ = app.emit("sources-updated", result["sources"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn get_txt_toc_rules(
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        service.get_txt_toc_rules().await
    }

    #[tauri::command]
    pub async fn upsert_txt_toc_rule(
        app: tauri::AppHandle,
        rule: crate::local_books::txt_toc_rules::TxtTocRule,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource = service.upsert_txt_toc_rule(rule).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "txtTocRules", "resource": resource }),
        );
        Ok(resource)
    }

    #[tauri::command]
    pub async fn delete_txt_toc_rule(
        app: tauri::AppHandle,
        rule_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource = service.delete_txt_toc_rule(&rule_id).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "txtTocRules", "resource": resource }),
        );
        Ok(resource)
    }

    #[tauri::command]
    pub async fn list_bookmarks(service: State<'_, ApplicationService>) -> Result<Value, String> {
        let resource = crate::reading_tools::bookmarks_resource(service.resource_store()).await?;
        Ok(service.resource_descriptor(&resource))
    }

    #[tauri::command]
    pub async fn upsert_bookmark(
        app: tauri::AppHandle,
        input: crate::reading_tools::BookmarkInput,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::upsert_bookmark(service.resource_store(), input).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "bookmarks", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn delete_bookmark(
        app: tauri::AppHandle,
        bookmark_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::delete_bookmark(service.resource_store(), &bookmark_id).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "bookmarks", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn reading_history_resource(
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource =
            crate::reading_tools::reading_history_resource(service.resource_store()).await?;
        Ok(service.resource_descriptor(&resource))
    }

    #[tauri::command]
    pub async fn record_reading_session(
        app: tauri::AppHandle,
        session: crate::reading_tools::ReadingSession,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::record_reading_session(service.resource_store(), session).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "readingHistory", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn delete_reading_history_for_book(
        app: tauri::AppHandle,
        book_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource = crate::reading_tools::delete_reading_history_for_book(
            service.resource_store(),
            &book_id,
        )
        .await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "readingHistory", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn clear_reading_history(
        app: tauri::AppHandle,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::clear_reading_history(service.resource_store()).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "readingHistory", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn list_replacement_rules(
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let resource =
            crate::reading_tools::replacement_rules_resource(service.resource_store()).await?;
        Ok(service.resource_descriptor(&resource))
    }

    #[tauri::command]
    pub async fn upsert_replacement_rule(
        app: tauri::AppHandle,
        input: crate::reading_tools::DisplayReplacementRuleInput,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::upsert_replacement_rule(service.resource_store(), input).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "replacementRules", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn delete_replacement_rule(
        app: tauri::AppHandle,
        rule_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::delete_replacement_rule(service.resource_store(), &rule_id)
                .await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "replacementRules", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn set_book_groups(
        app: tauri::AppHandle,
        book_id: String,
        groups: Vec<String>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::set_book_groups(service.resource_store(), &book_id, groups)
                .await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit("shelf-updated", descriptor.clone());
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn create_shelf_group(
        app: tauri::AppHandle,
        group_name: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::create_shelf_group(service.resource_store(), &group_name).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit("shelf-updated", descriptor.clone());
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn rename_shelf_group(
        app: tauri::AppHandle,
        old_name: String,
        new_name: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource = crate::reading_tools::rename_shelf_group(
            service.resource_store(),
            &old_name,
            &new_name,
        )
        .await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit("shelf-updated", descriptor.clone());
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn delete_shelf_group(
        app: tauri::AppHandle,
        group_name: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::delete_shelf_group(service.resource_store(), &group_name).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit("shelf-updated", descriptor.clone());
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn set_shelf_sort(
        app: tauri::AppHandle,
        key: crate::reading_tools::ShelfSortKey,
        order: crate::reading_tools::ShelfSortOrder,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::set_shelf_sort(service.resource_store(), key, order).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit("shelf-updated", descriptor.clone());
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn set_shelf_order(
        app: tauri::AppHandle,
        ordered_book_ids: Vec<String>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::reading_tools::set_shelf_order(service.resource_store(), ordered_book_ids)
                .await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit("shelf-updated", descriptor.clone());
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn list_discovery_categories(
        app: tauri::AppHandle,
        source_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let result = crate::discovery::list_categories(&service, &source_id, Some(false)).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "discoveryCategories", "resource": result["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn list_discovery_books(
        app: tauri::AppHandle,
        source_id: String,
        category_id: String,
        page: Option<u32>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let result = crate::discovery::list_books(
            &service,
            &source_id,
            &category_id,
            page.unwrap_or(1),
            Some(false),
        )
        .await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "discoveryResults", "resource": result["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn list_discovery_favorites(
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource = crate::discovery::favorites_resource(service.resource_store()).await?;
        Ok(json!({ "resource": service.resource_descriptor(&resource) }))
    }

    #[tauri::command]
    pub async fn set_discovery_favorite(
        app: tauri::AppHandle,
        source_id: String,
        category_id: String,
        favorite: bool,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let resource =
            crate::discovery::set_favorite(&service, &source_id, &category_id, favorite).await?;
        let descriptor = service.resource_descriptor(&resource);
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "discoveryFavorites", "resource": descriptor }),
        );
        Ok(descriptor)
    }

    #[tauri::command]
    pub async fn list_rss_categories(
        app: tauri::AppHandle,
        source_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let result = crate::rss::list_rss_categories(&service, &source_id).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssCategories", "resource": result["resource"] }),
        );
        if let Some(resource) = result.get("rssState") {
            let _ = app.emit(
                "resource-updated",
                json!({ "kind": "rssState", "resource": resource }),
            );
        }
        Ok(result)
    }

    #[tauri::command]
    pub async fn list_rss_articles(
        app: tauri::AppHandle,
        source_id: String,
        category_id: String,
        page: Option<u32>,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let result =
            crate::rss::list_rss_articles(&service, &source_id, &category_id, page.unwrap_or(1))
                .await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssArticles", "resource": result["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn open_rss_article(
        app: tauri::AppHandle,
        source_id: String,
        article_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let _operation = service.operation_read().await;
        let result =
            crate::rss::read_standard_feed_article(&service, &source_id, &article_id).await?;
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "rssArticle", "resource": result["resource"] }),
        );
        Ok(result)
    }

    #[tauri::command]
    pub async fn create_backup_from_picker(
        app: tauri::AppHandle,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let Some(destination) = pick_save_file(&app, "ZIP backup", &["zip"]).await? else {
            return Ok(json!({ "cancelled": true }));
        };
        if let Ok(path) = destination.clone().into_path() {
            service.create_backup(&path).await?;
        } else {
            let directory = std::env::temp_dir().join("legado-rs-backup-staging");
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| error.to_string())?;
            let staged = PickerFile {
                path: directory.join(format!("{}.zip", uuid::Uuid::new_v4().simple())),
                temporary: true,
                cleanup_dir: None,
            };
            service.create_backup(&staged.path).await?;
            let bytes = tokio::fs::read(&staged.path)
                .await
                .map_err(|error| error.to_string())?;
            write_selected_file(&app, destination, &bytes)?;
        }
        Ok(json!({ "backup": true }))
    }

    #[tauri::command]
    pub async fn restore_backup_from_picker(
        app: tauri::AppHandle,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let Some(path) = pick_file(&app, "Legado backup", &["zip"]).await? else {
            return Ok(json!({ "cancelled": true }));
        };
        let bootstrap = service.restore_backup(&path.path).await?;
        let _ = app.emit("app-state-updated", bootstrap.clone());
        let _ = app.emit("shelf-updated", bootstrap["shelf"].clone());
        let _ = app.emit("settings-updated", bootstrap["settings"].clone());
        let _ = app.emit("sources-updated", bootstrap["sources"].clone());
        Ok(json!({ "restored": true, "bootstrap": bootstrap }))
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use tauri_commands::*;

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet, VecDeque},
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    use serde_json::{Value, json};

    use super::{AppTask, ApplicationService, EngineFuture, SourceExecutor, task_summary, text_at};
    use crate::source_engine::SourceEngineRequest;

    #[derive(Clone)]
    struct ControlledExecutor {
        chapter_count: usize,
        catalog_responses: Arc<std::sync::Mutex<HashMap<String, VecDeque<Value>>>>,
        catalog_gates: Arc<std::sync::Mutex<HashMap<String, Arc<ExecutionGate>>>>,
        duplicate_catalogs: Arc<std::sync::Mutex<HashSet<String>>>,
        content_gates: Arc<std::sync::Mutex<HashMap<String, Arc<ExecutionGate>>>>,
        chapter_content_gates: Arc<std::sync::Mutex<HashMap<String, Arc<ExecutionGate>>>>,
        content_failures: Arc<std::sync::Mutex<HashSet<String>>>,
        search_gates: Arc<std::sync::Mutex<HashMap<String, Arc<ExecutionGate>>>>,
        operation_gates: Arc<std::sync::Mutex<HashMap<(String, String), Arc<ExecutionGate>>>>,
        operation_failures: Arc<std::sync::Mutex<HashSet<(String, String)>>>,
        operation_responses: Arc<std::sync::Mutex<HashMap<(String, String), VecDeque<Value>>>>,
        content_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    struct ExecutionGate {
        entered: tokio::sync::Notify,
        permits: tokio::sync::Semaphore,
    }

    impl ExecutionGate {
        fn new() -> Self {
            Self {
                entered: tokio::sync::Notify::new(),
                permits: tokio::sync::Semaphore::new(0),
            }
        }

        async fn wait_until_entered(&self) {
            tokio::time::timeout(Duration::from_secs(5), self.entered.notified())
                .await
                .expect("executor should reach the controlled operation");
        }

        fn release(&self) {
            self.permits.add_permits(1);
        }
    }

    impl ControlledExecutor {
        fn new(chapter_count: usize) -> Self {
            Self {
                chapter_count,
                catalog_responses: Arc::new(std::sync::Mutex::new(HashMap::new())),
                catalog_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
                duplicate_catalogs: Arc::new(std::sync::Mutex::new(HashSet::new())),
                content_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
                chapter_content_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
                content_failures: Arc::new(std::sync::Mutex::new(HashSet::new())),
                search_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
                operation_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
                operation_failures: Arc::new(std::sync::Mutex::new(HashSet::new())),
                operation_responses: Arc::new(std::sync::Mutex::new(HashMap::new())),
                content_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        fn queue_catalog_responses(&self, book_url: &str, responses: Vec<Value>) {
            self.catalog_responses
                .lock()
                .expect("catalog response mutex")
                .insert(book_url.to_owned(), responses.into());
        }

        fn gate_next_catalog(&self, book_url: &str) -> Arc<ExecutionGate> {
            let gate = Arc::new(ExecutionGate::new());
            self.catalog_gates
                .lock()
                .expect("catalog gate mutex")
                .insert(book_url.to_owned(), gate.clone());
            gate
        }

        fn duplicate_catalog_for(&self, book_url: &str) {
            self.duplicate_catalogs
                .lock()
                .expect("duplicate catalog mutex")
                .insert(book_url.to_owned());
        }

        fn gate_content(&self, book_url: &str) -> Arc<ExecutionGate> {
            let gate = Arc::new(ExecutionGate::new());
            self.content_gates
                .lock()
                .expect("content gate mutex")
                .insert(book_url.to_owned(), gate.clone());
            gate
        }

        fn gate_chapter_content(&self, chapter_url: &str) -> Arc<ExecutionGate> {
            let gate = Arc::new(ExecutionGate::new());
            self.chapter_content_gates
                .lock()
                .expect("chapter content gate mutex")
                .insert(chapter_url.to_owned(), gate.clone());
            gate
        }

        fn fail_content_once(&self, chapter_url: &str) {
            self.content_failures
                .lock()
                .expect("content failure mutex")
                .insert(chapter_url.to_owned());
        }

        fn gate_search(&self, keyword: &str) -> Arc<ExecutionGate> {
            let gate = Arc::new(ExecutionGate::new());
            self.search_gates
                .lock()
                .expect("search gate mutex")
                .insert(keyword.to_owned(), gate.clone());
            gate
        }

        fn fail_operation_for_source(&self, operation: &str, source_url: &str) {
            self.operation_failures
                .lock()
                .expect("operation failure mutex")
                .insert((operation.to_owned(), source_url.to_owned()));
        }

        fn queue_operation_response_for_source(
            &self,
            operation: &str,
            source_url: &str,
            response: Value,
        ) {
            self.operation_responses
                .lock()
                .expect("operation response mutex")
                .entry((operation.to_owned(), source_url.to_owned()))
                .or_default()
                .push_back(response);
        }

        fn gate_operation_for_source(
            &self,
            operation: &str,
            source_url: &str,
        ) -> Arc<ExecutionGate> {
            let gate = Arc::new(ExecutionGate::new());
            self.operation_gates
                .lock()
                .expect("operation gate mutex")
                .insert((operation.to_owned(), source_url.to_owned()), gate.clone());
            gate
        }

        fn content_calls(&self) -> Vec<(String, String)> {
            self.content_calls
                .lock()
                .expect("content call mutex")
                .clone()
        }
    }

    impl SourceExecutor for ControlledExecutor {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                let source_url = text_at(&request.source, &["bookSourceUrl", "sourceUrl", "url"])
                    .unwrap_or_default();
                let gate = self
                    .operation_gates
                    .lock()
                    .expect("operation gate mutex")
                    .remove(&(request.operation.clone(), source_url.clone()));
                if let Some(gate) = gate {
                    gate.entered.notify_one();
                    let permit = gate
                        .permits
                        .acquire()
                        .await
                        .map_err(|error| error.to_string())?;
                    permit.forget();
                }
                if self
                    .operation_failures
                    .lock()
                    .expect("operation failure mutex")
                    .remove(&(request.operation.clone(), source_url.clone()))
                {
                    return Err(format!(
                        "controlled {} failure for source {source_url}",
                        request.operation
                    ));
                }
                if let Some(response) = self
                    .operation_responses
                    .lock()
                    .expect("operation response mutex")
                    .get_mut(&(request.operation.clone(), source_url.clone()))
                    .and_then(VecDeque::pop_front)
                {
                    return Ok(response);
                }
                match request.operation.as_str() {
                    "exploreKinds" => Ok(json!([{
                        "title": "Fantasy",
                        "type": "text",
                        "url": "mock://catalog/fantasy",
                        "style": { "layout": "list" }
                    }])),
                    "search" => {
                        let keyword = request.keyword.unwrap_or_default();
                        let gate = self
                            .search_gates
                            .lock()
                            .expect("search gate mutex")
                            .remove(&keyword);
                        if let Some(gate) = gate {
                            gate.entered.notify_one();
                            let permit = gate
                                .permits
                                .acquire()
                                .await
                                .map_err(|error| error.to_string())?;
                            permit.forget();
                        }
                        Ok(json!({ "books": [{
                            "name": keyword,
                            "author": "Controlled executor",
                            "bookUrl": format!("mock://book/{keyword}"),
                        }] }))
                    }
                    "bookInfo" => Ok(request.book.unwrap_or_else(|| json!({}))),
                    "chapters" => {
                        let book_url = request
                            .book
                            .as_ref()
                            .and_then(|book| book.get("bookUrl"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| "mock bookUrl missing".to_owned())?
                            .to_owned();
                        let response = self
                            .catalog_responses
                            .lock()
                            .expect("catalog response mutex")
                            .get_mut(&book_url)
                            .and_then(VecDeque::pop_front);
                        if self
                            .duplicate_catalogs
                            .lock()
                            .expect("duplicate catalog mutex")
                            .remove(&book_url)
                        {
                            return Ok(json!([
                                { "title": "Repeated A", "url": format!("{book_url}/chapter/repeated") },
                                { "title": "Repeated B", "url": format!("{book_url}/chapter/repeated") }
                            ]));
                        }
                        let gate = self
                            .catalog_gates
                            .lock()
                            .expect("catalog gate mutex")
                            .remove(&book_url);
                        if let Some(gate) = gate {
                            gate.entered.notify_one();
                            let permit = gate
                                .permits
                                .acquire()
                                .await
                                .map_err(|error| error.to_string())?;
                            permit.forget();
                        }
                        if let Some(response) = response {
                            return Ok(response);
                        }
                        Ok(Value::Array(
                            (0..self.chapter_count)
                                .map(|index| {
                                    json!({
                                        "title": format!("Chapter {index}"),
                                        "url": format!("{book_url}/chapter/{index}"),
                                    })
                                })
                                .collect(),
                        ))
                    }
                    "content" => {
                        let book_url = request
                            .book
                            .as_ref()
                            .and_then(|book| book.get("bookUrl"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| "mock bookUrl missing".to_owned())?
                            .to_owned();
                        let chapter_url = request
                            .chapter
                            .as_ref()
                            .and_then(|chapter| chapter.get("url"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| "mock chapter URL missing".to_owned())?
                            .to_owned();
                        let chapter_gate = self
                            .chapter_content_gates
                            .lock()
                            .expect("chapter content gate mutex")
                            .remove(&chapter_url);
                        let gate = chapter_gate.or_else(|| {
                            self.content_gates
                                .lock()
                                .expect("content gate mutex")
                                .remove(&book_url)
                        });
                        if let Some(gate) = gate {
                            gate.entered.notify_one();
                            let permit = gate
                                .permits
                                .acquire()
                                .await
                                .map_err(|error| error.to_string())?;
                            permit.forget();
                        }
                        self.content_calls
                            .lock()
                            .expect("content call mutex")
                            .push((book_url, chapter_url.clone()));
                        if self
                            .content_failures
                            .lock()
                            .expect("content failure mutex")
                            .remove(&chapter_url)
                        {
                            return Err(format!("controlled failure for {chapter_url}"));
                        }
                        Ok(json!(format!("<p>Cached from {chapter_url}</p>")))
                    }
                    other => Err(format!("unexpected mock operation {other}")),
                }
            })
        }
    }

    struct SourceFixture {
        address: std::net::SocketAddr,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl SourceFixture {
        fn start() -> Self {
            let listener =
                TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("fixture bind");
            let address = listener.local_addr().expect("fixture address");
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread = thread::spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    let Ok((stream, _)) = listener.accept() else {
                        break;
                    };
                    if thread_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    serve_source_request(stream);
                }
            });
            Self {
                address,
                stop,
                thread: Some(thread),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.address)
        }
    }

    impl Drop for SourceFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn serve_source_request(stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
            return;
        }
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .split('?')
            .next()
            .unwrap_or("/");
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                return;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut request_body = vec![0; content_length];
        if reader.read_exact(&mut request_body).is_err() {
            return;
        }
        let body = match path {
            "/search" => {
                "<div class='item'><h3><a href='/book'>Fixture Novel</a></h3><span class='author'>A. Writer</span></div>"
            }
            "/book" => "<h1>Fixture Novel</h1><a class='toc' href='/toc'>目录</a>",
            "/toc" => {
                "<ul id='list'><li><a href='/chapter/one'>Chapter One</a></li><li><a href='/chapter/two'>Chapter Two</a></li></ul>"
            }
            "/chapter/one" => "<div class='content'>First cached chapter</div>",
            "/chapter/two" => "<div class='content'>Second cached chapter</div>",
            _ => "not found",
        };
        let status = if matches!(
            path,
            "/search" | "/book" | "/toc" | "/chapter/one" | "/chapter/two"
        ) {
            "200 OK"
        } else {
            "404 Not Found"
        };
        let mut stream = reader.into_inner();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("legado-app-flow-{}", uuid::Uuid::new_v4().simple()))
    }

    #[tokio::test]
    async fn source_change_remaps_latest_progress_bookmarks_and_survives_restart() {
        let (service, executor, root, original_source_id) = controlled_service(2).await;
        let replacement_source = json!({
            "bookSourceName": "Replacement fixture",
            "bookSourceUrl": "mock://source/replacement",
            "bookSourceType": 0,
            "ruleSearch": { "privateRule": "never expose replacement rule" }
        });
        let imported = service
            .import_sources(&json!([replacement_source]).to_string())
            .await
            .expect("import replacement source");
        let replacement_source_id = imported["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["name"] == "Replacement fixture")
            .and_then(|source| source["id"].as_str())
            .expect("replacement source ID")
            .to_owned();
        assert_eq!(imported["sources"][1]["isRss"], false);

        let book_id =
            add_controlled_book(&service, &original_source_id, "Cross-source Story").await;
        let public_book = service
            .store
            .read_json_ref(&service.store.book_ref(&book_id).unwrap())
            .await
            .unwrap();
        assert_eq!(public_book["canChangeSource"], true);
        service
            .prepare_chapters(&book_id, 0, 1)
            .await
            .expect("cache current-source chapter");
        let book_ref = service.store.book_ref(&book_id).expect("book ref");
        let before = service.store.read_json_ref(&book_ref).await.unwrap();
        let old_chapter_id = before["chapters"][0]["id"].as_str().unwrap().to_owned();
        assert!(before["chapters"][0]["src"].is_string());
        service
            .save_progress(
                &book_id,
                json!({
                    "chapterId": old_chapter_id,
                    "chapterIndex": 0,
                    "offset": 73,
                    "updatedAtMs": 1000,
                }),
            )
            .await
            .expect("save before replacement");
        crate::reading_tools::upsert_bookmark(
            &service.store,
            crate::reading_tools::BookmarkInput {
                id: Some("bookmark-before-source-change".into()),
                book_id: book_id.clone(),
                chapter_index: 0,
                offset: 17,
                note: "keep this note".into(),
            },
        )
        .await
        .expect("bookmark current chapter");

        let search = service
            .search_book_source_candidates(&book_id, &[replacement_source_id.clone()], None, 1)
            .await
            .expect("start bound replacement search");
        let task_id = search["taskId"].as_str().unwrap();
        wait_for_task_status(&service, task_id, "completed").await;
        let candidates = json_get(
            &reqwest::Client::new(),
            search["resource"]["src"].as_str().unwrap(),
        )
        .await;
        assert_eq!(candidates["complete"], true);
        assert_eq!(candidates["results"].as_array().unwrap().len(), 1);
        assert_eq!(candidates["results"][0]["sourceId"], replacement_source_id);
        assert_eq!(
            candidates["results"][0]["requiresIdentityConfirmation"],
            false
        );
        assert!(!candidates.to_string().contains("privateRule"));
        let result_id = candidates["results"][0]["resultId"]
            .as_str()
            .unwrap()
            .to_owned();

        // A delayed engine response must not overwrite progress saved while
        // the new source is resolving bookInfo.
        let gate = executor.gate_operation_for_source("bookInfo", "mock://source/replacement");
        let service_for_change = service.clone();
        let book_for_change = book_id.clone();
        let result_for_change = result_id.clone();
        let changing = tokio::spawn(async move {
            service_for_change
                .change_book_source(&book_for_change, &result_for_change, false)
                .await
        });
        gate.wait_until_entered().await;
        service
            .save_progress(
                &book_id,
                json!({
                    "chapterId": old_chapter_id,
                    "chapterIndex": 0,
                    "offset": 91,
                    "updatedAtMs": 2000,
                }),
            )
            .await
            .expect("progress remains writable during source engine request");
        gate.release();
        let changed = changing.await.unwrap().expect("confirm source replacement");
        assert_eq!(changed["progress"]["chapterIndex"], 0);
        assert_eq!(changed["progress"]["offset"], 91);
        assert_eq!(changed["bookmarks"]["migratedCount"], 1);
        assert_eq!(changed["bookmarks"]["orphanedCount"], 0);
        let after = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(after["id"], book_id);
        assert_ne!(after["chapters"][0]["id"], old_chapter_id);
        assert!(after["chapters"][0]["src"].is_null());
        assert_eq!(after["progress"]["offset"], 91);
        assert!(
            !root
                .join("books")
                .join(&book_id)
                .join("chapters")
                .join(format!("{old_chapter_id}.html"))
                .exists()
        );
        let bookmarks_ref = crate::reading_tools::bookmarks_resource(&service.store)
            .await
            .unwrap();
        let bookmarks = service.store.read_json_ref(&bookmarks_ref).await.unwrap();
        assert_eq!(bookmarks["bookmarks"][0]["chapterIndex"], 0);
        assert!(bookmarks["bookmarks"][0].get("orphaned").is_none());

        drop(service);
        let reopened = ApplicationService::open_with_executor(&root, executor)
            .await
            .expect("restart after source replacement");
        let private = reopened
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .expect("replacement private metadata after restart");
        assert_eq!(private["sourceId"], replacement_source_id);
        let reopened_book = reopened
            .store
            .read_json_ref(&reopened.store.book_ref(&book_id).unwrap())
            .await
            .unwrap();
        assert_eq!(
            reopened_book["chapters"][0]["id"],
            after["chapters"][0]["id"]
        );
        assert_eq!(reopened_book["progress"]["offset"], 91);
        reopened
            .prepare_chapters(&book_id, 0, 1)
            .await
            .expect("prepare replacement-source chapter after restart");
        let new_id = reopened_book["chapters"][0]["id"].as_str().unwrap();
        let chapter_ref = reopened.store.chapter_ref(&book_id, new_id).unwrap();
        let html = reqwest::get(reopened.resource_server().url_for(&chapter_ref))
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html.contains("mock://book/Cross-source Story/chapter/0"));
        drop(reopened);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_task_summary_includes_only_its_opaque_search_id() {
        let task = AppTask {
            id: "task-1".into(),
            kind: "search".into(),
            status: "running".into(),
            book_id: None,
            source_ids: Some(vec!["source-1".into()]),
            keyword: Some("story".into()),
            page: 1,
            from_index: 0,
            total: 1,
            completed: 0,
            check_only: false,
            search_id: Some("search-opaque-123".into()),
            result: None,
            error: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        };
        assert_eq!(task_summary(&task)["searchId"], "search-opaque-123");

        let mut other_task = task;
        other_task.kind = "chapterDownload".into();
        assert!(task_summary(&other_task).get("searchId").is_none());
    }

    #[tokio::test]
    async fn refresh_book_info_updates_display_and_private_details_without_touching_catalog_state()
    {
        let (service, executor, root, source_id) = controlled_service(2).await;
        service
            .update_source(
                &source_id,
                json!({ "name": "Controlled metadata", "group": "Fiction" }),
            )
            .await
            .expect("set source display labels");
        let book_id = add_controlled_book(&service, &source_id, "Refreshable story").await;
        service
            .prepare_chapters(&book_id, 0, 1)
            .await
            .expect("cache first chapter");
        let book_ref = service.store.book_ref(&book_id).unwrap();
        let before_book = service.store.read_json_ref(&book_ref).await.unwrap();
        let chapter_id = before_book["chapters"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let chapter_src = before_book["chapters"][0]["src"].clone();
        service
            .save_progress(
                &book_id,
                json!({
                    "chapterId": chapter_id,
                    "chapterIndex": 0,
                    "offset": 42,
                    "updatedAtMs": 1000,
                }),
            )
            .await
            .expect("save progress before metadata refresh");
        crate::reading_tools::set_book_groups(
            service.resource_store(),
            &book_id,
            vec!["Favorites".into()],
        )
        .await
        .expect("assign shelf group");

        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let mut before_private = service
            .read_private_json(&private_path)
            .await
            .expect("read private book");
        let canonical_book_url = before_private["book"]["bookUrl"].clone();
        before_private["book"]["engineContext"] = json!({ "session": "retained" });
        before_private["book"]["intro"] = json!("Cached introduction");
        service
            .write_private_json(&private_path, &before_private)
            .await
            .expect("seed engine context");
        executor.queue_operation_response_for_source(
            "bookInfo",
            "mock://source/controlled",
            json!({
                "name": "Refreshed story",
                "author": "Updated author",
                "bookUrl": "mock://different-catalog/story",
                "intro": "Updated introduction",
                "kind": ["Mystery", "Fantasy"],
                "wordCount": 12_345,
                "coverUrl": "https://example.test/refreshed-cover.jpg",
                "lastChapter": "New chapter from info",
                "newEngineContext": "merged",
                "discardedNull": null
            }),
        );

        let refreshed = service
            .refresh_book_info(&book_id)
            .await
            .expect("refresh processed book metadata");
        assert_eq!(refreshed["book"]["resourceId"], book_ref.as_str());
        let after_book = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(after_book["title"], "Refreshed story");
        assert_eq!(after_book["author"], "Updated author");
        assert_eq!(after_book["intro"], "Updated introduction");
        assert_eq!(after_book["kind"], "Mystery, Fantasy");
        assert_eq!(after_book["wordCount"], "12345");
        assert_eq!(after_book["sourceId"], source_id);
        assert_eq!(after_book["sourceName"], "Controlled metadata");
        assert_eq!(after_book["sourceGroup"], "Fiction");
        assert_eq!(after_book["progress"]["offset"], 42);
        assert_eq!(after_book["chapters"][0]["id"], chapter_id);
        assert_eq!(after_book["chapters"][0]["src"], chapter_src);
        let after_private = service
            .read_private_json(&private_path)
            .await
            .expect("read refreshed private book");
        assert_eq!(after_private["sourceId"], before_private["sourceId"]);
        assert_eq!(
            after_private["bookInstanceId"],
            before_private["bookInstanceId"]
        );
        assert_eq!(
            after_private["catalogGeneration"],
            before_private["catalogGeneration"]
        );
        assert_eq!(after_private["chapters"], before_private["chapters"]);
        assert_eq!(after_private["book"]["bookUrl"], canonical_book_url);
        assert_eq!(
            after_private["book"]["engineContext"],
            json!({ "session": "retained" })
        );
        assert_eq!(after_private["book"]["newEngineContext"], "merged");
        assert_eq!(after_private["book"]["intro"], "Updated introduction");
        assert!(after_private["book"].get("discardedNull").is_none());

        let shelf_ref = service.store.shelf_ref();
        let shelf = service.store.read_json_ref(&shelf_ref).await.unwrap();
        let shelf_book = shelf["books"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == book_id)
            .unwrap();
        assert_eq!(shelf_book["title"], "Refreshed story");
        assert_eq!(shelf_book["author"], "Updated author");
        assert_eq!(
            shelf_book["coverSrc"],
            "https://example.test/refreshed-cover.jpg"
        );
        assert_eq!(shelf_book["groups"], json!(["Favorites"]));
        let chapter_file = root
            .join("books")
            .join(&book_id)
            .join("chapters")
            .join(format!("{chapter_id}.html"));
        assert!(
            chapter_file.is_file(),
            "metadata refresh must retain cached HTML"
        );
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn refresh_book_info_does_not_overwrite_catalog_latest_chapter() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let book_id = add_controlled_book(&service, &source_id, "Catalog race story").await;
        let initial_private = service
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .unwrap();
        let book_url = initial_private["book"]["bookUrl"].as_str().unwrap();
        executor.queue_catalog_responses(
            book_url,
            vec![json!([
                { "title": "Chapter 0", "url": format!("{book_url}/chapter/0") },
                { "title": "Chapter 1", "url": format!("{book_url}/chapter/1") },
                { "title": "Chapter 2", "url": format!("{book_url}/chapter/2") }
            ])],
        );
        let gate = executor.gate_operation_for_source("bookInfo", "mock://source/controlled");
        executor.queue_operation_response_for_source(
            "bookInfo",
            "mock://source/controlled",
            json!({ "name": "Catalog race story", "lastChapter": "Stale info chapter" }),
        );
        let refresh_service = service.clone();
        let refresh_book_id = book_id.clone();
        let refreshing =
            tokio::spawn(async move { refresh_service.refresh_book_info(&refresh_book_id).await });
        gate.wait_until_entered().await;

        let catalog_task = service
            .refresh_chapters(&book_id)
            .await
            .expect("start catalog refresh during info refresh");
        let catalog_task_id = catalog_task["taskId"].as_str().unwrap();
        wait_for_task_status(&service, catalog_task_id, "completed").await;
        let latest_catalog_chapter = service
            .store
            .read_json_ref(&service.store.book_ref(&book_id).unwrap())
            .await
            .unwrap()["latestChapter"]
            .clone();
        assert_eq!(latest_catalog_chapter, "Chapter 2");

        gate.release();
        refreshing
            .await
            .unwrap()
            .expect("metadata details can commit after a catalog refresh");
        let after = service
            .store
            .read_json_ref(&service.store.book_ref(&book_id).unwrap())
            .await
            .unwrap();
        assert_eq!(after["latestChapter"], latest_catalog_chapter);
        assert_ne!(after["latestChapter"], "Stale info chapter");
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn refresh_book_info_rejects_source_reimported_during_network_request() {
        let (service, executor, root, source_id) = controlled_service(1).await;
        let book_id = add_controlled_book(&service, &source_id, "Stale refresh story").await;
        let before_book = service
            .store
            .read_json_ref(&service.store.book_ref(&book_id).unwrap())
            .await
            .unwrap();
        let gate = executor.gate_operation_for_source("bookInfo", "mock://source/controlled");
        let refresh_service = service.clone();
        let refresh_book_id = book_id.clone();
        let refreshing =
            tokio::spawn(async move { refresh_service.refresh_book_info(&refresh_book_id).await });
        gate.wait_until_entered().await;

        service
            .remove_sources(std::slice::from_ref(&source_id))
            .await
            .expect("remove source while KMP request is in flight");
        let imported = service
            .import_sources(
                &json!([{
                    "bookSourceName": "Controlled fixture",
                    "bookSourceUrl": "mock://source/controlled",
                    "bookSourceType": 0,
                    "ruleSearch": { "privateRule": "never expose" }
                }])
                .to_string(),
            )
            .await
            .expect("reimport same source ID");
        assert_eq!(imported["sources"][0]["id"], source_id);

        gate.release();
        let error = refreshing
            .await
            .unwrap()
            .expect_err("reject stale source revision");
        assert!(
            error.contains("source changed"),
            "unexpected error: {error}"
        );
        let after_book = service
            .store
            .read_json_ref(&service.store.book_ref(&book_id).unwrap())
            .await
            .unwrap();
        assert_eq!(after_book, before_book);
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn refresh_book_info_rejects_disable_instance_replacement_and_source_change() {
        let (service, executor, root, source_id) = controlled_service(1).await;
        let replacement_import = service
            .import_sources(
                &json!([{
                    "bookSourceName": "Replacement fixture",
                    "bookSourceUrl": "mock://source/replacement",
                    "bookSourceType": 0
                }])
                .to_string(),
            )
            .await
            .expect("import replacement source");
        let replacement_id = replacement_import["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["name"] == "Replacement fixture")
            .expect("replacement source record")["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let book_id = add_controlled_book(&service, &source_id, "Identity race story").await;
        let book_ref = service.store.book_ref(&book_id).unwrap();
        let before_book = service.store.read_json_ref(&book_ref).await.unwrap();

        let gate = executor.gate_operation_for_source("bookInfo", "mock://source/controlled");
        let refresh_service = service.clone();
        let refresh_book_id = book_id.clone();
        let refreshing =
            tokio::spawn(async move { refresh_service.refresh_book_info(&refresh_book_id).await });
        gate.wait_until_entered().await;
        service
            .update_source(&source_id, json!({ "enabled": false }))
            .await
            .expect("disable source while engine request is in flight");
        gate.release();
        let error = refreshing
            .await
            .unwrap()
            .expect_err("reject disabled source");
        assert!(error.contains("disabled"), "unexpected error: {error}");

        service
            .update_source(&source_id, json!({ "enabled": true }))
            .await
            .expect("re-enable source");
        let gate = executor.gate_operation_for_source("bookInfo", "mock://source/controlled");
        let refresh_service = service.clone();
        let refresh_book_id = book_id.clone();
        let refreshing =
            tokio::spawn(async move { refresh_service.refresh_book_info(&refresh_book_id).await });
        gate.wait_until_entered().await;
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let mut private = service.read_private_json(&private_path).await.unwrap();
        private["bookInstanceId"] = json!("replacement-instance");
        service
            .write_private_json(&private_path, &private)
            .await
            .expect("replace online book instance during request");
        gate.release();
        let error = refreshing
            .await
            .unwrap()
            .expect_err("reject changed book instance");
        assert!(error.contains("Book changed"), "unexpected error: {error}");

        let gate = executor.gate_operation_for_source("bookInfo", "mock://source/controlled");
        let refresh_service = service.clone();
        let refresh_book_id = book_id.clone();
        let refreshing =
            tokio::spawn(async move { refresh_service.refresh_book_info(&refresh_book_id).await });
        gate.wait_until_entered().await;
        let mut private = service.read_private_json(&private_path).await.unwrap();
        private["sourceId"] = json!(replacement_id);
        service
            .write_private_json(&private_path, &private)
            .await
            .expect("switch private source during request");
        gate.release();
        let error = refreshing
            .await
            .unwrap()
            .expect_err("reject changed source identity");
        assert!(error.contains("Book changed"), "unexpected error: {error}");
        assert_eq!(
            service.store.read_json_ref(&book_ref).await.unwrap(),
            before_book
        );
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn refresh_book_info_failure_and_disabled_source_keep_existing_details() {
        let (service, executor, root, source_id) = controlled_service(1).await;
        let book_id = add_controlled_book(&service, &source_id, "Refresh failure story").await;
        let before_book = service
            .store
            .read_json_ref(&service.store.book_ref(&book_id).unwrap())
            .await
            .unwrap();
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let before_private = service.read_private_json(&private_path).await.unwrap();
        executor.fail_operation_for_source("bookInfo", "mock://source/controlled");
        assert!(service.refresh_book_info(&book_id).await.is_err());
        assert_eq!(
            service
                .store
                .read_json_ref(&service.store.book_ref(&book_id).unwrap())
                .await
                .unwrap(),
            before_book
        );
        assert_eq!(
            service.read_private_json(&private_path).await.unwrap(),
            before_private
        );

        service
            .update_source(&source_id, json!({ "enabled": false }))
            .await
            .expect("disable source");
        let error = service.refresh_book_info(&book_id).await.unwrap_err();
        assert!(error.contains("disabled"), "unexpected error: {error}");
        assert_eq!(
            service
                .store
                .read_json_ref(&service.store.book_ref(&book_id).unwrap())
                .await
                .unwrap(),
            before_book
        );
        assert_eq!(
            service.read_private_json(&private_path).await.unwrap(),
            before_private
        );
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn book_detail_projection_updates_from_cached_engine_book_and_survives_source_removal() {
        let (service, _executor, root, source_id) = controlled_service(1).await;
        service
            .update_source(
                &source_id,
                json!({ "name": "Original metadata source", "group": "Fiction" }),
            )
            .await
            .expect("set source display labels");

        let search = service
            .search_books(&[source_id.clone()], "Metadata story", 1, |_| {})
            .await
            .expect("search metadata fixture");
        let search_ref = crate::resources::ResourceRef::new(
            search["resource"]["resourceId"]
                .as_str()
                .expect("search resource ID"),
        )
        .expect("valid search resource");
        let results = service
            .store
            .read_json_ref(&search_ref)
            .await
            .expect("search result document");
        let result_id = results["results"][0]["resultId"]
            .as_str()
            .expect("result ID")
            .to_owned();
        let candidate_path = Path::new("search-results").join(format!("{result_id}.json"));
        let mut candidate = service
            .read_private_json(&candidate_path)
            .await
            .expect("private candidate");
        candidate["book"]["intro"] = json!("<p>A readable summary</p>");
        candidate["book"]["kind"] = json!(["Fantasy", "Adventure"]);
        candidate["book"]["wordCount"] = json!(12_000);
        service
            .write_private_json(&candidate_path, &candidate)
            .await
            .expect("update fixture's processed detail");

        let added = service
            .add_book(&result_id)
            .await
            .expect("add metadata book");
        let book_ref = crate::resources::ResourceRef::new(
            added["book"]["resourceId"]
                .as_str()
                .expect("book resource ID"),
        )
        .expect("valid book resource");
        let book_id = service.store.read_json_ref(&book_ref).await.unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let added_book = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(added_book["intro"], "A readable summary");
        assert_eq!(added_book["kind"], "Fantasy, Adventure");
        assert_eq!(added_book["wordCount"], "12000");
        assert_eq!(added_book["sourceId"], source_id);
        assert_eq!(added_book["sourceName"], "Original metadata source");
        assert_eq!(added_book["sourceGroup"], "Fiction");
        assert_eq!(added_book["canChangeSource"], true);
        assert!(added_book.get("bookUrl").is_none());

        service
            .update_source(
                &source_id,
                json!({ "name": "Renamed metadata source", "group": "Drama" }),
            )
            .await
            .expect("rename current source");
        service
            .get_book(&book_id)
            .await
            .expect("refresh book projection");
        let renamed_book = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(renamed_book["sourceName"], "Renamed metadata source");
        assert_eq!(renamed_book["sourceGroup"], "Drama");

        service
            .remove_sources(&[source_id.clone()])
            .await
            .expect("remove source");
        service
            .get_book(&book_id)
            .await
            .expect("project cached detail after source removal");
        let detached_book = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(detached_book["canChangeSource"], true);
        assert_eq!(detached_book["intro"], "A readable summary");
        assert_eq!(detached_book["sourceId"], source_id);
        assert_eq!(detached_book["sourceName"], "Renamed metadata source");
        assert_eq!(detached_book["sourceGroup"], "Drama");

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn source_change_recovers_after_original_source_removal_and_survives_restart() {
        let (service, executor, root, original_source_id) = controlled_service(2).await;
        let imported = service
            .import_sources(
                &json!([{
                    "bookSourceName": "Replacement fixture",
                    "bookSourceUrl": "mock://source/replacement-after-delete",
                    "bookSourceType": 0
                }])
                .to_string(),
            )
            .await
            .expect("import replacement source");
        let replacement_source_id = imported["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["name"] == "Replacement fixture")
            .and_then(|source| source["id"].as_str())
            .expect("replacement source ID")
            .to_owned();
        let book_id = add_controlled_book(&service, &original_source_id, "Recoverable story").await;
        let book_ref = service.store.book_ref(&book_id).unwrap();
        let mut legacy_book = service.store.read_json_ref(&book_ref).await.unwrap();
        legacy_book
            .as_object_mut()
            .unwrap()
            .remove("canChangeSource");
        service
            .store
            .write_json_ref(&book_ref, &legacy_book)
            .await
            .unwrap();
        service
            .remove_sources(&[original_source_id.clone()])
            .await
            .unwrap();

        let refreshed_descriptor = service.get_book(&book_id).await.expect("get online book");
        let book_ref = service.store.book_ref(&book_id).unwrap();
        let public_book = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(public_book["canChangeSource"], true);
        assert_eq!(
            refreshed_descriptor["resourceId"],
            format!("resource://books/{book_id}/book.json")
        );
        let search = service
            .search_book_source_candidates(&book_id, &[replacement_source_id.clone()], None, 1)
            .await
            .expect("search with removed original source");
        wait_for_task_status(&service, search["taskId"].as_str().unwrap(), "completed").await;
        let candidates = json_get(
            &reqwest::Client::new(),
            search["resource"]["src"].as_str().unwrap(),
        )
        .await;
        let result_id = candidates["results"][0]["resultId"]
            .as_str()
            .expect("candidate result ID");
        let changed = service
            .change_book_source(&book_id, result_id, false)
            .await
            .expect("switch source after original source removal");
        assert_eq!(changed["book"]["resourceId"], book_ref.as_str());

        let private = service
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .unwrap();
        assert_eq!(private["sourceId"], replacement_source_id);
        let switched = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(switched["id"], book_id);
        assert_eq!(switched["canChangeSource"], true);
        assert_eq!(switched["sourceId"], replacement_source_id);
        assert_eq!(switched["sourceName"], "Replacement fixture");
        assert!(switched.get("sourceGroup").is_none());

        drop(service);
        let reopened = ApplicationService::open_with_executor(&root, executor)
            .await
            .expect("restart after recovered source switch");
        let reopened_private = reopened
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .unwrap();
        assert_eq!(reopened_private["sourceId"], replacement_source_id);
        assert_eq!(
            reopened
                .store
                .read_json_ref(&reopened.store.book_ref(&book_id).unwrap())
                .await
                .unwrap()["canChangeSource"],
            true
        );
        drop(reopened);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reimporting_a_removed_original_source_invalidates_old_replacement_candidates() {
        let (service, _executor, root, original_source_id) = controlled_service(1).await;
        let original_definition = json!({
            "bookSourceName": "Original fixture",
            "bookSourceUrl": "mock://source/controlled",
            "bookSourceType": 0,
            "ruleSearch": { "privateRule": "never expose" }
        });
        let replacement_definition = json!({
            "bookSourceName": "Replacement fixture",
            "bookSourceUrl": "mock://source/reimport-replacement",
            "bookSourceType": 0
        });
        let imported = service
            .import_sources(&json!([replacement_definition]).to_string())
            .await
            .unwrap();
        let replacement_source_id = imported["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["name"] == "Replacement fixture")
            .and_then(|source| source["id"].as_str())
            .unwrap()
            .to_owned();
        let book_id =
            add_controlled_book(&service, &original_source_id, "Stale absent story").await;
        service
            .remove_sources(&[original_source_id.clone()])
            .await
            .unwrap();
        let search = service
            .search_book_source_candidates(&book_id, &[replacement_source_id], None, 1)
            .await
            .expect("candidate search while original source is absent");
        wait_for_task_status(&service, search["taskId"].as_str().unwrap(), "completed").await;
        let candidates = json_get(
            &reqwest::Client::new(),
            search["resource"]["src"].as_str().unwrap(),
        )
        .await;
        let result_id = candidates["results"][0]["resultId"]
            .as_str()
            .unwrap()
            .to_owned();
        let book_ref = service.store.book_ref(&book_id).unwrap();
        let before = service.store.read_json_ref(&book_ref).await.unwrap();

        // The stable source ID is derived from its URL. Reimporting the same
        // definition restores the old ID but still invalidates the absent
        // source snapshot.
        let reimported = service
            .import_sources(&json!([original_definition]).to_string())
            .await
            .unwrap();
        assert!(
            reimported["sources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|source| source["id"] == original_source_id)
        );
        let error = service
            .change_book_source(&book_id, &result_id, false)
            .await
            .unwrap_err();
        assert!(error.contains("availability changed"), "{error}");
        assert_eq!(
            service.store.read_json_ref(&book_ref).await.unwrap(),
            before
        );
        let private = service
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .unwrap();
        assert_eq!(private["sourceId"], original_source_id);

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_stale_or_unbound_source_change_keeps_original_book_and_cache() {
        let (service, executor, root, original_source_id) = controlled_service(1).await;
        let source_definition = |rule_value: &str| {
            json!({
                "bookSourceName": "Replacement fixture",
                "bookSourceUrl": "mock://source/replacement-fails",
                "bookSourceType": 0,
                "ruleSearch": { "privateRule": rule_value }
            })
        };
        let imported = service
            .import_sources(&json!([source_definition("original private rule")]).to_string())
            .await
            .unwrap();
        let replacement_source_id = imported["sources"][1]["id"].as_str().unwrap().to_owned();
        let book_id =
            add_controlled_book(&service, &original_source_id, "Failure preserves book").await;
        service.prepare_chapters(&book_id, 0, 1).await.unwrap();
        let book_ref = service.store.book_ref(&book_id).unwrap();
        let before = service.store.read_json_ref(&book_ref).await.unwrap();
        let old_chapter_id = before["chapters"][0]["id"].as_str().unwrap().to_owned();

        let search = service
            .search_book_source_candidates(&book_id, &[replacement_source_id.clone()], None, 1)
            .await
            .unwrap();
        wait_for_task_status(&service, search["taskId"].as_str().unwrap(), "completed").await;
        let candidate_document = json_get(
            &reqwest::Client::new(),
            search["resource"]["src"].as_str().unwrap(),
        )
        .await;
        let candidate_id = candidate_document["results"][0]["resultId"]
            .as_str()
            .unwrap()
            .to_owned();

        let normal_search = service
            .search_books(
                &[replacement_source_id.clone()],
                "Failure preserves book",
                1,
                |_| {},
            )
            .await
            .unwrap();
        let normal_document = json_get(
            &reqwest::Client::new(),
            normal_search["resource"]["src"].as_str().unwrap(),
        )
        .await;
        let normal_result_id = normal_document["results"][0]["resultId"].as_str().unwrap();
        assert!(
            service
                .change_book_source(&book_id, normal_result_id, false)
                .await
                .unwrap_err()
                .contains("not a replacement candidate")
        );

        service
            .import_sources(&json!([source_definition("updated private rule")]).to_string())
            .await
            .unwrap();
        assert!(
            service
                .change_book_source(&book_id, &candidate_id, false)
                .await
                .unwrap_err()
                .contains("Replacement source changed")
        );

        let search = service
            .search_book_source_candidates(&book_id, &[replacement_source_id.clone()], None, 1)
            .await
            .unwrap();
        wait_for_task_status(&service, search["taskId"].as_str().unwrap(), "completed").await;
        let candidate_document = json_get(
            &reqwest::Client::new(),
            search["resource"]["src"].as_str().unwrap(),
        )
        .await;
        let candidate_id = candidate_document["results"][0]["resultId"]
            .as_str()
            .unwrap();
        executor.fail_operation_for_source("bookInfo", "mock://source/replacement-fails");
        assert!(
            service
                .change_book_source(&book_id, candidate_id, false)
                .await
                .unwrap_err()
                .contains("controlled bookInfo failure")
        );
        let after_failure = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(after_failure, before);
        let private = service
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .unwrap();
        assert_eq!(private["sourceId"], original_source_id);
        assert!(
            root.join("books")
                .join(&book_id)
                .join("chapters")
                .join(format!("{old_chapter_id}.html"))
                .exists()
        );

        service.remove_book(&book_id).await.unwrap();
        let readded_id =
            add_controlled_book(&service, &original_source_id, "Failure preserves book").await;
        assert_eq!(readded_id, book_id);
        assert!(
            service
                .change_book_source(&book_id, candidate_id, false)
                .await
                .unwrap_err()
                .contains("Book catalog changed after the search")
        );
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn search_history_tracks_user_searches_but_not_replacement_searches() {
        let (service, _executor, root, source_id) = controlled_service(2).await;
        let bootstrap = service.bootstrap().await.expect("bootstrap resources");
        assert_eq!(
            bootstrap["searchHistory"]["resourceId"],
            "resource://reading/search-history.json"
        );
        let client = reqwest::Client::new();
        let initial = json_get(&client, bootstrap["searchHistory"]["src"].as_str().unwrap()).await;
        assert_eq!(initial["entries"], json!([]));

        let feed = service
            .import_sources(
                &json!([{
                    "sourceName": "Search history feed fixture",
                    "sourceUrl": "https://feed.example.test/search-history.xml",
                    "bookSourceType": 5
                }])
                .to_string(),
            )
            .await
            .expect("import RSS source");
        let rss_source_id = feed["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["name"] == "Search history feed fixture")
            .and_then(|source| source["id"].as_str())
            .expect("RSS source ID")
            .to_owned();
        assert!(
            service
                .start_search(&[rss_source_id.clone()], "RSS must not search", 1)
                .await
                .unwrap_err()
                .contains("No enabled book sources")
        );
        assert!(
            service
                .search_books(&[rss_source_id], "RSS must not search", 1, |_| {})
                .await
                .unwrap_err()
                .contains("No enabled book sources")
        );
        assert!(
            crate::search_history::load(&service.store)
                .await
                .expect("history stays empty after rejected RSS searches")
                .entries
                .is_empty()
        );

        let started = service
            .start_search(&[], "  Alpha   Beta  ", 1)
            .await
            .expect("start user search");
        wait_for_task_status(&service, started["taskId"].as_str().unwrap(), "completed").await;
        let search_ref =
            crate::resources::ResourceRef::new(started["resource"]["resourceId"].as_str().unwrap())
                .expect("search resource ref");
        let search_document = service
            .store
            .read_json_ref(&search_ref)
            .await
            .expect("completed user search resource");
        assert!(
            search_document["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|result| { result["sourceId"].as_str() == Some(source_id.as_str()) })
        );
        let after_async = crate::search_history::load(&service.store)
            .await
            .expect("history after task search");
        assert_eq!(after_async.entries.len(), 1);
        assert_eq!(after_async.entries[0].query, "Alpha   Beta");
        assert_eq!(after_async.entries[0].usage, 1);

        service
            .search_books(&[], "ALPHA BETA", 1, |_| {})
            .await
            .expect("run synchronous user search");
        let after_sync = crate::search_history::load(&service.store)
            .await
            .expect("history after synchronous search");
        assert_eq!(after_sync.entries.len(), 1);
        assert_eq!(after_sync.entries[0].query, "ALPHA BETA");
        assert_eq!(after_sync.entries[0].usage, 2);
        assert_eq!(after_sync.processed_search_ids.len(), 2);

        let replacement_source = service
            .import_sources(
                &json!([{
                    "bookSourceName": "Replacement fixture",
                    "bookSourceUrl": "mock://source/replacement",
                    "bookSourceType": 0
                }])
                .to_string(),
            )
            .await
            .expect("import replacement source");
        let replacement_source_id = replacement_source["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["name"] == "Replacement fixture")
            .and_then(|source| source["id"].as_str())
            .expect("replacement source ID")
            .to_owned();
        let book_id = add_controlled_book(&service, &source_id, "Bound story").await;
        let before_internal_search = crate::search_history::load(&service.store)
            .await
            .expect("history before replacement search");
        let candidates = service
            .search_book_source_candidates(&book_id, &[replacement_source_id], None, 1)
            .await
            .expect("search source replacement candidates");
        wait_for_task_status(
            &service,
            candidates["taskId"].as_str().unwrap(),
            "completed",
        )
        .await;
        let after_internal_search = crate::search_history::load(&service.store)
            .await
            .expect("history after replacement search");
        assert_eq!(after_internal_search, before_internal_search);

        let deleted = service
            .delete_search_history("alpha\t beta")
            .await
            .expect("delete query history");
        let deleted_doc = json_get(&client, deleted["src"].as_str().unwrap()).await;
        assert_eq!(deleted_doc["entries"].as_array().unwrap().len(), 1);
        let cleared = service
            .clear_search_history()
            .await
            .expect("clear search history");
        let cleared_doc = json_get(&client, cleared["src"].as_str().unwrap()).await;
        assert_eq!(cleared_doc["entries"], json!([]));

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn home_rss_and_txt_toc_service_apis_persist_public_resources() {
        let (service, _executor, root, source_id) = controlled_service(2).await;
        let client = reqwest::Client::new();

        let categories = crate::discovery::list_categories(&service, &source_id, Some(false))
            .await
            .expect("processed categories");
        let category_ref = service
            .store
            .discovery_ref(&source_id)
            .expect("category resource ref");
        let category_document = service
            .store
            .read_json_ref(&category_ref)
            .await
            .expect("category document");
        let category_id = category_document["categories"][0]["categoryId"]
            .as_str()
            .expect("stable public category ID")
            .to_owned();

        let initial_home_ref = service
            .get_home_config()
            .await
            .expect("get home descriptor");
        let home_resource_id = initial_home_ref["resourceId"]
            .as_str()
            .expect("home resource ID");
        let home_ref =
            crate::resources::ResourceRef::new(home_resource_id).expect("valid home resource ref");
        let initial_home = service
            .store
            .read_json_ref(&home_ref)
            .await
            .expect("initialized home config");
        assert_eq!(initial_home["tabs"][0]["title"], "主页");

        let config: crate::discovery::home_config::HomeConfigDocument =
            serde_json::from_value(json!({
                "schemaVersion": crate::models::CURRENT_SCHEMA_VERSION,
                "tabs": [{
                    "id": "tab-main",
                    "title": "首页",
                    "sortOrder": 0,
                    "sections": [{
                        "id": "section-featured",
                        "title": "精选",
                        "sourceId": source_id,
                        "sourceName": "UI-supplied source name",
                        "categoryId": category_id,
                        "categoryName": "UI-supplied category name",
                        "style": 0,
                        "sortOrder": 0,
                        "coverVideo": false
                    }]
                }]
            }))
            .expect("valid processed home DTO");
        let mut invalid_home = config.clone();
        invalid_home.tabs[0].sections[0].source_id = "source-does-not-exist".to_owned();
        let saved_home_ref = service
            .save_home_config(config)
            .await
            .expect("save home config");
        let saved_home = json_get(&client, saved_home_ref["src"].as_str().unwrap()).await;
        assert_eq!(
            saved_home["tabs"][0]["sections"][0]["sourceName"],
            "Controlled fixture"
        );
        assert_eq!(
            saved_home["tabs"][0]["sections"][0]["categoryName"],
            "Fantasy"
        );
        assert!(!saved_home.to_string().contains("mock://catalog/fantasy"));
        assert!(!saved_home.to_string().contains("privateRule"));
        assert!(categories["resource"]["src"].as_str().is_some());
        assert!(service.save_home_config(invalid_home).await.is_err());
        assert_eq!(
            service.store.read_json_ref(&home_ref).await.unwrap(),
            saved_home
        );

        let rss_source = service
            .import_sources(
                &json!([{
                    "sourceName": "Fixture feed",
                    "sourceUrl": "https://feed.example.test/rss",
                    "bookSourceType": 5
                }])
                .to_string(),
            )
            .await
            .expect("import processed RSS source");
        let rss_source_id = rss_source["sources"]
            .as_array()
            .and_then(|sources| sources.last())
            .and_then(|source| source.get("id"))
            .and_then(Value::as_str)
            .expect("RSS source ID")
            .to_owned();
        let source_list = service
            .list_sources()
            .await
            .expect("public source metadata");
        let rss_metadata = source_list["sources"]
            .as_array()
            .and_then(|sources| sources.iter().find(|source| source["id"] == rss_source_id))
            .expect("public RSS metadata");
        assert_eq!(rss_metadata["isRss"], true);
        assert!(rss_metadata.get("sourceUrl").is_none());
        assert!(!rss_metadata.to_string().contains("feed.example.test"));
        let stored_rss_source = service
            .source_record(&rss_source_id)
            .await
            .expect("stored RSS source");
        assert!(
            crate::rss::is_rss_source(&stored_rss_source.source),
            "expected RSS envelope, got {}",
            stored_rss_source.source
        );
        let rss_resource = service.get_rss_state().await.expect("RSS state descriptor");
        let rss_resource_id = rss_resource["resourceId"]
            .as_str()
            .expect("RSS state resource ID");
        let rss_ref =
            crate::resources::ResourceRef::new(rss_resource_id).expect("valid RSS state ref");
        service
            .set_rss_filter(&rss_source_id, "favorites")
            .await
            .expect("set RSS filter");
        let article_id = "article-00000000000000000000000000000001";
        let mut rss_state = service
            .store
            .read_json_ref(&rss_ref)
            .await
            .expect("RSS state document");
        rss_state["articles"] = json!([{
            "sourceId": rss_source_id,
            "articleId": article_id,
            "isRead": false,
            "isFavorite": false,
            "updatedAtMs": 1
        }]);
        service
            .store
            .write_json_ref(&rss_ref, &rss_state)
            .await
            .expect("seed one already processed RSS article");
        service
            .set_rss_article_state(&rss_source_id, article_id, Some(true), Some(true))
            .await
            .expect("update RSS read/favorite flags");
        let changed_state = service
            .store
            .read_json_ref(&rss_ref)
            .await
            .expect("updated RSS state");
        assert_eq!(changed_state["subscriptions"][0]["filter"], "favorites");
        assert_eq!(changed_state["articles"][0]["isRead"], true);
        assert_eq!(changed_state["articles"][0]["isFavorite"], true);
        assert!(!changed_state.to_string().contains("feed.example.test"));
        assert!(
            service
                .set_rss_filter(&rss_source_id, "unknown-filter")
                .await
                .is_err()
        );
        assert!(
            service
                .set_rss_article_state(&rss_source_id, "invalid", Some(false), None)
                .await
                .is_err()
        );

        let unsubscribe = service
            .unsubscribe_rss(&rss_source_id)
            .await
            .expect("unsubscribe RSS source and clear state");
        assert_eq!(unsubscribe["sources"].as_array().unwrap().len(), 1);
        assert_eq!(unsubscribe["sources"][0]["id"], source_id);
        let cleared_rss_state = service
            .store
            .read_json_ref(&rss_ref)
            .await
            .expect("cleared RSS state");
        assert!(
            cleared_rss_state["subscriptions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(cleared_rss_state["articles"].as_array().unwrap().is_empty());
        assert!(service.source_record(&rss_source_id).await.is_err());

        let initial_toc = service
            .get_txt_toc_rules()
            .await
            .expect("TXT TOC resource descriptor");
        let toc_ref = crate::resources::ResourceRef::new(
            initial_toc["resourceId"]
                .as_str()
                .expect("TXT TOC resource ID"),
        )
        .expect("valid TXT TOC resource ref");
        let initial_toc_document = service
            .store
            .read_json_ref(&toc_ref)
            .await
            .expect("initialized TXT TOC JSON");
        assert!(initial_toc_document["rules"].as_array().unwrap().is_empty());
        let rule = crate::local_books::txt_toc_rules::TxtTocRule {
            id: "custom-heading".to_owned(),
            name: "Chapter headings".to_owned(),
            rule: r"^第[0-9一二三四五六七八九十]+章\s+(.+)$".to_owned(),
            example: Some("第一章 开始".to_owned()),
            serial_number: 0,
            enable: true,
        };
        let saved_toc = service
            .upsert_txt_toc_rule(rule.clone())
            .await
            .expect("save TXT TOC rule");
        let persisted_toc = json_get(&client, saved_toc["src"].as_str().unwrap()).await;
        assert_eq!(persisted_toc["rules"][0]["id"], "custom-heading");
        assert_eq!(persisted_toc["rules"][0]["rule"], rule.rule);
        let invalid_rule = crate::local_books::txt_toc_rules::TxtTocRule {
            id: "broken-pattern".to_owned(),
            name: "Broken".to_owned(),
            rule: "[".to_owned(),
            example: None,
            serial_number: 1,
            enable: true,
        };
        assert!(service.upsert_txt_toc_rule(invalid_rule).await.is_err());
        assert_eq!(
            service.store.read_json_ref(&toc_ref).await.unwrap(),
            persisted_toc
        );
        drop(service);
        let reopened = ApplicationService::open_with_executor(
            &root,
            std::sync::Arc::new(ControlledExecutor::new(2)),
        )
        .await
        .expect("reopen service for persisted resource");
        let reopened_toc = reopened
            .get_txt_toc_rules()
            .await
            .expect("read persisted TXT TOC descriptor");
        let persisted_after_reopen = json_get(&client, reopened_toc["src"].as_str().unwrap()).await;
        assert_eq!(persisted_after_reopen, persisted_toc);
        reopened
            .delete_txt_toc_rule("custom-heading")
            .await
            .expect("delete TXT TOC rule");
        assert!(
            reopened.store.read_json_ref(&toc_ref).await.unwrap()["rules"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        drop(reopened);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn settings_round_trip_migrates_legacy_fields_and_validates_ui_ranges() {
        let root = temp_root();
        let service =
            ApplicationService::open_with_executor(&root, Arc::new(ControlledExecutor::new(1)))
                .await
                .expect("application service");

        service
            .save_settings(json!({
                "reader": {
                    "fontSize": 19,
                    "lineHeight": 1.8,
                    "preloadCount": 5,
                    "theme": "light"
                }
            }))
            .await
            .expect("legacy settings should migrate");
        let settings_ref = service.store.settings_ref();
        let saved = service
            .store
            .read_json_ref(&settings_ref)
            .await
            .expect("saved settings");
        assert_eq!(saved["reader"]["fontSizePx"], 19);
        assert!(saved["reader"].get("fontSize").is_none());
        assert_eq!(saved["reader"]["theme"], "system");
        assert_eq!(saved["reader"]["replacements"], json!([]));

        assert!(
            service
                .save_settings(json!({ "reader": { "fontSizePx": 37 } }))
                .await
                .is_err()
        );
        assert!(
            service
                .save_settings(json!({ "reader": { "fontSizePx": "20" } }))
                .await
                .is_err()
        );
        assert!(
            service
                .save_settings(json!({ "reader": { "lineHeight": 2.9 } }))
                .await
                .is_err()
        );
        assert!(
            service
                .save_settings(json!({ "reader": { "preloadCount": 5.5 } }))
                .await
                .is_err()
        );
        assert_eq!(
            service
                .store
                .read_json_ref(&settings_ref)
                .await
                .expect("settings remain unchanged after rejected input"),
            saved
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reader_defaults_use_canonical_fields_and_reject_unsafe_css_values() {
        let defaults = super::reader_defaults(Some(json!({
            "reader": {
                "fontSizePx": 28,
                "fontSize": 13,
                "lineHeight": 2.4,
                "fontFamily": "mono",
                "textColor": "red",
                "backgroundColor": "url(javascript:alert(1))",
                "textAlign": "center",
                "theme": "dark",
                "preloadCount": 9,
                "replacements": [{ "find": "x", "replace": "y" }]
            }
        })));
        assert_eq!(defaults.font_size_px, 28.0);
        assert_eq!(defaults.line_height, 2.4);
        assert_eq!(defaults.font_family, "monospace");
        assert_eq!(defaults.text_color, "red");
        assert_eq!(defaults.background_color, "#f7f3e9");
        assert_eq!(defaults.text_align, "center");
        assert_eq!(defaults.theme, crate::models::ReaderTheme::Dark);
        assert_eq!(defaults.preload_count, 9);
        assert!(defaults.replacements.is_empty());
    }

    async fn json_get(client: &reqwest::Client, url: &str) -> Value {
        let text = client
            .get(url)
            .send()
            .await
            .expect("resource request")
            .error_for_status()
            .expect("resource status")
            .text()
            .await
            .expect("resource body");
        serde_json::from_str(&text).expect("resource JSON")
    }

    async fn controlled_service(
        chapter_count: usize,
    ) -> (ApplicationService, Arc<ControlledExecutor>, PathBuf, String) {
        let root = temp_root();
        let executor = Arc::new(ControlledExecutor::new(chapter_count));
        let service = ApplicationService::open_with_executor(&root, executor.clone())
            .await
            .expect("controlled application service");
        let imported = service
            .import_sources(
                &json!([{
                    "bookSourceName": "Controlled fixture",
                    "bookSourceUrl": "mock://source/controlled",
                    "bookSourceType": 0,
                    "ruleSearch": { "privateRule": "never expose" }
                }])
                .to_string(),
            )
            .await
            .expect("import controlled source");
        let source_id = imported["sources"][0]["id"]
            .as_str()
            .expect("controlled source ID")
            .to_owned();
        (service, executor, root, source_id)
    }

    async fn add_controlled_book(
        service: &ApplicationService,
        source_id: &str,
        keyword: &str,
    ) -> String {
        let search = service
            .search_books(&[source_id.to_owned()], keyword, 1, |_| {})
            .await
            .expect("controlled search");
        let search_ref = crate::resources::ResourceRef::new(
            search["resource"]["resourceId"]
                .as_str()
                .expect("search resource ID"),
        )
        .expect("valid search resource");
        let document = service
            .store
            .read_json_ref(&search_ref)
            .await
            .expect("controlled search results");
        let result_id = document["results"][0]["resultId"]
            .as_str()
            .expect("search result ID");
        let added = service
            .add_book(result_id)
            .await
            .expect("add controlled book");
        let book_ref = crate::resources::ResourceRef::new(
            added["book"]["resourceId"]
                .as_str()
                .expect("book resource ID"),
        )
        .expect("valid book resource");
        service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("controlled book document")["id"]
            .as_str()
            .expect("book ID")
            .to_owned()
    }

    async fn wait_for_task<F>(service: &ApplicationService, task_id: &str, predicate: F) -> AppTask
    where
        F: Fn(&AppTask) -> bool,
    {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let task = service
                    .task_summary_record(task_id)
                    .await
                    .expect("task must remain available");
                if predicate(&task) {
                    return task;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("task should reach expected state")
    }

    async fn wait_for_task_status(
        service: &ApplicationService,
        task_id: &str,
        status: &str,
    ) -> AppTask {
        wait_for_task(service, task_id, |task| task.status == status).await
    }

    fn task_id_from_start(result: &Value) -> String {
        result["taskId"]
            .as_str()
            .expect("task ID returned from start")
            .to_owned()
    }

    async fn wait_for_task_worker_exit(service: &ApplicationService, task_id: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !service.tasks.lock().await.signals.contains_key(task_id) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("task worker should release its control signal");
    }

    #[tokio::test]
    async fn chapter_download_task_processes_more_than_fifty_requested_chapters() {
        let (service, executor, root, source_id) = controlled_service(61).await;
        let book_id = add_controlled_book(&service, &source_id, "long-book").await;
        let started = service
            .start_chapter_download(&book_id, 0, 61)
            .await
            .expect("start full-book download");
        let task_id = task_id_from_start(&started);
        let completed = wait_for_task_status(&service, &task_id, "completed").await;
        assert_eq!(completed.total, 61);
        assert_eq!(completed.completed, 61);
        assert_eq!(executor.content_calls().len(), 61);

        let book_ref = service.store.book_ref(&book_id).expect("book resource");
        let book = service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("downloaded book document");
        assert_eq!(book["chapters"].as_array().unwrap().len(), 61);
        assert!(
            book["chapters"]
                .as_array()
                .unwrap()
                .iter()
                .all(|chapter| chapter["src"].is_string())
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn pausing_a_download_releases_its_worker_slot_for_another_task() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let book_a = add_controlled_book(&service, &source_id, "paused-a").await;
        let book_b = add_controlled_book(&service, &source_id, "available-b").await;
        let book_c = add_controlled_book(&service, &source_id, "blocked-c").await;
        let gate_a = executor.gate_content("mock://book/paused-a");
        let gate_c = executor.gate_content("mock://book/blocked-c");

        let task_a = task_id_from_start(
            &service
                .start_chapter_download(&book_a, 0, 2)
                .await
                .expect("start task A"),
        );
        gate_a.wait_until_entered().await;
        let task_c = task_id_from_start(
            &service
                .start_chapter_download(&book_c, 0, 1)
                .await
                .expect("start task C"),
        );
        gate_c.wait_until_entered().await;

        service.pause_task(&task_a).await.expect("pause task A");
        gate_a.release();
        let paused = wait_for_task_status(&service, &task_a, "paused").await;
        assert_eq!(paused.completed, 1);

        let task_b = task_id_from_start(
            &service
                .start_chapter_download(&book_b, 0, 1)
                .await
                .expect("start task B while A is paused and C is blocked"),
        );
        let completed_b = wait_for_task_status(&service, &task_b, "completed").await;
        assert_eq!(completed_b.completed, 1);
        assert_eq!(
            service
                .task_summary_record(&task_a)
                .await
                .expect("paused task A")
                .status,
            "paused"
        );

        gate_c.release();
        wait_for_task_status(&service, &task_c, "completed").await;
        service.resume_task(&task_a).await.expect("resume task A");
        let completed_a = wait_for_task_status(&service, &task_a, "completed").await;
        assert_eq!(completed_a.completed, 2);

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancelling_a_delayed_search_discards_its_late_results() {
        let (service, executor, root, source_id) = controlled_service(1).await;
        let gate = executor.gate_search("cancel-me");
        let started = service
            .start_search(&[source_id], "cancel-me", 1)
            .await
            .expect("start delayed search");
        let task_id = task_id_from_start(&started);
        gate.wait_until_entered().await;
        service.cancel_task(&task_id).await.expect("cancel search");
        gate.release();
        let cancelled = wait_for_task_status(&service, &task_id, "cancelled").await;
        assert_eq!(cancelled.completed, 0);

        let search_ref = crate::resources::ResourceRef::new(
            started["resource"]["resourceId"]
                .as_str()
                .expect("search resource ID"),
        )
        .expect("valid search resource");
        let document = service
            .store
            .read_json_ref(&search_ref)
            .await
            .expect("cancelled search document");
        assert_eq!(document["complete"], true);
        assert_eq!(document["cancelled"], true);
        assert!(document["results"].as_array().unwrap().is_empty());

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn interrupted_download_resumes_from_its_persisted_chapter_cursor() {
        let (service, executor, root, source_id) = controlled_service(3).await;
        let book_id = add_controlled_book(&service, &source_id, "restart-book").await;
        let gate = executor.gate_content("mock://book/restart-book");
        let task_id = task_id_from_start(
            &service
                .start_chapter_download(&book_id, 0, 3)
                .await
                .expect("start restartable download"),
        );
        gate.wait_until_entered().await;
        service.pause_task(&task_id).await.expect("pause download");
        gate.release();
        let paused = wait_for_task_status(&service, &task_id, "paused").await;
        assert_eq!(paused.completed, 1);
        service
            .cancel_task(&task_id)
            .await
            .expect("stop old worker");
        wait_for_task_status(&service, &task_id, "cancelled").await;
        wait_for_task_worker_exit(&service, &task_id).await;

        let tasks_ref = service.store.reading_ref("tasks").expect("tasks resource");
        service
            .store
            .update_json_ref(&tasks_ref, |mut document| {
                let task = document["tasks"]
                    .as_array_mut()
                    .and_then(|tasks| tasks.iter_mut().find(|task| task["id"] == task_id))
                    .ok_or_else(|| "restart test task missing".to_owned())?;
                task["status"] = json!("running");
                Ok(document)
            })
            .await
            .expect("simulate process interruption on disk");
        drop(service);

        let restarted = ApplicationService::open_with_executor(&root, executor.clone())
            .await
            .expect("reopen after interruption");
        let interrupted = restarted
            .task_summary_record(&task_id)
            .await
            .expect("restored task history");
        assert_eq!(interrupted.status, "interrupted");
        assert_eq!(interrupted.completed, 1);

        restarted
            .resume_task(&task_id)
            .await
            .expect("resume interrupted task");
        let completed = wait_for_task_status(&restarted, &task_id, "completed").await;
        assert_eq!(completed.completed, 3);
        let calls = executor.content_calls();
        for chapter_index in 0..3 {
            let chapter_url = format!("mock://book/restart-book/chapter/{chapter_index}");
            assert_eq!(
                calls.iter().filter(|(_, url)| url == &chapter_url).count(),
                1,
                "chapter {chapter_index} must be prepared exactly once"
            );
        }

        drop(restarted);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn restore_excludes_new_operations_drains_workers_and_keeps_archived_task_history() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let snapshot_book = add_controlled_book(&service, &source_id, "snapshot-book").await;
        let archived_task = task_id_from_start(
            &service
                .start_chapter_download(&snapshot_book, 0, 1)
                .await
                .expect("start archived task"),
        );
        wait_for_task_status(&service, &archived_task, "completed").await;
        service
            .save_progress(
                &snapshot_book,
                json!({
                    "chapterIndex": 0,
                    "offset": 7,
                    "updatedAtMs": 1_800_000_000_001u64,
                }),
            )
            .await
            .expect("persist snapshot progress");
        crate::reading_tools::record_reading_session(
            &service.store,
            crate::reading_tools::ReadingSession {
                book_id: snapshot_book.clone(),
                session_id: "before-restore".to_owned(),
                duration_ms: 1_000,
                ended_at_ms: 10_000,
                utc_offset_minutes: 0,
            },
        )
        .await
        .expect("persist snapshot history");

        let archive = root.with_extension("zip");
        service
            .create_backup(&archive)
            .await
            .expect("create snapshot backup");
        service
            .save_progress(
                &snapshot_book,
                json!({
                    "chapterIndex": 0,
                    "offset": 99,
                    "updatedAtMs": 1_800_000_000_099u64,
                }),
            )
            .await
            .expect("write data after snapshot");

        let active_book = add_controlled_book(&service, &source_id, "post-snapshot-book").await;
        let active_gate = executor.gate_content("mock://book/post-snapshot-book");
        let active_task = task_id_from_start(
            &service
                .start_chapter_download(&active_book, 0, 1)
                .await
                .expect("start active download"),
        );
        active_gate.wait_until_entered().await;

        let restoring_service = service.clone();
        let restoring_archive = archive.clone();
        let restore =
            tokio::spawn(async move { restoring_service.restore_backup(&restoring_archive).await });
        wait_for_task_status(&service, &active_task, "cancelling").await;

        let progress_service = service.clone();
        let progress_book = snapshot_book.clone();
        let progress_write = tokio::spawn(async move {
            progress_service
                .save_progress(
                    &progress_book,
                    json!({
                        "chapterIndex": 0,
                        "offset": 55,
                        "updatedAtMs": 1_800_000_000_155u64,
                    }),
                )
                .await
        });
        let history_service = service.clone();
        let history_book = snapshot_book.clone();
        let history_write = tokio::spawn(async move {
            let _operation = history_service.operation_read().await;
            crate::reading_tools::record_reading_session(
                &history_service.store,
                crate::reading_tools::ReadingSession {
                    book_id: history_book,
                    session_id: "after-restore".to_owned(),
                    duration_ms: 2_000,
                    ended_at_ms: 20_000,
                    utc_offset_minutes: 0,
                },
            )
            .await
        });
        let start_service = service.clone();
        let start_book = active_book.clone();
        let blocked_start = tokio::spawn(async move {
            start_service
                .start_chapter_download(&start_book, 0, 1)
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !restore.is_finished(),
            "restore must drain the active engine call"
        );
        assert!(
            !progress_write.is_finished(),
            "progress mutation must wait behind restore"
        );
        assert!(
            !history_write.is_finished(),
            "history mutation must wait behind restore"
        );
        assert!(
            !blocked_start.is_finished(),
            "new task starts must be rejected until restore ends"
        );

        active_gate.release();
        restore
            .await
            .expect("restore task join")
            .expect("restore backup");
        progress_write
            .await
            .expect("progress task join")
            .expect("progress write after restore");
        history_write
            .await
            .expect("history task join")
            .expect("history write after restore");
        assert!(
            blocked_start
                .await
                .expect("blocked task-start join")
                .is_err(),
            "a task targeting a removed book must fail after restore opens admission"
        );

        let restored_book_ref = service
            .store
            .book_ref(&snapshot_book)
            .expect("restored book ref");
        let restored_book = service
            .store
            .read_json_ref(&restored_book_ref)
            .await
            .expect("restored book");
        assert_eq!(restored_book["progress"]["offset"], 55);
        let removed_book_ref = service
            .store
            .book_ref(&active_book)
            .expect("removed book ref is syntactically valid");
        assert!(
            service
                .store
                .read_json_ref(&removed_book_ref)
                .await
                .is_err()
        );

        let history_ref = crate::reading_tools::reading_history_resource(&service.store)
            .await
            .expect("history ref");
        let history = service
            .store
            .read_json_ref(&history_ref)
            .await
            .expect("restored and updated history");
        let sessions = history["sessions"].as_array().unwrap();
        assert!(
            sessions
                .iter()
                .any(|session| session["sessionId"] == "before-restore")
        );
        assert!(
            sessions
                .iter()
                .any(|session| session["sessionId"] == "after-restore")
        );

        let tasks_ref = service.store.reading_ref("tasks").expect("tasks ref");
        let tasks = service
            .store
            .read_json_ref(&tasks_ref)
            .await
            .expect("restored task history");
        let archived = tasks["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == archived_task)
            .expect("completed task from snapshot");
        assert_eq!(archived["status"], "completed");
        assert!(
            !tasks["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|task| task["id"] == active_task)
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(archive);
    }

    #[tokio::test]
    async fn backup_waits_for_a_chapter_boundary_and_keeps_a_resumable_cursor() {
        let (service, executor, root, source_id) = controlled_service(3).await;
        let book_id = add_controlled_book(&service, &source_id, "backup-cursor").await;
        let gate = executor.gate_content("mock://book/backup-cursor");
        let task_id = task_id_from_start(
            &service
                .start_chapter_download(&book_id, 0, 3)
                .await
                .expect("start download"),
        );
        gate.wait_until_entered().await;

        let archive = root.with_extension("snapshot.zip");
        let backup_service = service.clone();
        let backup_archive = archive.clone();
        let backup =
            tokio::spawn(async move { backup_service.create_backup(&backup_archive).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !backup.is_finished(),
            "backup must wait while the current chapter is being materialized"
        );

        gate.release();
        backup
            .await
            .expect("backup task join")
            .expect("create consistent backup");
        let completed = wait_for_task_status(&service, &task_id, "completed").await;
        assert_eq!(completed.completed, 3);

        service
            .restore_backup(&archive)
            .await
            .expect("restore boundary snapshot");
        let interrupted = service
            .task_summary_record(&task_id)
            .await
            .expect("archived task cursor");
        assert_eq!(interrupted.status, "interrupted");
        assert_eq!(interrupted.completed, 1);

        let book_ref = service.store.book_ref(&book_id).expect("book resource");
        let book = service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("book from boundary snapshot");
        assert!(book["chapters"][0]["src"].is_string());
        assert!(book["chapters"][1]["src"].is_null());
        assert!(book["chapters"][2]["src"].is_null());

        service
            .resume_task(&task_id)
            .await
            .expect("resume from cursor");
        let resumed = wait_for_task_status(&service, &task_id, "completed").await;
        assert_eq!(resumed.completed, 3);
        let calls = executor.content_calls();
        for chapter_index in 0..3 {
            let chapter_url = format!("mock://book/backup-cursor/chapter/{chapter_index}");
            assert_eq!(
                calls.iter().filter(|(_, url)| url == &chapter_url).count(),
                if chapter_index == 0 { 1 } else { 2 },
                "snapshot-resume must not redownload chapter {chapter_index}"
            );
        }

        drop(service);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(archive);
    }

    #[tokio::test]
    async fn slow_failing_prefetch_keeps_first_chapter_and_progress_available_during_restore() {
        let (service, executor, root, source_id) = controlled_service(3).await;
        let book_id = add_controlled_book(&service, &source_id, "slow-second-chapter").await;
        let book_ref = service.store.book_ref(&book_id).expect("book resource");
        let initial_book = service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("initial book JSON");
        let first_chapter_id = initial_book["chapters"][0]["id"]
            .as_str()
            .expect("first chapter ID")
            .to_owned();
        let first_chapter_url = "mock://book/slow-second-chapter/chapter/0";
        let second_chapter_url = "mock://book/slow-second-chapter/chapter/1";

        service
            .prepare_chapters(&book_id, 0, 1)
            .await
            .expect("prepare the first chapter");
        service
            .save_progress(
                &book_id,
                json!({
                    "chapterId": first_chapter_id,
                    "chapterIndex": 0,
                    "offset": 41,
                    "updatedAtMs": 1000,
                }),
            )
            .await
            .expect("save baseline progress");

        let archive = root.with_extension("reader-snapshot.zip");
        service
            .create_backup(&archive)
            .await
            .expect("create restore snapshot");

        let gate = executor.gate_chapter_content(second_chapter_url);
        executor.fail_content_once(second_chapter_url);
        let prepare_service = service.clone();
        let preparing_book_id = book_id.clone();
        let prepare = tokio::spawn(async move {
            prepare_service
                .prepare_chapters(&preparing_book_id, 0, 2)
                .await
        });
        gate.wait_until_entered().await;

        let chapter_ref = service
            .store
            .chapter_ref(&book_id, &first_chapter_id)
            .expect("first chapter ref");
        let chapter_url = service.resource_server().url_for(&chapter_ref);
        let chapter_html = reqwest::Client::new()
            .get(chapter_url)
            .send()
            .await
            .expect("first chapter HTTP request")
            .error_for_status()
            .expect("first chapter remains readable while chapter two waits")
            .text()
            .await
            .expect("first chapter HTML");
        assert!(chapter_html.contains(first_chapter_url));

        tokio::time::timeout(
            Duration::from_secs(2),
            service.save_progress(
                &book_id,
                json!({
                    "chapterId": first_chapter_id,
                    "chapterIndex": 0,
                    "offset": 99,
                    "updatedAtMs": 2000,
                }),
            ),
        )
        .await
        .expect("progress persistence must not wait for another chapter's network call")
        .expect("save progress while next chapter is slow");

        let mut restore_barrier = service.restore_barrier.subscribe();
        let restore_service = service.clone();
        let restore_archive = archive.clone();
        let restore =
            tokio::spawn(async move { restore_service.restore_backup(&restore_archive).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !*restore_barrier.borrow() {
                restore_barrier
                    .changed()
                    .await
                    .expect("restore barrier channel remains open");
            }
        })
        .await
        .expect("restore should enter its admission barrier");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !restore.is_finished(),
            "restore must wait for the in-flight chapter operation"
        );

        gate.release();
        let prepare_result = tokio::time::timeout(Duration::from_secs(3), prepare)
            .await
            .expect("chapter preparation should finish after releasing its gate")
            .expect("prepare task join");
        assert!(
            prepare_result
                .expect_err("the controlled second chapter fails")
                .contains("controlled failure")
        );
        tokio::time::timeout(Duration::from_secs(5), restore)
            .await
            .expect("restore should proceed after chapter operation exits")
            .expect("restore task join")
            .expect("restore snapshot");

        let restored = service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("restored book JSON");
        assert!(restored["chapters"][0]["src"].is_string());
        assert!(restored["chapters"][1]["src"].is_null());
        assert_eq!(restored["progress"]["offset"], 41);
        let restored_chapter_url = service.resource_server().url_for(&chapter_ref);
        reqwest::Client::new()
            .get(restored_chapter_url)
            .send()
            .await
            .expect("restored chapter HTTP request")
            .error_for_status()
            .expect("snapshot keeps first chapter readable");

        drop(service);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(archive);
    }

    #[tokio::test]
    async fn concurrent_prepare_requests_fetch_the_same_chapter_once() {
        let (service, executor, root, source_id) = controlled_service(3).await;
        let book_id = add_controlled_book(&service, &source_id, "deduplicate-chapter").await;
        let chapter_url = "mock://book/deduplicate-chapter/chapter/1";
        let gate = executor.gate_chapter_content(chapter_url);

        let first_service = service.clone();
        let first_book_id = book_id.clone();
        let first =
            tokio::spawn(async move { first_service.prepare_chapters(&first_book_id, 1, 1).await });
        gate.wait_until_entered().await;

        let second_service = service.clone();
        let second_book_id = book_id.clone();
        let second =
            tokio::spawn(
                async move { second_service.prepare_chapters(&second_book_id, 1, 1).await },
            );
        tokio::task::yield_now().await;
        assert!(
            !second.is_finished(),
            "duplicate request should wait on the chapter-level in-flight lock"
        );

        gate.release();
        first
            .await
            .expect("first prepare task")
            .expect("first prepare");
        second
            .await
            .expect("second prepare task")
            .expect("second prepare should use the cache");
        assert_eq!(
            executor
                .content_calls()
                .iter()
                .filter(|(_, url)| url == chapter_url)
                .count(),
            1,
            "only one source-engine call should fetch this chapter"
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn deleting_a_book_during_chapter_fetch_does_not_resurrect_it() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let book_id = add_controlled_book(&service, &source_id, "delete-during-fetch").await;
        let chapter_url = "mock://book/delete-during-fetch/chapter/0";
        let gate = executor.gate_chapter_content(chapter_url);

        let prepare_service = service.clone();
        let preparing_book_id = book_id.clone();
        let prepare = tokio::spawn(async move {
            prepare_service
                .prepare_chapters(&preparing_book_id, 0, 1)
                .await
        });
        gate.wait_until_entered().await;

        tokio::time::timeout(Duration::from_secs(2), service.remove_book(&book_id))
            .await
            .expect("book deletion must not wait for chapter network work")
            .expect("delete book during fetch");
        gate.release();
        let prepare_error = tokio::time::timeout(Duration::from_secs(3), prepare)
            .await
            .expect("prepare should finish after releasing its gate")
            .expect("prepare task join")
            .expect_err("removed book must not receive a late chapter write");
        assert!(prepare_error.contains("removed while preparing"));

        let shelf = service
            .store
            .read_json_ref(&service.store.shelf_ref())
            .await
            .expect("shelf JSON");
        assert!(
            !shelf["books"]
                .as_array()
                .unwrap()
                .iter()
                .any(|book| book["id"] == book_id)
        );
        assert!(
            service
                .store
                .read_json_ref(&service.store.book_ref(&book_id).expect("book ref"))
                .await
                .is_err()
        );
        assert!(
            tokio::fs::metadata(root.join("books").join(&book_id))
                .await
                .is_err()
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn add_book_rejects_duplicate_chapter_urls_before_publishing_resources() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let search = service
            .search_books(&[source_id.clone()], "duplicate-catalog", 1, |_| {})
            .await
            .expect("controlled search");
        let search_ref = crate::resources::ResourceRef::new(
            search["resource"]["resourceId"]
                .as_str()
                .expect("search resource ID"),
        )
        .expect("valid search resource");
        let search_document = service
            .store
            .read_json_ref(&search_ref)
            .await
            .expect("search results");
        let result_id = search_document["results"][0]["resultId"]
            .as_str()
            .expect("result ID")
            .to_owned();
        let book_url = "mock://book/duplicate-catalog";
        executor.duplicate_catalog_for(book_url);

        let error = service
            .add_book(&result_id)
            .await
            .expect_err("ambiguous source catalog must be rejected");
        assert!(error.contains("duplicate"), "unexpected error: {error}");
        let shelf = service
            .store
            .read_json_ref(&service.store.shelf_ref())
            .await
            .expect("shelf JSON");
        assert!(shelf["books"].as_array().unwrap().is_empty());
        let book_id = format!(
            "book-{:016x}",
            super::stable_hash(&format!("{source_id}\0{book_url}"))
        );
        assert!(
            service
                .read_private_json(std::path::Path::new("books").join(format!("{book_id}.json")))
                .await
                .is_err()
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn catalog_refresh_preserves_cached_chapter_progress_and_does_not_lock_over_network() {
        let (service, executor, root, source_id) = controlled_service(3).await;
        let book_id = add_controlled_book(&service, &source_id, "catalog-reorder").await;
        let book_ref = service.store.book_ref(&book_id).expect("book resource");
        let initial = service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("initial book");
        let chapter_one_id = initial["chapters"][1]["id"]
            .as_str()
            .expect("chapter one ID")
            .to_owned();
        service
            .prepare_chapters(&book_id, 1, 1)
            .await
            .expect("cache chapter one");
        service
            .save_progress(
                &book_id,
                json!({
                    "chapterId": chapter_one_id,
                    "chapterIndex": 1,
                    "offset": 30,
                    "updatedAtMs": 1000,
                }),
            )
            .await
            .expect("save initial progress");

        let book_url = "mock://book/catalog-reorder";
        let refreshed = json!([
            { "title": "Chapter 1", "url": format!("{book_url}/chapter/1") },
            { "title": "Brand New", "url": format!("{book_url}/chapter/new") },
            { "title": "Chapter 0", "url": format!("{book_url}/chapter/0") }
        ]);
        executor.queue_catalog_responses(book_url, vec![refreshed.clone()]);
        let gate = executor.gate_next_catalog(book_url);
        let started = service
            .refresh_chapters(&book_id)
            .await
            .expect("start catalog refresh");
        let task_id = task_id_from_start(&started);
        gate.wait_until_entered().await;

        tokio::time::timeout(
            Duration::from_secs(2),
            service.save_progress(
                &book_id,
                json!({
                    "chapterId": chapter_one_id,
                    "chapterIndex": 1,
                    "offset": 88,
                    "updatedAtMs": 2000,
                }),
            ),
        )
        .await
        .expect("progress save must not wait for catalog network work")
        .expect("save progress while refresh is waiting in source engine");
        gate.release();
        let completed = wait_for_task_status(&service, &task_id, "completed").await;
        assert_eq!(completed.result.as_ref().unwrap()["addedCount"], 1);
        assert_eq!(completed.result.as_ref().unwrap()["matchedCount"], 2);
        assert_eq!(completed.result.as_ref().unwrap()["committed"], true);

        let book = service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("refreshed book");
        let chapter_one = &book["chapters"][0];
        assert_eq!(chapter_one["id"], chapter_one_id);
        assert_eq!(chapter_one["title"], "Chapter 1");
        assert!(chapter_one["src"].is_string(), "cached src is retained");
        assert_eq!(book["progress"]["chapterId"], chapter_one_id);
        assert_eq!(book["progress"]["chapterIndex"], 0);
        assert_eq!(book["progress"]["offset"], 88);
        assert_eq!(book["progress"]["updatedAtMs"], 2000);
        let progress_ref = service.store.progress_ref(&book_id).expect("progress ref");
        let progress = service
            .store
            .read_json_ref(&progress_ref)
            .await
            .expect("progress mirror");
        assert_eq!(progress["chapterId"], chapter_one_id);
        assert_eq!(progress["chapterIndex"], 0);
        assert_eq!(progress["offset"], 88);
        let private = service
            .read_private_json(std::path::Path::new("books").join(format!("{book_id}.json")))
            .await
            .expect("private engine catalog");
        assert_eq!(private["chapters"], refreshed);

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn empty_catalog_refresh_fails_without_changing_existing_resources() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let book_id = add_controlled_book(&service, &source_id, "empty-refresh").await;
        let book_ref = service.store.book_ref(&book_id).expect("book resource");
        let before_book = service
            .store
            .read_json_ref(&book_ref)
            .await
            .expect("book before refresh");
        let private_path = std::path::Path::new("books").join(format!("{book_id}.json"));
        let before_private = service
            .read_private_json(&private_path)
            .await
            .expect("private catalog before refresh");
        executor.queue_catalog_responses("mock://book/empty-refresh", vec![json!([])]);

        let started = service
            .refresh_chapters(&book_id)
            .await
            .expect("start empty refresh");
        let task_id = task_id_from_start(&started);
        let failed = wait_for_task_status(&service, &task_id, "failed").await;
        assert!(
            failed
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("empty")
        );
        assert_eq!(
            service.store.read_json_ref(&book_ref).await.unwrap(),
            before_book
        );
        assert_eq!(
            service.read_private_json(&private_path).await.unwrap(),
            before_private
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn deleting_book_during_catalog_request_does_not_resurrect_resources() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let book_id = add_controlled_book(&service, &source_id, "delete-catalog-refresh").await;
        let gate = executor.gate_next_catalog("mock://book/delete-catalog-refresh");
        let started = service
            .refresh_chapters(&book_id)
            .await
            .expect("start catalog refresh");
        let task_id = task_id_from_start(&started);
        gate.wait_until_entered().await;

        tokio::time::timeout(Duration::from_secs(2), service.remove_book(&book_id))
            .await
            .expect("delete must not wait for source engine network call")
            .expect("remove book");
        gate.release();
        let failed = wait_for_task_status(&service, &task_id, "failed").await;
        assert!(
            failed
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("removed")
        );
        assert!(
            service
                .store
                .read_json_ref(&service.store.book_ref(&book_id).unwrap())
                .await
                .is_err()
        );
        let shelf = service
            .store
            .read_json_ref(&service.store.shelf_ref())
            .await
            .expect("shelf JSON");
        assert!(
            !shelf["books"]
                .as_array()
                .unwrap()
                .iter()
                .any(|book| book["id"] == book_id)
        );

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn stale_concurrent_catalog_refresh_cannot_overwrite_newer_commit() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let book_id = add_controlled_book(&service, &source_id, "concurrent-catalog").await;
        let book_url = "mock://book/concurrent-catalog";
        let older = json!([
            { "title": "Older Catalog", "url": format!("{book_url}/chapter/older") }
        ]);
        let newer = json!([
            { "title": "Newer Catalog", "url": format!("{book_url}/chapter/newer") }
        ]);
        executor.queue_catalog_responses(book_url, vec![older, newer.clone()]);
        let gate = executor.gate_next_catalog(book_url);
        let first = service
            .refresh_chapters(&book_id)
            .await
            .expect("start first refresh");
        let first_id = task_id_from_start(&first);
        gate.wait_until_entered().await;

        let second = service
            .refresh_chapters(&book_id)
            .await
            .expect("start second refresh");
        let second_id = task_id_from_start(&second);
        let committed = wait_for_task_status(&service, &second_id, "completed").await;
        assert_eq!(committed.result.as_ref().unwrap()["committed"], true);
        gate.release();
        let stale = wait_for_task_status(&service, &first_id, "failed").await;
        assert!(
            stale
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("stale refresh")
        );

        let book = service
            .store
            .read_json_ref(&service.store.book_ref(&book_id).unwrap())
            .await
            .expect("book after concurrent refreshes");
        assert_eq!(book["chapters"][0]["title"], "Newer Catalog");
        let private = service
            .read_private_json(std::path::Path::new("books").join(format!("{book_id}.json")))
            .await
            .expect("private catalog after concurrent refreshes");
        assert_eq!(private["chapters"], newer);

        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn chapter_lock_registry_drops_idle_chapter_keys() {
        let (service, _executor, root, _source_id) = controlled_service(1).await;
        for index in 0..1000 {
            let guard = service
                .chapter_lock("book", &format!("chapter-{index}"))
                .await;
            drop(guard);
        }
        let registry_size = service.chapter_locks.lock().await.len();
        assert!(
            registry_size <= 1,
            "idle chapter lock registry should stay bounded, got {registry_size}"
        );
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn paused_download_does_not_hold_the_backup_operation_gate() {
        let (service, executor, root, source_id) = controlled_service(2).await;
        let book_id = add_controlled_book(&service, &source_id, "paused-backup").await;
        let gate = executor.gate_content("mock://book/paused-backup");
        let task_id = task_id_from_start(
            &service
                .start_chapter_download(&book_id, 0, 2)
                .await
                .expect("start download"),
        );
        gate.wait_until_entered().await;
        service.pause_task(&task_id).await.expect("pause download");
        gate.release();
        let paused = wait_for_task_status(&service, &task_id, "paused").await;
        assert_eq!(paused.completed, 1);

        let archive = root.with_extension("paused.zip");
        tokio::time::timeout(Duration::from_secs(2), service.create_backup(&archive))
            .await
            .expect("a paused task must not hold the operation gate")
            .expect("backup paused state");
        assert_eq!(
            service
                .task_summary_record(&task_id)
                .await
                .expect("paused task record")
                .status,
            "paused"
        );

        service
            .resume_task(&task_id)
            .await
            .expect("resume after backup");
        let completed = wait_for_task_status(&service, &task_id, "completed").await;
        assert_eq!(completed.completed, 2);

        drop(service);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(archive);
    }

    #[tokio::test]
    async fn source_to_cached_reader_flow_keeps_rules_private_and_survives_restart() {
        let fixture = SourceFixture::start();
        let root = temp_root();
        let service = ApplicationService::open(&root, None)
            .await
            .expect("application service");
        let base = fixture.base_url();
        let source_json = json!([{
            "bookSourceName": "Local fixture",
            "bookSourceUrl": base,
            "bookSourceType": 0,
            "searchUrl": format!("{base}/search,") + r#"{"method":"POST","body":"q={{key}}"}"#,
            "ruleSearch": {
                "bookList": "@css:.item", "name": "@css:h3 a@text",
                "author": "@css:.author@text", "bookUrl": "@css:h3 a@href"
            },
            "ruleBookInfo": { "name": "@css:h1@text", "tocUrl": "@css:a.toc@href" },
            "ruleToc": { "chapterList": "@css:#list li", "chapterName": "@css:a@text", "chapterUrl": "@css:a@href" },
            "ruleContent": { "content": "@css:.content@textNodes" }
        }]).to_string();

        let imported = service
            .import_sources(&source_json)
            .await
            .expect("import source");
        let source_id = imported["sources"][0]["id"]
            .as_str()
            .expect("source ID")
            .to_owned();
        let metadata_text = serde_json::to_string(&imported).expect("metadata JSON");
        assert!(!metadata_text.contains("ruleSearch"));
        assert!(!metadata_text.contains("searchUrl"));

        let search = service
            .search_books(&[source_id], "fixture", 1, |_| {})
            .await
            .expect("search");
        let client = reqwest::Client::new();
        let search_result = json_get(&client, search["resource"]["src"].as_str().unwrap()).await;
        assert_eq!(search_result["results"].as_array().map(Vec::len), Some(1));
        let public_search = serde_json::to_string(&search_result).expect("public search JSON");
        assert!(!public_search.contains("ruleSearch"));
        assert!(!public_search.contains("searchUrl"));

        let result_id = search_result["results"][0]["resultId"]
            .as_str()
            .expect("result ID");
        let added = service.add_book(result_id).await.expect("add book");
        let book_id = json_get(&client, added["book"]["src"].as_str().unwrap()).await["id"]
            .as_str()
            .expect("book ID")
            .to_owned();
        let initial_book = json_get(&client, added["book"]["src"].as_str().unwrap()).await;
        assert_eq!(initial_book["chapters"].as_array().map(Vec::len), Some(2));
        assert!(initial_book["chapters"][0]["src"].is_null());

        let prepared = service
            .prepare_chapters(&book_id, 0, 1)
            .await
            .expect("prepare first chapter");
        assert_eq!(prepared["prepared"], 1);
        let book_after_prepare = json_get(&client, prepared["book"]["src"].as_str().unwrap()).await;
        let first_src = book_after_prepare["chapters"][0]["src"]
            .as_str()
            .expect("prepared src");
        assert!(
            first_src.starts_with("http://127.0.0.1:")
                || first_src.starts_with("http://localhost:")
        );
        let first_html = client
            .get(first_src)
            .send()
            .await
            .expect("chapter resource")
            .error_for_status()
            .expect("chapter HTTP status")
            .text()
            .await
            .expect("chapter HTML");
        assert!(first_html.contains("First cached chapter"));

        let second_download = service
            .start_chapter_download(&book_id, 1, 1)
            .await
            .expect("start real KMP second chapter download");
        let second_task_id = task_id_from_start(&second_download);
        let second_task = wait_for_task_status(&service, &second_task_id, "completed").await;
        assert_eq!(second_task.completed, 1);
        let second_book = service
            .store
            .read_json_ref(
                &service
                    .store
                    .book_ref(&book_id)
                    .expect("book ref after second chapter"),
            )
            .await
            .expect("book after second chapter download");
        let second_chapter_id = second_book["chapters"][1]["id"]
            .as_str()
            .expect("second chapter ID");
        let second_chapter_ref = service
            .store
            .chapter_ref(&book_id, second_chapter_id)
            .expect("second chapter resource ref");
        let second_src = service.resource_server().url_for(&second_chapter_ref);
        let second_html = client
            .get(second_src)
            .send()
            .await
            .expect("second chapter resource")
            .error_for_status()
            .expect("second chapter HTTP status")
            .text()
            .await
            .expect("second chapter HTML");
        assert!(second_html.contains("Second cached chapter"));

        assert!(
            service
                .save_progress(
                    &book_id,
                    json!({
                        "chapterId": book_after_prepare["chapters"][0]["id"],
                        "chapterIndex": 0, "offset": -1, "updatedAtMs": 1_800_000_000_000u64,
                    })
                )
                .await
                .is_err()
        );
        let unchanged = json_get(&client, prepared["book"]["src"].as_str().unwrap()).await;
        assert_eq!(unchanged["progress"]["offset"], 0);

        service
            .save_progress(
                &book_id,
                json!({
                    "chapterId": book_after_prepare["chapters"][0]["id"],
                    "chapterIndex": 0, "offset": 129, "updatedAtMs": 1_800_000_000_000u64,
                }),
            )
            .await
            .expect("save progress");
        let duplicated_add = service.add_book(result_id).await.expect("idempotent add");
        let duplicated_book =
            json_get(&client, duplicated_add["book"]["src"].as_str().unwrap()).await;
        assert_eq!(duplicated_book["progress"]["offset"], 129);
        assert!(duplicated_book["chapters"][0].get("src").is_some());

        let persisted =
            std::fs::read_to_string(root.join("books").join(&book_id).join("book.json"))
                .expect("stable persisted JSON");
        assert!(persisted.contains("resource://books/"));
        assert!(!persisted.contains(&service.resource_server().local_addr().port().to_string()));

        drop(service);
        let restarted = ApplicationService::open(&root, None)
            .await
            .expect("restart application");
        let bootstrap = restarted
            .bootstrap()
            .await
            .expect("bootstrap after restart");
        let shelf = json_get(&client, bootstrap["shelf"]["src"].as_str().unwrap()).await;
        assert_eq!(shelf["books"].as_array().map(Vec::len), Some(1));
        assert_eq!(shelf["books"][0]["progress"]["offset"], 129);
        let removed = restarted.remove_book(&book_id).await.expect("remove book");
        let shelf = json_get(&client, removed["shelf"]["src"].as_str().unwrap()).await;
        assert_eq!(shelf["books"].as_array().map(Vec::len), Some(0));

        drop(restarted);
        drop(fixture);
        let _ = std::fs::remove_dir_all(root);
    }
}
