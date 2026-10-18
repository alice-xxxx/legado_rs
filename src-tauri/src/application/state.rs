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
        let settings_ref = store.settings_ref();
        let mut empty_settings = serde_json::to_value(crate::models::SettingsDocument::default())
            .map_err(|error| error.to_string())?;
        empty_settings["lastBackupAtMs"] = Value::Null;
        let mut startup_settings = store
            .read_json_ref_or_default(&settings_ref, empty_settings.clone())
            .await
            .map_err(|error| error.to_string())?;
        let previous_settings = startup_settings.clone();
        crate::models::upgrade_settings_document(&mut startup_settings);
        if !valid_startup_settings(&startup_settings) {
            startup_settings = empty_settings;
        }
        if startup_settings != previous_settings {
            store.write_json_ref(&settings_ref, &startup_settings).await
                .map_err(|error| error.to_string())?;
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
        let staged_picker_imports = super::file_import::picker_import_directory(&root);
        match tokio::fs::remove_dir_all(&staged_picker_imports).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Cannot clear stale picker imports: {error}"));
            }
        }
        let root = store.root().to_path_buf();
        let task_ref = store
            .document_ref("tasks")
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
            search_group_indexes: HashMap::new(),
            dirty_search_results: HashSet::new(),
            search_results_need_sort: false,
            published_search_error_count: 0,
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
            _process_lock: process_lock,
            store,
            server,
            executor,
            book_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            chapter_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            sources_lock: Arc::new(tokio::sync::Mutex::new(())),
            source_catalog: Arc::new(tokio::sync::Mutex::new(None)),
            source_revisions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tasks: Arc::new(tokio::sync::Mutex::new(task_registry)),
            task_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            search_slots: Arc::new(tokio::sync::Semaphore::new(task_execution::SEARCH_CONCURRENCY)),
            detail_preparation: Arc::new(std::sync::Mutex::new((0, None))),
            book_notifier: Arc::new(std::sync::RwLock::new(None)),
            task_notifier: Arc::new(std::sync::RwLock::new(None)),
            task_recovery_notifier: Arc::new(std::sync::RwLock::new(None)),
            search_notifiers: Arc::new(std::sync::RwLock::new(HashMap::new())),
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

    pub fn set_book_notifier(&self, notifier: Arc<dyn Fn(Value) + Send + Sync>) {
        if let Ok(mut target) = self.book_notifier.write() { *target = Some(notifier); }
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


    pub async fn operation_read(&self) -> OperationReadGuard {
        let mut barrier = self.restore_barrier.subscribe();
        loop {
            if *barrier.borrow() {
                let _ = barrier.changed().await;
                continue;
            }
            let admission = self.admission_gate.clone().read_owned().await;
            if *barrier.borrow() {
                drop(admission);
                continue;
            }
            let operation = self.operation_gate.clone().read_owned().await;
            if !*barrier.borrow() {
                return OperationReadGuard { _admission: admission, _operation: operation };
            }
            drop(operation);
            drop(admission);
        }
    }

    pub fn resource_descriptor(&self, resource: &ResourceRef) -> Value {
        if crate::resources::content::is_content_document(resource.path()) {
            let (_,id,_)=crate::resources::content::content_path(resource.path()).expect("book identity");
            return json!({"bookId":id});
        }
        if resource.path().ends_with(".json") {
            return json!({"updated":true});
        }
        json!({
            "resourceId": resource.as_str(),
            "src": self.server.url_for(resource),
            "contentType": content_type(resource.as_str()),
            "format": resource_format(resource.as_str()),
        })
    }
    pub(crate) fn book_engine_path(&self, id: &str) -> Result<PathBuf, String> {
        self.store.book_ref(id).map(|reference| PathBuf::from(reference.path())).map_err(|error| error.to_string())
    }

    pub(crate) async fn read_internal_json(
        &self,
        relative: impl AsRef<Path>,
    ) -> Result<Value, String> {
        let relative = relative.as_ref();
        if crate::resources::content::is_content_document(&relative.to_string_lossy().replace('\\', "/")) {
            let reference = ResourceRef::new(format!("resource://{}", relative.to_string_lossy().replace('\\', "/"))).map_err(|error| error.to_string())?;
            let database=self.store.database().clone();
            let path=self.store.canonical_reference(&reference).map_err(|e|e.to_string())?.path().to_owned();
            return tokio::task::spawn_blocking(move || -> Result<Value,String> {
                let document=database.read(&path).map_err(|e|e.to_string())?.ok_or("Content does not exist")?;
                if !document["engine"].is_object() {return Err("Content has no source execution data".into());}
                Ok(crate::resources::content::source_state(&document))
            }).await.map_err(|e|e.to_string())?;
        }
        let key = relative.to_string_lossy().replace('\\',"/");
        let path = self.root.join(relative);
        crate::resources::check_path_no_symlink(&self.root,&path,true).map_err(|e|e.to_string())?;
        let db = self.store.database().clone();
        tokio::task::spawn_blocking(move || db.read(&key)?.ok_or_else(||crate::resources::ResourceError::new("Private state does not exist")))
            .await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())
    }

    pub(crate) async fn write_internal_json(
        &self,
        relative: impl AsRef<Path>,
        value: &Value,
    ) -> Result<(), String> {
        let relative = relative.as_ref();
        let relative_string = relative.to_string_lossy().replace('\\', "/");
        if crate::resources::content::is_content_document(&relative_string) {
            let (_, id, _) = crate::resources::content::content_path(&relative_string).expect("content path");
            let reference = self.store.book_ref(id).map_err(|error| error.to_string())?;
            let current = self.store.read_json_ref(&reference).await.unwrap_or_else(|_| json!({"id": id, "title": "Untitled", "chapters": []}));
            return self.store.write_book_state(id, &current, value).await.map_err(|error| error.to_string());
        }
        let path = self.root.join(relative);
        crate::resources::check_path_no_symlink(&self.root,&path,true).map_err(|e|e.to_string())?;
        let db = self.store.database().clone();
        let value = value.clone();
        tokio::task::spawn_blocking(move || db.write(&relative_string,&value))
            .await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())
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
    serde_json::from_value::<crate::models::SettingsDocument>(value.clone()).is_ok_and(|settings| {
        settings.schema_version == CURRENT_SCHEMA_VERSION
            && crate::discovery::home_config::validate_home_document(&settings.home).is_ok()
    })
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

#[cfg(test)]
mod settings_startup_tests {
    use super::*;

    struct UnusedExecutor;
    impl SourceExecutor for UnusedExecutor {
        fn execute<'a>(&'a self, _request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async { Err("Settings initialization must not execute a source".into()) })
        }
    }

    #[tokio::test]
    async fn startup_persists_complete_settings_and_saves_both_reading_profiles() {
        let fresh = tempfile::tempdir().unwrap();
        let service = ApplicationService::open_with_executor(fresh.path(), Arc::new(UnusedExecutor)).await.unwrap();
        let settings = service.store.read_json_ref(&service.store.settings_ref()).await.unwrap();
        assert_eq!(settings["theme"], "system");
        assert_eq!(settings["reader"]["themes"], json!({"light":"paper","dark":"dark"}));
        assert_eq!(settings["reader"]["audioPlaybackRate"], 1.0);
        assert_eq!(settings["reader"]["audioSkipSeconds"], 15);

        let legacy = tempfile::tempdir().unwrap();
        let mut settings = serde_json::to_value(crate::models::SettingsDocument::default()).unwrap();
        settings.as_object_mut().unwrap().remove("theme");
        settings["reader"] = json!({"theme":"sepia","fontSizePx":25,"ttsRate":1.75});
        std::fs::write(legacy.path().join("settings.json"), serde_json::to_vec(&settings).unwrap()).unwrap();
        let service = ApplicationService::open_with_executor(legacy.path(), Arc::new(UnusedExecutor)).await.unwrap();
        let mut persisted = service.store.read_json_ref(&service.store.settings_ref()).await.unwrap();
        assert_eq!(persisted["theme"], "light", "Settings remain in their existing JSON file");
        assert_eq!(persisted["reader"]["themes"]["light"], "sepia");
        assert_eq!(persisted["reader"]["fontSizePx"], 25);
        persisted["reader"]["themes"]["light"] = json!("sepia");
        persisted["reader"]["fontSizePx"] = json!(25);
        assert_eq!(persisted["reader"]["audioSkipSeconds"], 15);
        persisted["theme"] = json!("dark");
        persisted["reader"]["themes"]["dark"] = json!("paper");
        service.save_settings(persisted).await.unwrap();
        let saved = service.store.read_json_ref(&service.store.settings_ref()).await.unwrap();
        assert_eq!(saved["theme"], "dark");
        assert_eq!(saved["reader"]["themes"], json!({"light":"sepia","dark":"paper"}));
        assert_eq!(saved["reader"]["fontSizePx"], 25);
    }
}
