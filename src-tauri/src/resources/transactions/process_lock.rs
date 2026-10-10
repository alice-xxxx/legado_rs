//! 获取应用数据目录的跨进程写锁。

use super::*;
use std::fs::{self, OpenOptions};

impl AppDataProcessLock {
    pub(crate) fn acquire(root: &Path) -> Result<Self, TransactionError> {
        let root_name = root
            .file_name()
            .ok_or_else(|| TransactionError::new("App-data root has no folder name"))?;
        let requested_parent = root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        let requested_parent = requested_parent.unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(requested_parent).map_err(|error| {
            TransactionError::new(format!("Cannot create app-data parent: {error}"))
        })?;
        let parent = fs::canonicalize(requested_parent).map_err(|error| {
            TransactionError::new(format!("Cannot resolve app-data parent: {error}"))
        })?;
        let canonical_root = parent.join(root_name);
        match fs::symlink_metadata(&canonical_root) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(TransactionError::new(
                    "App-data root must be a regular directory, not a symbolic link",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect app-data root: {error}"
                )));
            }
        }
        let parent = canonical_root
            .parent()
            .ok_or_else(|| TransactionError::new("App-data root has no parent directory"))?;
        let hash = hex_digest(canonical_root.to_string_lossy().as_bytes());
        let lock_path = parent.join(format!(".legado-app-data-{hash}.lock"));
        match fs::symlink_metadata(&lock_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(TransactionError::new(
                    "App-data process lock path is not a regular file",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect app-data process lock: {error}"
                )));
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|error| {
                TransactionError::new(format!("Cannot open app-data lock: {error}"))
            })?;
        if !file
            .metadata()
            .map_err(|error| {
                TransactionError::new(format!("Cannot inspect app-data lock: {error}"))
            })?
            .is_file()
        {
            return Err(TransactionError::new(
                "App-data process lock path is not a regular file",
            ));
        }
        file.try_lock().map_err(|error| {
            TransactionError::new(format!(
                "App-data root is already in use by another process or locking is unavailable: {error}"
            ))
        })?;
        Ok(Self {
            _file: file,
            canonical_root,
        })
    }

    pub(crate) fn data_root(&self) -> &Path {
        &self.canonical_root
    }
}
