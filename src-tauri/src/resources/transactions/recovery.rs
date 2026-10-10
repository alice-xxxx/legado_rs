//! 扫揻待恢复日志，并回滚未提交事务。

use super::*;
use std::fs;

/// Recover all journals in the app-data root before opening ResourceStore.
/// Prepared transactions restore exact before-images; committed transactions
/// replay after-images and cleanup. Invalid/multiple journals fail closed.
pub(crate) fn recover_all(root: &Path) -> Result<(), TransactionError> {
    let root = canonical_root(root)?;
    let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
    match fs::symlink_metadata(&transactions_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot inspect transaction journal directory: {error}"
            )));
        }
        Ok(_) => {}
    }
    validate_existing_path_no_symlinks(&root, &transactions_dir)?;
    let mut journal_dirs = Vec::new();
    let mut orphan_dirs = Vec::new();
    for entry in fs::read_dir(&transactions_dir).map_err(|error| {
        TransactionError::new(format!("Cannot scan transaction journal: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            TransactionError::new(format!("Cannot read transaction journal entry: {error}"))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            TransactionError::new(format!("Cannot inspect transaction entry: {error}"))
        })?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(TransactionError::new(
                "Unexpected file type in transaction staging directory",
            ));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        validate_transaction_id(&name)?;
        let path = entry.path();
        let journal_path = path.join("journal.json");
        match fs::symlink_metadata(&journal_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(TransactionError::new(
                    "Transaction journal is not a regular file",
                ));
            }
            Ok(_) => journal_dirs.push((name, path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => orphan_dirs.push(path),
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect transaction journal: {error}"
                )));
            }
        }
    }
    if journal_dirs.len() > 1 {
        return Err(TransactionError::new(
            "Multiple pending file transactions found; refusing to guess recovery order",
        ));
    }
    for orphan in orphan_dirs {
        fs::remove_dir_all(orphan).map_err(|error| {
            TransactionError::new(format!("Cannot remove orphan transaction staging: {error}"))
        })?;
    }
    if !journal_dirs.is_empty() {
        sync_directory(&transactions_dir)?;
    }

    if let Some((transaction_id, stage_dir)) = journal_dirs.pop() {
        let journal = read_journal(&stage_dir, &transaction_id)?;
        match journal.phase {
            TransactionPhase::Prepared => rollback_prepared(&root, &stage_dir, &journal)?,
            TransactionPhase::Committed => {
                // A committed transaction must redo one complete generation.
                // Validate all after-images before replacing its first target.
                for entry in &journal.targets {
                    if !entry.after.missing {
                        let target = TransactionTarget::from_journal(entry.domain, &entry.path)?;
                        read_validated_payload(&stage_dir, &target, &entry.after)?;
                    }
                }
                for entry in &journal.targets {
                    let target = TransactionTarget::from_journal(entry.domain, &entry.path)?;
                    restore_snapshot(&root, &stage_dir, &target, &entry.after)?;
                }
                apply_post_commit_deletes(&root, &journal)?;
                finish_committed_transaction(&root, &stage_dir)?;
            }
        }
    }
    Ok(())
}

pub(super) fn rollback_prepared(
    root: &Path,
    stage_dir: &Path,
    journal: &Journal,
) -> Result<(), TransactionError> {
    if journal.phase != TransactionPhase::Prepared {
        return Err(TransactionError::new(
            "Refusing to roll back a committed transaction",
        ));
    }
    // Validate the complete rollback set before changing any target. If one
    // before-image is damaged, preserve the generation currently on disk and
    // leave the journal intact for explicit recovery handling.
    for entry in &journal.targets {
        if !entry.before.missing {
            let target = TransactionTarget::from_journal(entry.domain, &entry.path)?;
            read_validated_payload(stage_dir, &target, &entry.before)?;
        }
    }
    // Attempt every target even after one error. The journal remains intact if
    // any restore fails, so the next startup can retry the complete rollback.
    let mut failures = Vec::new();
    for entry in &journal.targets {
        let result = TransactionTarget::from_journal(entry.domain, &entry.path)
            .and_then(|target| restore_snapshot(root, stage_dir, &target, &entry.before));
        if let Err(error) = result {
            failures.push(error.to_string());
        }
    }
    if !failures.is_empty() {
        return Err(TransactionError::new(format!(
            "Could not restore all before-images: {}",
            failures.join("; ")
        )));
    }
    finish_rolled_back_transaction(root, stage_dir)
}

pub(super) fn restore_snapshot(
    root: &Path,
    stage_dir: &Path,
    target: &TransactionTarget,
    snapshot: &Snapshot,
) -> Result<(), TransactionError> {
    let relative_path = target.relative_path();
    let absolute_path = root.join(&relative_path);
    validate_target_path(root, target, true)?;
    if snapshot.missing {
        match fs::remove_file(&absolute_path) {
            Ok(()) => sync_parent(root, &absolute_path),
            // A prior rollback may have unlinked the target but stopped before
            // syncing its containing directory. Retry that sync even when the
            // target is already absent; its parent may itself have been
            // removed, so sync the nearest existing parent under the root.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                sync_parent(root, &absolute_path)
            }
            Err(error) => Err(TransactionError::new(format!(
                "Cannot remove transaction target {relative_path}: {error}"
            ))),
        }
    } else {
        let bytes = read_validated_payload(stage_dir, target, snapshot)?;
        atomic_replace(&root, &absolute_path, &bytes)
    }
}

/// Fail closed when ordinary ResourceStore writes encounter an unrecovered
/// transaction. Orphan staging directories without journals are harmless: no
/// target may be changed before the prepared journal is durable.
pub(crate) fn ensure_no_unrecovered_transaction(root: &Path) -> Result<(), TransactionError> {
    let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
    match fs::symlink_metadata(&transactions_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot inspect transaction staging directory: {error}"
            )));
        }
        Ok(_) => {}
    }
    validate_existing_path_no_symlinks(root, &transactions_dir)?;
    for entry in fs::read_dir(&transactions_dir).map_err(|error| {
        TransactionError::new(format!("Cannot inspect existing transactions: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            TransactionError::new(format!("Cannot inspect existing transaction: {error}"))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            TransactionError::new(format!("Cannot inspect existing transaction: {error}"))
        })?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(TransactionError::new(
                "Unexpected file type in transaction staging directory",
            ));
        }
        validate_transaction_id(&entry.file_name().to_string_lossy())?;
        match fs::symlink_metadata(entry.path().join("journal.json")) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(TransactionError::new(
                    "Transaction journal is not a regular file",
                ));
            }
            Ok(_) => {
                return Err(TransactionError::recovery_required(
                    "A previous file transaction needs recovery before resource writes",
                    CommitState::NotCommitted,
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect previous transaction journal: {error}"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn ensure_no_pending_transaction(root: &Path) -> Result<(), TransactionError> {
    let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
    match fs::symlink_metadata(&transactions_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot inspect transaction staging directory: {error}"
            )));
        }
        Ok(_) => {}
    }
    validate_existing_path_no_symlinks(root, &transactions_dir)?;
    for entry in fs::read_dir(&transactions_dir).map_err(|error| {
        TransactionError::new(format!("Cannot inspect existing transactions: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            TransactionError::new(format!("Cannot inspect existing transaction: {error}"))
        })?;
        if entry
            .file_type()
            .map_err(|error| {
                TransactionError::new(format!("Cannot inspect existing transaction: {error}"))
            })?
            .is_symlink()
        {
            return Err(TransactionError::new(
                "Symbolic link found in transaction staging directory",
            ));
        }
        if entry
            .file_type()
            .map_err(|error| {
                TransactionError::new(format!("Cannot inspect existing transaction: {error}"))
            })?
            .is_dir()
        {
            let dir = entry.path();
            match fs::symlink_metadata(dir.join("journal.json")) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(TransactionError::new(
                        "Transaction journal is not a regular file",
                    ));
                }
                Ok(_) => {
                    return Err(TransactionError::recovery_required(
                        "A previous file transaction needs recovery before writing",
                        CommitState::NotCommitted,
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(TransactionError::new(format!(
                        "Cannot inspect previous transaction journal: {error}"
                    )));
                }
            }
            validate_transaction_id(&entry.file_name().to_string_lossy())?;
            fs::remove_dir_all(&dir).map_err(|error| {
                TransactionError::new(format!("Cannot remove orphan transaction stage: {error}"))
            })?;
        } else {
            return Err(TransactionError::new(
                "Unexpected file in transaction staging directory",
            ));
        }
    }
    sync_directory(&transactions_dir)
}
