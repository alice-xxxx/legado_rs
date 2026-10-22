//! 应用服务共享状态、操作门闩、资源入口与书架锁。

use super::{login, search, tts};
use crate::{
    models::CURRENT_SCHEMA_VERSION,
    resources::{ResourceServer, ResourceStore},
    settings::configured_source_http_timeout_seconds,
    source_engine::SourceExecutor,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc},
};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::State;

impl ApplicationService {
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub fn set_source_engine_app(&mut self, app: tauri::AppHandle) {
        self.app = Some(app);
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
            store
                .write_json_ref(&settings_ref, &startup_settings)
                .await
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
        let staged_picker_imports = crate::file_access::picker_import_directory(&root);
        match tokio::fs::remove_dir_all(&staged_picker_imports).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Cannot clear stale picker imports: {error}"));
            }
        }
        let root = store.root().to_path_buf();
        let (restore_barrier, _) = tokio::sync::watch::channel(false);
        Ok(Self {
            root,
            _process_lock: process_lock,
            sources: Arc::new(crate::source_catalog::SourceCatalog::new(
                store.database().clone(),
            )),
            store,
            server,
            executor,
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            app: None,
            book_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            search_slots: Arc::new(tokio::sync::Semaphore::new(search::SEARCH_CONCURRENCY)),
            information_cancel: tokio::sync::broadcast::channel(64).0,
            text_cancel: tokio::sync::broadcast::channel(64).0,
            tts: tokio::sync::watch::channel(None).0,
            login: tokio::sync::watch::channel(None).0,
            searches: Arc::new(tokio::sync::Mutex::new(search::SearchRegistry::default())),
            admission_gate: Arc::new(tokio::sync::RwLock::new(())),
            operation_gate: Arc::new(tokio::sync::RwLock::new(())),
            webdav_backup_upload_gate: Arc::new(tokio::sync::Mutex::new(())),
            webdav_backup_config_gate: Arc::new(tokio::sync::Mutex::new(())),
            restore_barrier: Arc::new(restore_barrier),
            restore_serial: Arc::new(tokio::sync::Mutex::new(())),
            restore_epoch: Arc::new(AtomicU64::new(0)),
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
                return OperationReadGuard {
                    _admission: admission,
                    _operation: operation,
                };
            }
            drop(operation);
            drop(admission);
        }
    }

    pub(crate) async fn book_lock(&self, book_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
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

fn valid_startup_settings(value: &Value) -> bool {
    serde_json::from_value::<crate::models::SettingsDocument>(value.clone()).is_ok_and(|settings| {
        settings.schema_version == CURRENT_SCHEMA_VERSION
            && crate::discovery::home_config::validate_home_document(&settings.home).is_ok()
    })
}

/// Resource-oriented service for source processing and resource delivery.
#[derive(Clone)]
pub struct ApplicationService {
    pub(crate) root: PathBuf,
    pub(crate) _process_lock: Arc<crate::resource_transactions::AppDataProcessLock>,
    pub(crate) store: Arc<ResourceStore>,
    pub(crate) server: Arc<ResourceServer>,
    pub(crate) executor: Arc<dyn SourceExecutor>,
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(crate) app: Option<tauri::AppHandle>,
    pub(crate) book_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    pub(crate) sources: Arc<crate::source_catalog::SourceCatalog>,
    pub(crate) searches: Arc<tokio::sync::Mutex<search::SearchRegistry>>,
    pub(crate) search_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) information_cancel: tokio::sync::broadcast::Sender<Option<(String, String)>>,
    pub(crate) text_cancel: tokio::sync::broadcast::Sender<Option<(String, String)>>,
    pub(crate) tts: tokio::sync::watch::Sender<Option<tts::TtsSession>>,
    pub(crate) login: tokio::sync::watch::Sender<Option<Arc<login::LoginSession>>>,
    pub(crate) admission_gate: Arc<tokio::sync::RwLock<()>>,
    pub(crate) operation_gate: Arc<tokio::sync::RwLock<()>>,
    pub(crate) webdav_backup_upload_gate: Arc<tokio::sync::Mutex<()>>,
    pub(crate) webdav_backup_config_gate: Arc<tokio::sync::Mutex<()>>,
    pub(crate) restore_barrier: Arc<tokio::sync::watch::Sender<bool>>,
    pub(crate) restore_serial: Arc<tokio::sync::Mutex<()>>,
    pub(crate) restore_epoch: Arc<AtomicU64>,
}

pub(crate) struct RestoreAdmissionGuard {
    pub(crate) barrier: Arc<tokio::sync::watch::Sender<bool>>,
    pub(crate) release_on_drop: bool,
}

pub struct OperationReadGuard {
    pub(crate) _admission: tokio::sync::OwnedRwLockReadGuard<()>,
    pub(crate) _operation: tokio::sync::OwnedRwLockReadGuard<()>,
}

impl Drop for RestoreAdmissionGuard {
    fn drop(&mut self) {
        if self.release_on_drop {
            self.barrier.send_replace(false);
        }
    }
}

impl ApplicationService {
    pub async fn bootstrap(&self) -> Result<Value, String> {
        let settings = self
            .store
            .read_json_ref(&self.store.settings_ref())
            .await
            .map_err(|error| error.to_string())?;
        crate::source_http::set_network_timeout_seconds(configured_source_http_timeout_seconds(
            Some(&settings),
        ));
        let base = self.server.url_for(&self.store.settings_ref());
        Ok(json!({"settings":settings,"assetBaseUrl":base.trim_end_matches("settings.json")}))
    }
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn app_bootstrap(service: State<'_, ApplicationService>) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    service.bootstrap().await
}
