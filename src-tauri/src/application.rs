//! Rust application services and Tauri commands.
//!
//! Source definitions and parser output remain inside this module's private
//! data directory. The WebView receives processed resource JSON/HTML only.

use std::io::Write;
use std::{
    collections::HashMap,
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{
    models::{
        BookDocument, ChapterDescriptor, ProgressDocument, ReaderDefaults, ReaderTheme,
        CURRENT_SCHEMA_VERSION,
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
    store: Arc<ResourceStore>,
    server: Arc<ResourceServer>,
    executor: Arc<dyn SourceExecutor>,
    book_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    sources_lock: Arc<tokio::sync::Mutex<()>>,
    tasks: Arc<tokio::sync::Mutex<TaskRegistry>>,
    task_slots: Arc<tokio::sync::Semaphore>,
    task_notifier: Arc<std::sync::RwLock<Option<Arc<dyn Fn(Value) + Send + Sync>>>>,
    admission_gate: Arc<tokio::sync::RwLock<()>>,
    operation_gate: Arc<tokio::sync::RwLock<()>>,
    restore_barrier: Arc<tokio::sync::watch::Sender<bool>>,
    restore_serial: Arc<tokio::sync::Mutex<()>>,
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
        let root = root.into();
        crate::backup::recover_interrupted_restore(&root)?;
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|error| format!("Cannot create application data directory: {error}"))?;
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
            store,
            server,
            executor,
            book_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            sources_lock: Arc::new(tokio::sync::Mutex::new(())),
            tasks: Arc::new(tokio::sync::Mutex::new(task_registry)),
            task_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            task_notifier: Arc::new(std::sync::RwLock::new(None)),
            admission_gate: Arc::new(tokio::sync::RwLock::new(())),
            operation_gate: Arc::new(tokio::sync::RwLock::new(())),
            restore_barrier: Arc::new(restore_barrier),
            restore_serial: Arc::new(tokio::sync::Mutex::new(())),
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

    pub async fn bootstrap(&self) -> Result<Value, String> {
        Ok(json!({
            "shelf": self.resource_descriptor(&self.store.shelf_ref()),
            "settings": self.resource_descriptor(&self.store.settings_ref()),
            "sources": self.source_metadata().await?,
        }))
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
                source.enabled && (source_ids.is_empty() || source_ids.contains(&source.id))
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("No enabled book sources are selected".into());
        }

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
        self.store_processed_results(keyword, page, source_results, errors)
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
                source.enabled && (source_ids.is_empty() || source_ids.contains(&source.id))
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
                "search" => self.run_search_task(&task_id, &mut signal).await,
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
                    if task.kind == "search" {
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
        match response {
            Ok(value) => {
                let books = value
                    .get("books")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for book in books {
                    let result_id = format!("result-{}", uuid::Uuid::new_v4().simple());
                    self.write_private_json(
                        Path::new("search-results").join(format!("{result_id}.json")),
                        &json!({ "sourceId": source.id, "book": book }),
                    )
                    .await?;
                    results.push(project_search_result(&result_id, source, &book));
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
        let _book_lock = self.book_lock(book_id).await;
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let mut private = self.read_private_json(&private_path).await?;
        let source_id = private["sourceId"]
            .as_str()
            .ok_or_else(|| "Private book is missing sourceId".to_owned())?
            .to_owned();
        let engine_book = private
            .get("book")
            .cloned()
            .ok_or_else(|| "Private book is missing engine metadata".to_owned())?;
        let legacy_rss =
            crate::rss::is_legacy_rss_source(&self.find_source(&source_id).await?.source);
        let response = self
            .execute_source_operation(
                &source_id,
                if legacy_rss {
                    "rssChapters"
                } else {
                    "chapters"
                },
                None,
                None,
                Some(engine_book),
                None,
                None,
            )
            .await?;
        let new_raw = response
            .as_array()
            .cloned()
            .ok_or_else(|| "Source engine returned an invalid chapter list".to_owned())?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let mut book_json = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|error| error.to_string())?;
        let old_raw = private
            .get("chapters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let old_descriptors = book_json
            .get("chapters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let old_by_url: HashMap<String, Value> = old_raw
            .iter()
            .enumerate()
            .filter_map(|(index, raw)| {
                let url = text_at(raw, &["url", "chapterUrl"])?;
                Some((url, old_descriptors.get(index)?.clone()))
            })
            .collect();
        let mut added = 0usize;
        let descriptors = new_raw
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                let url =
                    text_at(raw, &["url", "chapterUrl"]).unwrap_or_else(|| format!("@{index}"));
                let previous = old_by_url.get(&url);
                let id = previous
                    .and_then(|entry| entry.get("id").and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_else(|| chapter_id(book_id, &url));
                if previous.is_none() {
                    added += 1;
                }
                let cached = previous
                    .and_then(|entry| entry.get("src"))
                    .filter(|src| !src.is_null())
                    .and_then(|_| self.store.chapter_ref(book_id, &id).ok())
                    .filter(|_| {
                        self.root
                            .join("books")
                            .join(book_id)
                            .join("chapters")
                            .join(format!("{id}.html"))
                            .is_file()
                    });
                ChapterDescriptor {
                    id,
                    title: text_at(raw, &["title", "chapterName", "name"])
                        .unwrap_or_else(|| format!("Chapter {}", index + 1)),
                    index,
                    src: cached,
                }
            })
            .collect::<Vec<_>>();
        let old_progress = book_json
            .get("progress")
            .cloned()
            .unwrap_or_else(|| json!({"chapterIndex":0,"offset":0,"updatedAtMs":0}));
        let old_progress_index = old_progress
            .get("chapterIndex")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        let old_progress_id = old_progress
            .get("chapterId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let existing_progress_chapter = old_progress_id
            .as_ref()
            .and_then(|id| descriptors.iter().find(|item| &item.id == id));
        let moved_progress = old_progress_id.is_some() && existing_progress_chapter.is_none();
        if let Some(existing) = existing_progress_chapter {
            book_json["progress"]["chapterIndex"] = json!(existing.index);
        } else if moved_progress && commit {
            let nearest = if descriptors.is_empty() {
                None
            } else {
                Some(old_progress_index.min(descriptors.len() - 1))
            };
            book_json["progress"]["chapterIndex"] = json!(nearest.unwrap_or(0));
            book_json["progress"]["chapterId"] = nearest
                .map(|index| json!(descriptors[index].id))
                .unwrap_or(Value::Null);
            book_json["progress"]["updatedAtMs"] = json!(now_ms());
            let progress_ref = self
                .store
                .progress_ref(book_id)
                .map_err(|error| error.to_string())?;
            self.store
                .update_json_ref(&progress_ref, |mut progress| {
                    progress["chapterIndex"] = json!(nearest.unwrap_or(0));
                    progress["chapterId"] = nearest
                        .map(|index| json!(descriptors[index].id))
                        .unwrap_or(Value::Null);
                    progress["updatedAtMs"] = json!(now_ms());
                    Ok(progress)
                })
                .await
                .map_err(|error| error.to_string())?;
        }
        let title_latest = new_raw
            .last()
            .and_then(|raw| text_at(raw, &["title", "chapterName", "name"]));
        if commit {
            book_json["chapters"] =
                serde_json::to_value(&descriptors).map_err(|error| error.to_string())?;
            book_json["chapterCount"] = json!(descriptors.len());
            book_json["latestChapter"] = json!(title_latest);
            self.store
                .write_json_ref(&book_ref, &book_json)
                .await
                .map_err(|error| error.to_string())?;
            private["chapters"] = json!(new_raw);
            self.write_private_json(private_path, &private).await?;
            self.upsert_shelf(book_id).await?;
        }
        Ok(json!({
            "bookResourceId": book_ref.as_str(),
            "addedCount": added,
            "movedProgress": moved_progress,
            "committed": commit,
        }))
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
            if task.kind == "search"
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
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let resource = self
            .store
            .search_ref(&search_id)
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
        let title = text_at(&engine_book, &["name", "title"])
            .or_else(|| text_at(&raw_book, &["name", "title"]))
            .unwrap_or_else(|| "Untitled".to_owned());
        let author = text_at(&engine_book, &["author"])
            .or_else(|| text_at(&raw_book, &["author"]))
            .unwrap_or_default();
        let cover_url = text_at(&engine_book, &["coverUrl", "cover", "coverSrc"])
            .or_else(|| text_at(&raw_book, &["coverUrl", "cover", "coverSrc"]));
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
        let book = BookDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: book_id.clone(),
            title,
            author,
            cover_src: cover_url.and_then(|cover| ResourceRef::new(cover).ok()),
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
            &json!({ "sourceId": source_id, "book": engine_book, "chapters": raw_chapters }),
        )
        .await?;
        self.upsert_shelf(&book_id).await?;
        Ok(json!({
            "book": self.resource_descriptor(&book_ref),
            "shelf": self.resource_descriptor(&self.store.shelf_ref()),
        }))
    }

    pub async fn get_book(&self, book_id: &str) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let reference = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        self.store
            .read_json_ref(&reference)
            .await
            .map_err(|error| error.to_string())?;
        Ok(self.resource_descriptor(&reference))
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
        let _book_lock = self.book_lock(book_id).await;
        let count = count.clamp(1, 50);
        let private = self
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await?;
        let source_id = private["sourceId"]
            .as_str()
            .ok_or_else(|| "Book source ID is missing".to_owned())?;
        let engine_book = private
            .get("book")
            .cloned()
            .ok_or_else(|| "Book engine data is missing".to_owned())?;
        let raw_chapters = private
            .get("chapters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let source = self.find_source(source_id).await?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let mut book_json = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|error| error.to_string())?;
        let mut prepared = 0usize;
        let end = from_index.saturating_add(count).min(raw_chapters.len());
        let defaults = reader_defaults(
            self.store
                .read_json_ref(&self.store.settings_ref())
                .await
                .ok(),
        );
        for index in from_index.min(end)..end {
            let raw_chapter = &raw_chapters[index];
            let descriptor = book_json
                .get("chapters")
                .and_then(Value::as_array)
                .and_then(|chapters| chapters.get(index))
                .cloned()
                .ok_or_else(|| format!("Book chapter metadata missing at index {index}"))?;
            let chapter_id = descriptor["id"]
                .as_str()
                .ok_or_else(|| "Chapter ID missing".to_owned())?;
            let already_ready = tokio::fs::metadata(
                self.root
                    .join("books")
                    .join(book_id)
                    .join("chapters")
                    .join(format!("{chapter_id}.html")),
            )
            .await
            .is_ok();
            if already_ready {
                if let Some(chapter) = book_json
                    .get_mut("chapters")
                    .and_then(Value::as_array_mut)
                    .and_then(|chapters| chapters.get_mut(index))
                {
                    chapter["src"] = Value::String(
                        self.store
                            .chapter_ref(book_id, chapter_id)
                            .map_err(|error| error.to_string())?
                            .to_string(),
                    );
                }
                self.store
                    .write_json_ref(&book_ref, &book_json)
                    .await
                    .map_err(|error| error.to_string())?;
                continue;
            }
            let next_chapter_url = raw_chapters
                .get(index + 1)
                .and_then(|chapter| text_at(chapter, &["url", "chapterUrl"]));
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
            let reference = if looks_like_html(content) {
                self.store
                    .write_chapter_html(book_id, chapter_id, content, &defaults)
                    .await
            } else {
                self.store
                    .write_chapter_text(book_id, chapter_id, content, &defaults)
                    .await
            }
            .map_err(|error| error.to_string())?;
            if let Some(chapter) = book_json
                .get_mut("chapters")
                .and_then(Value::as_array_mut)
                .and_then(|chapters| chapters.get_mut(index))
            {
                chapter["src"] = Value::String(reference.to_string());
            }
            prepared += 1;
            // Publish each completed chapter immediately. A later network failure
            // must not leave earlier HTML files disconnected from the catalog.
            self.store
                .write_json_ref(&book_ref, &book_json)
                .await
                .map_err(|error| error.to_string())?;
        }
        self.store
            .write_json_ref(&book_ref, &book_json)
            .await
            .map_err(|error| error.to_string())?;
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

    async fn source_metadata(&self) -> Result<Vec<Value>, String> {
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

    async fn write_sources(&self, sources: &[SourceRecord]) -> Result<(), String> {
        self.write_private_json(Path::new("sources.json"), &json!(sources))
            .await
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

fn metadata(records: &[SourceRecord]) -> Vec<Value> {
    records.iter().map(|record| json!({
        "id": record.id, "name": record.name, "group": record.group, "enabled": record.enabled,
    })).collect()
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

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod tauri_commands {
    use super::*;
    use tauri::{Emitter, Manager, State};
    use tauri_plugin_dialog::{DialogExt, FilePath};

    struct PickerFile {
        path: PathBuf,
        temporary: bool,
    }

    impl Drop for PickerFile {
        fn drop(&mut self) {
            if self.temporary {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }

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
        if let Ok(path) = selected.clone().into_path() {
            validate_picker_extension(&path, allowed_extensions)?;
            return Ok(PickerFile {
                path,
                temporary: false,
            });
        }
        use tauri_plugin_fs::FsExt;
        let bytes = app
            .fs()
            .read(selected.clone())
            .map_err(|error| format!("Cannot read selected file: {error}"))?;
        let preferred = selected_extension(&selected);
        let extension = preferred
            .filter(|extension| allowed_extensions.contains(&extension.as_str()))
            .or_else(|| {
                if allowed_extensions.contains(&"epub") && bytes.starts_with(b"PK\x03\x04") {
                    Some("epub".to_owned())
                } else {
                    allowed_extensions
                        .first()
                        .map(|extension| (*extension).to_owned())
                }
            })
            .ok_or_else(|| "Selected file type is not supported".to_owned())?;
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("private-data")
            .join("picker-imports");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| error.to_string())?;
        let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4().simple(), extension));
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|error| format!("Cannot stage selected file: {error}"))?;
        Ok(PickerFile {
            path,
            temporary: true,
        })
    }

    fn selected_extension(selected: &FilePath) -> Option<String> {
        let name = match selected {
            FilePath::Path(path) => path.file_name()?.to_str()?.to_owned(),
            FilePath::Url(url) => url.path_segments()?.next_back()?.to_owned(),
        };
        Path::new(&name)
            .extension()?
            .to_str()
            .map(|extension| extension.to_ascii_lowercase())
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
        service.bootstrap().await
    }

    #[tauri::command]
    pub async fn list_sources(service: State<'_, ApplicationService>) -> Result<Value, String> {
        service.list_sources().await
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
    pub async fn import_book_from_picker(
        app: tauri::AppHandle,
        options: crate::local_books::LocalImportOptions,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        let Some(path) = pick_file(&app, "Books", &["txt", "epub", "cbz", "pdf"]).await? else {
            return Ok(json!({ "cancelled": true }));
        };
        let result = service.import_local_book(&path.path, options).await?;
        let _ = app.emit("book-added", result["book"].clone());
        let _ = app.emit("shelf-updated", result["shelf"].clone());
        Ok(result)
    }

    #[tauri::command]
    pub async fn get_book(
        book_id: String,
        service: State<'_, ApplicationService>,
    ) -> Result<Value, String> {
        service.get_book(&book_id).await
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
        collections::HashMap,
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    use serde_json::{json, Value};

    use super::{AppTask, ApplicationService, EngineFuture, SourceExecutor};
    use crate::source_engine::SourceEngineRequest;

    #[derive(Clone)]
    struct ControlledExecutor {
        chapter_count: usize,
        content_gates: Arc<std::sync::Mutex<HashMap<String, Arc<ExecutionGate>>>>,
        search_gates: Arc<std::sync::Mutex<HashMap<String, Arc<ExecutionGate>>>>,
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
                content_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
                search_gates: Arc::new(std::sync::Mutex::new(HashMap::new())),
                content_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        fn gate_content(&self, book_url: &str) -> Arc<ExecutionGate> {
            let gate = Arc::new(ExecutionGate::new());
            self.content_gates
                .lock()
                .expect("content gate mutex")
                .insert(book_url.to_owned(), gate.clone());
            gate
        }

        fn gate_search(&self, keyword: &str) -> Arc<ExecutionGate> {
            let gate = Arc::new(ExecutionGate::new());
            self.search_gates
                .lock()
                .expect("search gate mutex")
                .insert(keyword.to_owned(), gate.clone());
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
                match request.operation.as_str() {
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
                            .ok_or_else(|| "mock bookUrl missing".to_owned())?;
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
                        let gate = self
                            .content_gates
                            .lock()
                            .expect("content gate mutex")
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
                        self.content_calls
                            .lock()
                            .expect("content call mutex")
                            .push((book_url, chapter_url.clone()));
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
            "/search" => "<div class='item'><h3><a href='/book'>Fixture Novel</a></h3><span class='author'>A. Writer</span></div>",
            "/book" => "<h1>Fixture Novel</h1><a class='toc' href='/toc'>目录</a>",
            "/toc" => "<ul id='list'><li><a href='/chapter/one'>Chapter One</a></li><li><a href='/chapter/two'>Chapter Two</a></li></ul>",
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

        assert!(service
            .save_settings(json!({ "reader": { "fontSizePx": 37 } }))
            .await
            .is_err());
        assert!(service
            .save_settings(json!({ "reader": { "fontSizePx": "20" } }))
            .await
            .is_err());
        assert!(service
            .save_settings(json!({ "reader": { "lineHeight": 2.9 } }))
            .await
            .is_err());
        assert!(service
            .save_settings(json!({ "reader": { "preloadCount": 5.5 } }))
            .await
            .is_err());
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
        assert!(book["chapters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|chapter| chapter["src"].is_string()));

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
        assert!(service
            .store
            .read_json_ref(&removed_book_ref)
            .await
            .is_err());

        let history_ref = crate::reading_tools::reading_history_resource(&service.store)
            .await
            .expect("history ref");
        let history = service
            .store
            .read_json_ref(&history_ref)
            .await
            .expect("restored and updated history");
        let sessions = history["sessions"].as_array().unwrap();
        assert!(sessions
            .iter()
            .any(|session| session["sessionId"] == "before-restore"));
        assert!(sessions
            .iter()
            .any(|session| session["sessionId"] == "after-restore"));

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
        assert!(!tasks["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == active_task));

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

        assert!(service
            .save_progress(
                &book_id,
                json!({
                    "chapterId": book_after_prepare["chapters"][0]["id"],
                    "chapterIndex": 0, "offset": -1, "updatedAtMs": 1_800_000_000_000u64,
                })
            )
            .await
            .is_err());
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
