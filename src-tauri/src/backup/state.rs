//! 备份恢复的状态与数据目录进程锁。
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitState {
    NotCommitted,
    Committed,
    Indeterminate,
}
#[derive(Debug)]
pub(crate) struct TransactionError {
    message: String,
    state: CommitState,
    recovery: bool,
}
impl TransactionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            state: CommitState::NotCommitted,
            recovery: false,
        }
    }
    pub(crate) fn recovery_required(message: impl Into<String>, state: CommitState) -> Self {
        Self {
            message: message.into(),
            state,
            recovery: true,
        }
    }
    pub(crate) fn commit_state(&self) -> CommitState {
        self.state
    }
    pub(crate) fn recovery_required_flag(&self) -> bool {
        self.recovery
    }
}
impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for TransactionError {}

pub(crate) struct AppDataProcessLock {
    _file: File,
    root: PathBuf,
}
impl AppDataProcessLock {
    pub(crate) fn acquire(root: &Path) -> Result<Self, TransactionError> {
        let acquire = || -> std::io::Result<Self> {
            let name = root
                .file_name()
                .ok_or_else(|| std::io::Error::other("数据目录缺少名称"))?;
            let parent = root
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::create_dir_all(parent)?;
            let parent = std::fs::canonicalize(parent)?;
            let root = parent.join(name);
            let path = parent.join(format!(
                ".foliver-{}.lock",
                crate::support::sha256_hex(root.to_string_lossy().as_bytes())
            ));
            for candidate in [&root, &path] {
                if std::fs::symlink_metadata(candidate).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err(std::io::Error::other("数据目录或进程锁不允许符号链接"));
                }
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            file.try_lock().map_err(std::io::Error::other)?;
            Ok(Self { _file: file, root })
        };
        acquire().map_err(|error| TransactionError::new(format!("无法独占数据目录：{error}")))
    }
    pub(crate) fn data_root(&self) -> &Path {
        &self.root
    }
}

/// 备份恢复的准入屏障；各模块只共享操作时序。
pub(crate) struct Operations {
    pub(crate) admission_gate: std::sync::Arc<tokio::sync::RwLock<()>>,
    pub(crate) operation_gate: std::sync::Arc<tokio::sync::RwLock<()>>,
    pub(crate) webdav_backup_upload_gate: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub(crate) restore_barrier: std::sync::Arc<tokio::sync::watch::Sender<bool>>,
    pub(crate) restore_serial: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub(crate) restore_epoch: std::sync::atomic::AtomicU64,
}
impl Operations {
    pub(crate) fn new() -> Self {
        Self {
            admission_gate: Default::default(),
            operation_gate: Default::default(),
            webdav_backup_upload_gate: Default::default(),
            restore_barrier: std::sync::Arc::new(tokio::sync::watch::channel(false).0),
            restore_serial: Default::default(),
            restore_epoch: std::sync::atomic::AtomicU64::new(0),
        }
    }
    pub(crate) async fn operation_read(&self) -> OperationReadGuard {
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
}
pub(crate) struct RestoreAdmissionGuard {
    pub(crate) barrier: std::sync::Arc<tokio::sync::watch::Sender<bool>>,
    pub(crate) release_on_drop: bool,
}

pub(crate) struct OperationReadGuard {
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavBackupConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub auto_interval_hours: Option<u32>,
    #[serde(default)]
    pub last_webdav_backup_at_ms: Option<u64>,
    #[serde(default)]
    pub auto_backup_scheduled_at_ms: Option<u64>,
    #[serde(default)]
    pub last_auto_attempt_at_ms: Option<u64>,
}
