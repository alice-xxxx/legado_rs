//! Retry committed cache deletion intents from SQLite after an interrupted run.
use super::*;

pub(crate) fn recover_all(root: &Path) -> Result<(), TransactionError> {
    let root = canonical_root(root)?;
    recover_database_cleanup(&root)
}

pub(crate) fn ensure_no_unrecovered_transaction(root: &Path) -> Result<(), TransactionError> {
    if !root.join(crate::database::FILE_NAME).is_file() {
        return Ok(());
    }
    let connection = crate::database::Database::at(root)
        .connect()
        .map_err(|e| TransactionError::new(e.to_string()))?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| TransactionError::new(e.to_string()))?;
    if version == 0 {
        return Ok(());
    }
    let pending: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM pending_cleanup)", [], |row| {
            row.get(0)
        })
        .map_err(|e| TransactionError::new(e.to_string()))?;
    if pending {
        return Err(TransactionError::recovery_required(
            "Committed cache cleanup must finish at startup before publishing more assets",
            CommitState::Committed,
        ));
    }
    Ok(())
}

pub(super) fn recover_database_cleanup(root: &Path) -> Result<(), TransactionError> {
    if !root.join(crate::database::FILE_NAME).is_file() {
        return Ok(());
    }
    let db = crate::database::Database::at(root);
    let connection = db
        .connect()
        .map_err(|e| TransactionError::new(e.to_string()))?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| TransactionError::new(e.to_string()))?;
    if version == 0 {
        return Ok(());
    }
    let mut stmt = connection
        .prepare("SELECT path FROM pending_cleanup ORDER BY path")
        .map_err(|e| TransactionError::new(e.to_string()))?;
    let paths = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| TransactionError::new(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TransactionError::new(e.to_string()))?;
    for path in paths {
        apply_post_commit_deletes(root, std::slice::from_ref(&path)).map_err(|error| {
            TransactionError::recovery_required(error.to_string(), CommitState::Committed)
        })?;
        connection
            .execute("DELETE FROM pending_cleanup WHERE path=?1", [path])
            .map_err(|error| {
                TransactionError::recovery_required(error.to_string(), CommitState::Committed)
            })?;
    }
    Ok(())
}
