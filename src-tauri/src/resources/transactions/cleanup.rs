//! 执行提交后的缓存清理并移除完成的事务日志。

use super::*;
use std::fs;

pub(super) fn apply_post_commit_deletes(
    root: &Path,
    journal: &Journal,
) -> Result<(), TransactionError> {
    for relative_path in &journal.post_commit_deletes {
        if is_chapter_resource_path(relative_path) {
            let reference = ResourceRef::new(format!("resource://{relative_path}"))
                .map_err(|_| TransactionError::new("Unsafe post-commit cleanup reference"))?;
            let path = root.join(reference.path());
            reject_symlink_components(root, &path, true)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(TransactionError::new(format!(
                        "Cannot delete cached chapter {}: {error}",
                        reference.path()
                    )));
                }
            }
            // Also sync on NotFound: a previous cleanup may have unlinked the
            // path but crashed before its parent directory was flushed.
            sync_parent(root, &path)?;
        } else if is_book_directory_path(relative_path) {
            let path = root.join(relative_path);
            reject_symlink_components(root, &path, true)?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    validate_removable_tree(&path)?;
                    fs::remove_dir_all(&path).map_err(|error| {
                        TransactionError::new(format!(
                            "Cannot delete cached book directory {relative_path}: {error}"
                        ))
                    })?;
                }
                Ok(_) => {
                    return Err(TransactionError::new(format!(
                        "Cached book cleanup target is not a regular directory: {relative_path}"
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(TransactionError::new(format!(
                        "Cannot inspect cached book directory {relative_path}: {error}"
                    )));
                }
            }
            sync_parent(root, &path)?;
        } else {
            return Err(TransactionError::new(
                "Post-commit cleanup path is not a typed chapter or book directory",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_removable_tree(path: &Path) -> Result<(), TransactionError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        TransactionError::new(format!("Cannot inspect cached book entry: {error}"))
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(TransactionError::new(
            "Refusing book cleanup because the cache contains a symbolic link",
        ));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(TransactionError::new(
            "Refusing book cleanup because the cache contains a special file",
        ));
    }
    for entry in fs::read_dir(path).map_err(|error| {
        TransactionError::new(format!("Cannot list cached book directory: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            TransactionError::new(format!("Cannot inspect cached book entry: {error}"))
        })?;
        validate_removable_tree(&entry.path())?;
    }
    Ok(())
}

pub(super) fn finish_committed_transaction(
    root: &Path,
    stage_dir: &Path,
) -> Result<(), TransactionError> {
    finish_transaction_directory(root, stage_dir)
}

pub(super) fn finish_rolled_back_transaction(
    root: &Path,
    stage_dir: &Path,
) -> Result<(), TransactionError> {
    finish_transaction_directory(root, stage_dir)
}

pub(super) fn finish_transaction_directory(
    root: &Path,
    stage_dir: &Path,
) -> Result<(), TransactionError> {
    let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
    validate_existing_path_no_symlinks(root, stage_dir)?;
    let journal_path = stage_dir.join("journal.json");
    match fs::remove_file(&journal_path) {
        Ok(()) => sync_directory(stage_dir)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot remove completed transaction journal: {error}"
            )));
        }
    }
    fs::remove_dir_all(stage_dir).map_err(|error| {
        TransactionError::new(format!(
            "Cannot remove completed transaction journal: {error}"
        ))
    })?;
    sync_directory(&transactions_dir)
}
