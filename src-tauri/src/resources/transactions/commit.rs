//! Publish immutable assets, then commit all metadata and deletion intents.
use super::*;

impl<'guard> FileTransaction<'guard> {
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
                "A transaction must replace at least one target",
            ));
        }
        validate_unique_entries(&replacements, &post_commit_deletes)?;
        let mut total = 0;
        for replacement in &replacements {
            validate_target_path(&root, &replacement.target, true)?;
            if let Some(bytes) = &replacement.bytes {
                validate_target_payload(&replacement.target, bytes)?;
                add_snapshot_size(&mut total, bytes.len() as u64)?;
            }
        }
        Ok(Self {
            root,
            replacements,
            post_commit_deletes: post_commit_deletes
                .into_iter()
                .map(|entry| entry.path)
                .collect(),
            _writer_guard: PhantomData,
        })
    }

    pub(crate) fn commit(self) -> Result<(), TransactionError> {
        let db = crate::database::Database::at(&self.root);
        // Publish immutable chapter bodies before committing the references.
        // An interrupted operation can leave an unused body, never a broken ref.
        for replacement in &self.replacements {
            let path = replacement.target.relative_path();
            if !path.ends_with(".json") {
                let bytes = replacement.bytes.as_deref().ok_or_else(|| {
                    TransactionError::new(
                        "Native transaction bodies cannot be deleted before commit",
                    )
                })?;
                let target = self.root.join(&path);
                if let Some(existing) = read_native_body(&self.root, &target)? {
                    if existing != bytes {
                        return Err(TransactionError::new(
                            "Cannot replace an immutable chapter resource",
                        ));
                    }
                } else {
                    atomic_replace(&self.root, &target, bytes)?;
                }
            }
        }
        db.transaction(|tx| {
            for replacement in &self.replacements {
                let path = replacement.target.relative_path();
                if !path.ends_with(".json") { continue; }
                if let Some(bytes) = &replacement.bytes {
                    let document: serde_json::Value = serde_json::from_slice(bytes).map_err(|e|crate::resources::ResourceError::new(e.to_string()))?;
                    crate::database::write(tx, &path, &document)?;
                    if crate::resources::content::is_content_document(&path) {
                        let (kind,id,_) = crate::resources::content::content_path(&path).expect("content path");
                        let (_,preview) = crate::database::book_preview(tx,id)?;
                        tx.execute("UPDATE records SET value=?1 WHERE path=?2 AND field=?3 AND item_id=?4 AND value!=?1",
                            rusqlite::params![preview.to_string(),crate::resources::content::LIBRARY_FILE,kind,id])?;
                    }
                } else { tx.execute("DELETE FROM documents WHERE path=?1",[path])?; }
            }
            for path in &self.post_commit_deletes {
                tx.execute("INSERT OR IGNORE INTO pending_cleanup(path) VALUES(?1)",[path])?;
            }
            Ok(())
        }).map_err(|error|TransactionError::new(error.to_string()))?;
        super::recovery::recover_database_cleanup(&self.root)
    }
}

fn read_native_body(root: &Path, path: &Path) -> Result<Option<Vec<u8>>, TransactionError> {
    reject_symlink_components(root, path, true)?;
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(TransactionError::new(error.to_string())),
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_TARGET_BYTES => {
            std::fs::read(path)
                .map(Some)
                .map_err(|e| TransactionError::new(e.to_string()))
        }
        Ok(_) => Err(TransactionError::new(
            "Native body is not a regular file within the size limit",
        )),
    }
}
