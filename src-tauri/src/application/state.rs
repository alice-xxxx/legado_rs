//! 应用服务共享状态、操作门闩、资源入口与书架锁。

use super::*;

impl ApplicationService {
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
        // Debug previews have no shelf identity and must not survive an app restart.
        store
            .remove_book_resources("debug-source")
            .await
            .map_err(|error| error.to_string())?;
        // HTTP TTS audio files are request-scoped cache entries, never backups.
        store
            .remove_book_resources("tts-audio")
            .await
            .map_err(|error| error.to_string())?;
        // Search snapshots are immutable browser-session publications. No
        // descriptor from a previous WebView process remains consumable.
        store
            .remove_search_snapshots()
            .await
            .map_err(|error| error.to_string())?;
        store
            .remove_txt_toc_rule_snapshots()
            .await
            .map_err(|error| error.to_string())?;
        store
            .remove_source_definition_snapshots()
            .await
            .map_err(|error| error.to_string())?;
        let private_root = root.join("private-data");
        tokio::fs::create_dir_all(private_root.join("books"))
            .await
            .map_err(|error| {
                format!("Cannot create private application data directory: {error}")
            })?;
        let http_tts_ref = store.http_tts_configs_ref();
        let empty_http_tts = crate::http_tts_config::empty_document();
        let http_tts_configs = store
            .read_json_ref_or_default(&http_tts_ref, empty_http_tts.clone())
            .await
            .map_err(|error| error.to_string())?;
        if crate::http_tts_config::validate_document(&http_tts_configs).is_err() {
            store
                .write_json_ref(&http_tts_ref, &empty_http_tts)
                .await
                .map_err(|error| error.to_string())?;
        }
        let shelf_ref = store.shelf_ref();
        let empty_shelf = serde_json::to_value(crate::models::ShelfDocument::default())
            .map_err(|error| error.to_string())?;
        let shelf = store
            .read_json_ref_or_default(&shelf_ref, empty_shelf.clone())
            .await
            .map_err(|error| error.to_string())?;
        if !valid_startup_shelf(&shelf) {
            store
                .write_json_ref(&shelf_ref, &empty_shelf)
                .await
                .map_err(|error| error.to_string())?;
        }
        cleanup_non_shelf_book_resources(&store, &private_root).await?;
        let settings_ref = store.settings_ref();
        let mut empty_settings = serde_json::to_value(crate::models::SettingsDocument::default())
            .map_err(|error| error.to_string())?;
        empty_settings["lastBackupAtMs"] = Value::Null;
        let mut startup_settings = store
            .read_json_ref_or_default(&settings_ref, empty_settings.clone())
            .await
            .map_err(|error| error.to_string())?;
        if !valid_startup_settings(&startup_settings) {
            store
                .write_json_ref(&settings_ref, &empty_settings)
                .await
                .map_err(|error| error.to_string())?;
            startup_settings = empty_settings;
        }
        crate::source_http::set_network_timeout_seconds(configured_source_http_timeout_seconds(
            Some(&startup_settings),
        ));
        let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let server = Arc::new(
            store
                .start_http(bind)
                .await
                .map_err(|error| error.to_string())?,
        );
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
        let empty_tasks = json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "tasks": [] });
        let task_document = store
            .read_json_ref_or_default(&task_ref, empty_tasks)
            .await
            .map_err(|error| error.to_string())?;
        let mut task_records = if task_document.get("schemaVersion").and_then(Value::as_u64)
            == Some(u64::from(CURRENT_SCHEMA_VERSION))
        {
            serde_json::from_value::<Vec<AppTask>>(
                task_document
                    .get("tasks")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut task_registry = TaskRegistry {
            records: task_records,
            signals: HashMap::new(),
            active_search_task_id: None,
            search_documents: HashMap::new(),
            search_result_documents: HashMap::new(),
            search_replacement_contexts: HashMap::new(),
            search_book_group_roots: HashMap::new(),
        };
        for task in &mut task_registry.records {
            if matches!(task.kind.as_str(), "search" | "bookSourceCandidates") {
                // Published search files are session resources and were reclaimed at startup.
                task.result = None;
            }
            if task.kind == "search" {
                // Search results are session memory. Preserve explicit stop
                // intent, but restart resumable searches with an empty result
                // set and a source-zero checkpoint.
                let recovered_status = match task.status.as_str() {
                    "cancelling" => Some("cancelled"),
                    "pausing" => Some("paused"),
                    "queued" | "running" => Some("interrupted"),
                    "paused" => Some("paused"),
                    "interrupted" => Some("interrupted"),
                    _ => None,
                };
                if let Some(status) = recovered_status {
                    task.status = status.to_owned();
                    task.updated_at_ms = now_ms();
                    if !matches!(status, "cancelled") {
                        task.completed = 0;
                        task.error = None;
                        if let Some(search_id) = task.search_id.as_ref() {
                            task_registry.search_documents.insert(
                                search_id.clone(),
                                empty_search_task_document(task),
                            );
                        }
                    }
                }
                continue;
            }
            // A persisted user stop request remains effective after process loss.
            let recovered_status = match task.status.as_str() {
                "cancelling" => Some("cancelled"),
                "pausing" => Some("paused"),
                "queued" | "running" => Some("interrupted"),
                _ => None,
            };
            if let Some(status) = recovered_status {
                task.status = status.to_owned();
                task.updated_at_ms = now_ms();
            }
            if task.kind == "bookSourceCandidates"
                && !matches!(
                    task.status.as_str(),
                    "completed" | "failed" | "cancelled" | "recoveryRequired"
                )
            {
                task.status = "failed".into();
                task.error = Some(
                    "Candidate search results are transient and cannot resume after restart; retry the search.".into(),
                );
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
            task_recovery_notifier: Arc::new(std::sync::RwLock::new(None)),
            search_snapshot_notifier: Arc::new(std::sync::RwLock::new(None)),
            admission_gate: Arc::new(tokio::sync::RwLock::new(())),
            operation_gate: Arc::new(tokio::sync::RwLock::new(())),
            webdav_backup_upload_gate: Arc::new(tokio::sync::Mutex::new(())),
            webdav_backup_config_gate: Arc::new(tokio::sync::Mutex::new(())),
            restore_barrier: Arc::new(restore_barrier),
            restore_serial: Arc::new(tokio::sync::Mutex::new(())),
            restore_epoch: Arc::new(AtomicU64::new(0)),
            pending_pdf_imports: PendingPdfImportRegistry::new(),
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

    pub fn set_task_recovery_notifier(&self, notifier: Arc<dyn Fn(Value) + Send + Sync>) {
        if let Ok(mut target) = self.task_recovery_notifier.write() {
            *target = Some(notifier);
        }
    }

    pub fn set_search_snapshot_notifier(&self, notifier: Arc<dyn Fn(Value) + Send + Sync>) {
        if let Ok(mut target) = self.search_snapshot_notifier.write() {
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
            "format": resource_format(resource.as_str()),
        })
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

    pub(super) async fn upsert_shelf(&self, book_id: &str) -> Result<(), String> {
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

    pub(super) async fn book_lock(&self, book_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.book_locks.lock().await;
            locks
                .entry(book_id.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    pub(super) async fn chapter_lock(
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

fn valid_startup_settings(value: &Value) -> bool {
    serde_json::from_value::<crate::models::SettingsDocument>(value.clone())
        .is_ok_and(|settings| settings.schema_version == CURRENT_SCHEMA_VERSION)
}

fn valid_startup_shelf(value: &Value) -> bool {
    let Ok(shelf) = serde_json::from_value::<crate::models::ShelfDocument>(value.clone()) else {
        return false;
    };
    if shelf.schema_version != CURRENT_SCHEMA_VERSION {
        return false;
    }
    let mut ids = HashSet::with_capacity(shelf.books.len());
    shelf
        .books
        .iter()
        .all(|book| validate_id(&book.id, "bookId").is_ok() && ids.insert(book.id.as_str()))
}

fn is_persisted_book_id(book_id: &str) -> bool {
    (book_id.starts_with("book-") || book_id.starts_with("local-"))
        && validate_id(book_id, "bookId").is_ok()
}

async fn cleanup_non_shelf_book_resources(
    store: &ResourceStore,
    private_root: &Path,
) -> Result<(), String> {
    let shelf = store
        .read_json_ref(&store.shelf_ref())
        .await
        .map_err(|error| error.to_string())?;
    let shelf_books = shelf
        .get("books")
        .and_then(Value::as_array)
        .ok_or_else(|| "Cannot clean book resources: shelf books must be an array".to_owned())?;
    let mut shelf_ids = HashSet::with_capacity(shelf_books.len());
    for entry in shelf_books {
        let book_id = entry
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Cannot clean book resources: shelf entry has no ID".to_owned())?;
        validate_id(book_id, "bookId")?;
        shelf_ids.insert(book_id.to_owned());
    }

    let root = store.root();
    let public_books_dir = root.join("books");
    let progress_dir = root.join("progress");
    let private_books_dir = private_root.join("books");
    for directory in [&public_books_dir, &progress_dir] {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|error| format!("Cannot prepare book resource cleanup: {error}"))?;
    }

    let mut candidates = HashSet::new();
    let mut public_entries = tokio::fs::read_dir(&public_books_dir)
        .await
        .map_err(|error| format!("Cannot scan public book resources: {error}"))?;
    while let Some(entry) = public_entries
        .next_entry()
        .await
        .map_err(|error| format!("Cannot scan public book resources: {error}"))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| format!("Cannot inspect public book resource: {error}"))?;
        if file_type.is_dir() {
            if let Some(book_id) = entry.file_name().to_str() {
                if is_persisted_book_id(book_id) {
                    candidates.insert(book_id.to_owned());
                }
            }
        }
    }

    let mut progress_entries = tokio::fs::read_dir(&progress_dir)
        .await
        .map_err(|error| format!("Cannot scan book progress: {error}"))?;
    while let Some(entry) = progress_entries
        .next_entry()
        .await
        .map_err(|error| format!("Cannot scan book progress: {error}"))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| format!("Cannot inspect book progress: {error}"))?;
        if file_type.is_file() {
            if let Some(book_id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
            {
                if is_persisted_book_id(book_id) {
                    candidates.insert(book_id.to_owned());
                }
            }
        }
    }

    let mut private_entries = tokio::fs::read_dir(&private_books_dir)
        .await
        .map_err(|error| format!("Cannot scan private book resources: {error}"))?;
    while let Some(entry) = private_entries
        .next_entry()
        .await
        .map_err(|error| format!("Cannot scan private book resources: {error}"))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| format!("Cannot inspect private book resource: {error}"))?;
        if file_type.is_file() {
            if let Some(book_id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
            {
                if is_persisted_book_id(book_id) {
                    candidates.insert(book_id.to_owned());
                }
            }
        }
    }

    for book_id in candidates.difference(&shelf_ids) {
        let writer_guard = store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let book_ref = store.book_ref(book_id).map_err(|error| error.to_string())?;
        let progress_ref = store
            .progress_ref(book_id)
            .map_err(|error| error.to_string())?;
        let replacements = vec![
            crate::resource_transactions::Replacement::delete_book_json(book_ref)
                .map_err(|error| error.to_string())?,
            crate::resource_transactions::Replacement::delete_progress_json(progress_ref)
                .map_err(|error| error.to_string())?,
            crate::resource_transactions::Replacement::delete_private_book_json(book_id)
                .map_err(|error| error.to_string())?,
        ];
        let post_commit_deletes = vec![
            crate::resource_transactions::PostCommitDelete::book_directory(book_id)
                .map_err(|error| error.to_string())?,
        ];
        let transaction_root = writer_guard.data_root().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "startup-prune-book-cache",
                replacements,
                post_commit_deletes,
                &writer_guard,
            )?;
            transaction.commit()
        })
        .await
        .map_err(|error| format!("Book resource cleanup worker failed: {error}"))?
        .map_err(|error| format!("Cannot clean resources for book '{book_id}': {error}"))?;
    }
    Ok(())
}
