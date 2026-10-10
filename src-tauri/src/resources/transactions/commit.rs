//! 准备事务快照、发布提交标记并应用目标文件。

use super::*;
use std::fs;
use uuid::Uuid;

impl<'guard> FileTransaction<'guard> {
    /// Stage exact before/after bytes and durably publish the `prepared` journal.
    /// No target file is modified by this operation.
    pub(crate) fn prepare(
        root: &Path,
        operation: &str,
        replacements: Vec<Replacement>,
        post_commit_deletes: Vec<PostCommitDelete>,
        guard: &'guard dyn ResourceWriterGuard,
    ) -> Result<Self, TransactionError> {
        let root = canonical_root(root)?;
        validate_guard_root(guard, &root)?;
        validate_operation(operation)?;
        if replacements.is_empty() {
            return Err(TransactionError::new(
                "A file transaction must replace at least one target",
            ));
        }
        ensure_no_pending_transaction(&root)?;
        validate_unique_entries(&replacements, &post_commit_deletes)?;

        let transaction_id = format!("txn-{}", Uuid::new_v4().simple());
        let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
        ensure_safe_directory(&root, &transactions_dir)?;
        let stage_dir = transactions_dir.join(&transaction_id);
        fs::create_dir(&stage_dir).map_err(|error| {
            TransactionError::new(format!(
                "Cannot create transaction staging directory: {error}"
            ))
        })?;
        sync_directory(&transactions_dir)?;

        let stage_result = (|| {
            let mut total_bytes = 0u64;
            let mut entries = Vec::with_capacity(replacements.len());
            for (index, replacement) in replacements.iter().enumerate() {
                if replacement
                    .bytes
                    .as_ref()
                    .is_some_and(|bytes| bytes.len() as u64 > MAX_TARGET_BYTES)
                {
                    return Err(TransactionError::new(
                        "A transaction target exceeds the 64 MiB safety limit",
                    ));
                }
                let target = replacement.target.clone();
                let relative_path = target.relative_path();
                validate_target_path(&root, &target, true)?;
                let absolute_path = root.join(&relative_path);
                let before = read_snapshot_bytes(&root, &absolute_path)?;
                let before_snapshot = match before {
                    Some(bytes) => {
                        validate_target_payload(&target, &bytes)?;
                        add_snapshot_size(&mut total_bytes, bytes.len() as u64)?;
                        write_payload(&stage_dir, "before", index, &bytes)?
                    }
                    None => Snapshot::missing(),
                };
                let after_snapshot = match replacement.bytes.as_deref() {
                    Some(bytes) => {
                        add_snapshot_size(&mut total_bytes, bytes.len() as u64)?;
                        write_payload(&stage_dir, "after", index, bytes)?
                    }
                    None => Snapshot::missing(),
                };
                entries.push(JournalTarget {
                    domain: target.domain(),
                    path: relative_path,
                    before: before_snapshot,
                    after: after_snapshot,
                });
            }

            let deletes = post_commit_deletes
                .into_iter()
                .map(|entry| entry.path)
                .collect();
            let journal = Journal {
                format: JOURNAL_FORMAT.to_owned(),
                version: JOURNAL_VERSION,
                transaction_id: transaction_id.clone(),
                operation: operation.to_owned(),
                phase: TransactionPhase::Prepared,
                targets: entries,
                post_commit_deletes: deletes,
            };
            validate_journal(&journal, &transaction_id)?;
            write_journal(&stage_dir, &journal)?;
            sync_directory(&stage_dir)?;
            Ok(journal)
        })();

        match stage_result {
            Ok(journal) => Ok(Self {
                root,
                stage_dir,
                journal,
                _writer_guard: PhantomData,
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(&stage_dir);
                let _ = sync_directory(&transactions_dir);
                Err(error)
            }
        }
    }

    /// Apply staged after-images, persist the commit decision, then perform
    /// idempotent post-commit cache cleanup.
    pub(crate) fn commit(self) -> Result<(), TransactionError> {
        self.commit_inner()
    }

    fn commit_inner(mut self) -> Result<(), TransactionError> {
        // Detect damaged after-images before the first target is changed. A
        // prepared journal remains recoverable by restoring its before-images.
        for entry in &self.journal.targets {
            if !entry.after.missing {
                let target = TransactionTarget::from_journal(entry.domain, &entry.path)?;
                if let Err(error) = read_validated_payload(&self.stage_dir, &target, &entry.after) {
                    return self.abort_prepared(error);
                }
            }
        }

        for index in 0..self.journal.targets.len() {
            if let Err(error) = self.apply_target(index) {
                return self.abort_prepared(error);
            }
        }

        let mut committed = self.journal.clone();
        committed.phase = TransactionPhase::Committed;
        if let Err(error) = write_journal(&self.stage_dir, &committed) {
            // The atomic journal replacement may have succeeded even if the
            // directory sync failed. Never roll back until disk says prepared.
            match read_journal(&self.stage_dir, &self.journal.transaction_id) {
                Ok(disk_journal) if disk_journal.phase == TransactionPhase::Prepared => {
                    self.journal = disk_journal;
                    return self.abort_prepared(error);
                }
                Ok(disk_journal) if disk_journal.phase == TransactionPhase::Committed => {
                    self.journal = disk_journal;
                    return Err(TransactionError::recovery_required(
                        format!("Commit marker is present but not fully synced: {error}"),
                        CommitState::Committed,
                    ));
                }
                _ => {
                    return Err(TransactionError::recovery_required(
                        format!("Cannot determine transaction commit state: {error}"),
                        CommitState::Indeterminate,
                    ));
                }
            }
        }
        self.journal = committed;

        if let Err(error) = apply_post_commit_deletes(&self.root, &self.journal) {
            return Err(TransactionError::recovery_required(
                format!("Files committed; cleanup will retry during startup: {error}"),
                CommitState::Committed,
            ));
        }
        match finish_committed_transaction(&self.root, &self.stage_dir) {
            Ok(()) => Ok(()),
            Err(error) => Err(TransactionError::recovery_required(
                format!("Files committed; journal cleanup will retry during startup: {error}"),
                CommitState::Committed,
            )),
        }
    }

    fn apply_target(&self, index: usize) -> Result<(), TransactionError> {
        let entry = self
            .journal
            .targets
            .get(index)
            .ok_or_else(|| TransactionError::new("Invalid transaction target index"))?;
        let target = TransactionTarget::from_journal(entry.domain, &entry.path)?;
        restore_snapshot(&self.root, &self.stage_dir, &target, &entry.after)
    }

    fn abort_prepared(self, original: TransactionError) -> Result<(), TransactionError> {
        match rollback_prepared(&self.root, &self.stage_dir, &self.journal) {
            Ok(()) => Err(TransactionError::aborted(format!(
                "Transaction was rolled back after a write error: {original}"
            ))),
            Err(rollback_error) => Err(TransactionError::recovery_required(
                format!(
                    "Transaction failed and rollback was incomplete; startup recovery is required. Original error: {original}; rollback error: {rollback_error}"
                ),
                CommitState::NotCommitted,
            )),
        }
    }
}
