//! 序列化事务日志并验证恢复记录。

use super::*;
use std::collections::HashSet;
use std::fs;

pub(super) fn write_journal(stage_dir: &Path, journal: &Journal) -> Result<(), TransactionError> {
    validate_journal(journal, &journal.transaction_id)?;
    let bytes = serde_json::to_vec_pretty(journal).map_err(|error| {
        TransactionError::new(format!("Cannot encode transaction journal: {error}"))
    })?;
    let journal_path = stage_dir.join("journal.json");
    atomic_replace_path(&journal_path, &bytes)?;
    sync_directory(stage_dir)
}

pub(super) fn read_journal(
    stage_dir: &Path,
    transaction_id: &str,
) -> Result<Journal, TransactionError> {
    let path = stage_dir.join("journal.json");
    reject_symlink_components(stage_dir, &path, false)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        TransactionError::new(format!("Cannot inspect transaction journal: {error}"))
    })?;
    if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 {
        return Err(TransactionError::new(
            "Transaction journal has invalid type or size",
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        TransactionError::new(format!("Cannot read transaction journal: {error}"))
    })?;
    let journal: Journal = serde_json::from_slice(&bytes)
        .map_err(|error| TransactionError::new(format!("Invalid transaction journal: {error}")))?;
    validate_journal(&journal, transaction_id)?;
    Ok(journal)
}

pub(super) fn validate_journal(
    journal: &Journal,
    expected_id: &str,
) -> Result<(), TransactionError> {
    validate_transaction_id(expected_id)?;
    if journal.format != JOURNAL_FORMAT
        || journal.version != JOURNAL_VERSION
        || journal.transaction_id != expected_id
    {
        return Err(TransactionError::new(
            "Transaction journal format, version, or identifier is invalid",
        ));
    }
    validate_operation(&journal.operation)?;
    if journal.targets.is_empty() {
        return Err(TransactionError::new("Transaction journal has no targets"));
    }
    let mut targets = HashSet::new();
    let mut total_bytes = 0u64;
    for (index, entry) in journal.targets.iter().enumerate() {
        let target = TransactionTarget::from_journal(entry.domain, &entry.path)?;
        if target.relative_path() != entry.path || !targets.insert(entry.path.clone()) {
            return Err(TransactionError::new(
                "Transaction journal has a duplicate or noncanonical target path",
            ));
        }
        validate_snapshot(&entry.before, "before", index)?;
        validate_snapshot(&entry.after, "after", index)?;
        if !entry.before.missing {
            add_snapshot_size(&mut total_bytes, entry.before.bytes)?;
        }
        if !entry.after.missing {
            add_snapshot_size(&mut total_bytes, entry.after.bytes)?;
        }
        if entry.after.missing && !is_deletable_target(&target) {
            return Err(TransactionError::new(
                "Only typed book/progress/private-book targets may be deleted by a transaction",
            ));
        }
    }
    let mut deletes = HashSet::<&str>::new();
    for path in &journal.post_commit_deletes {
        if !is_chapter_resource_path(path) && !is_book_directory_path(path) {
            return Err(TransactionError::new(
                "Journal cleanup path is not a typed chapter or book directory",
            ));
        }
        if is_chapter_resource_path(path) {
            ResourceRef::new(format!("resource://{path}"))
                .map_err(|_| TransactionError::new("Unsafe journal cleanup path"))?;
        }
        if !deletes.insert(path.as_str()) {
            return Err(TransactionError::new("Duplicate journal cleanup path"));
        }
    }
    for path in &journal.post_commit_deletes {
        if has_cleanup_ancestor(path, &deletes) {
            return Err(TransactionError::new("Journal cleanup paths overlap"));
        }
    }
    for entry in &journal.targets {
        if !entry.after.missing
            && (deletes.contains(entry.path.as_str())
                || has_cleanup_ancestor(&entry.path, &deletes))
        {
            return Err(TransactionError::new(
                "Journal cleanup would delete a newly written target",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_snapshot(
    snapshot: &Snapshot,
    area: &str,
    index: usize,
) -> Result<(), TransactionError> {
    if snapshot.missing {
        if snapshot.blob.is_some() || snapshot.bytes != 0 || snapshot.sha256.is_some() {
            return Err(TransactionError::new(
                "Missing snapshot contains unexpected payload metadata",
            ));
        }
        return Ok(());
    }
    let expected_blob = format!("{area}/{index:06}.bin");
    if snapshot.blob.as_deref() != Some(expected_blob.as_str())
        || snapshot.bytes > MAX_TARGET_BYTES
        || !snapshot.sha256.as_ref().is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err(TransactionError::new(
            "Invalid transaction snapshot metadata",
        ));
    }
    Ok(())
}
