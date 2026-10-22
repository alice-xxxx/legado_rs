//! 备份恢复的状态与数据目录进程锁。
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
                ".legado-{}.lock",
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
