//! Commit metadata atomically in SQLite and publish immutable file assets.
//! Cache deletion intents live in the same database transaction and retry at
//! startup. Old JSON state and file journals are not imported.
//!
//! Callers own book/chapter coordination. The resource guard coordinates asset
//! publication and cleanup. Independent SQLite updates need no file lock.

use std::fmt;
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use crate::resources::ResourceRef;

const MAX_OPERATION_BYTES: usize = 96;
const MAX_TARGET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRANSACTION_BYTES: u64 = 128 * 1024 * 1024;

/// Marker for the filesystem publication/cleanup guard for this data root.
/// SQLite transactions commit metadata; domain guards protect any application
/// snapshot captured before this helper is called.
pub(crate) trait ResourceWriterGuard {
    fn data_root(&self) -> &Path;
}

/// A typed target for a public JSON document or one private book snapshot.
#[derive(Clone, Debug)]
pub(crate) struct Replacement {
    target: TransactionTarget,
    bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransactionTarget {
    PublicJson(String),
}

impl TransactionTarget {
    fn public_json(reference: ResourceRef) -> Result<Self, TransactionError> {
        if !reference.is_local() || !reference.path().ends_with(".json") {
            return Err(TransactionError::new(
                "Transaction public target must be a local JSON resource",
            ));
        }
        // ResourceRef::new applies the same public-resource allowlist used by
        // ResourceStore. Reconstruct it here so this invariant also holds for
        // references deserialized through other call paths.
        ResourceRef::new(format!("resource://{}", reference.path()))
            .map_err(|_| TransactionError::new("Invalid public JSON transaction path"))?;
        Ok(Self::PublicJson(reference.path().to_owned()))
    }

    fn public_resource(reference: ResourceRef) -> Result<Self, TransactionError> {
        if !reference.is_local() {
            return Err(TransactionError::new("Transaction target must be local"));
        }
        ResourceRef::new(reference.as_str())
            .map_err(|_| TransactionError::new("Invalid transaction resource"))?;
        Ok(Self::PublicJson(reference.path().to_owned()))
    }

    fn relative_path(&self) -> String {
        match self {
            Self::PublicJson(path) => path.clone(),
        }
    }
}

/// A cleanup action limited to one cached chapter or one complete book cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PostCommitDelete {
    path: String,
}

/// Metadata commits atomically in SQLite. Native chapter bodies are published
/// first and unused bodies are harmless if the database transaction rolls back.
#[derive(Debug)]
pub(crate) struct FileTransaction<'guard> {
    root: PathBuf,
    replacements: Vec<Replacement>,
    post_commit_deletes: Vec<String>,
    _writer_guard: PhantomData<&'guard ()>,
}

/// An advisory process-wide lock stored beside, rather than inside, the data
/// root. It does not create the root, so it can be acquired before interrupted
/// backup-restore recovery. Keeping its inode outside the root lets restore
/// rename the active root without releasing the app's single-writer lock.
pub(crate) struct AppDataProcessLock {
    _file: File,
    canonical_root: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitState {
    NotCommitted,
    Committed,
    Indeterminate,
}

#[derive(Debug)]
pub(crate) struct TransactionError {
    message: String,
    commit_state: CommitState,
    recovery_required: bool,
}

impl TransactionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            commit_state: CommitState::NotCommitted,
            recovery_required: false,
        }
    }

    pub(crate) fn recovery_required(message: impl Into<String>, commit_state: CommitState) -> Self {
        Self {
            message: message.into(),
            commit_state,
            recovery_required: true,
        }
    }

    pub(crate) fn commit_state(&self) -> CommitState {
        self.commit_state
    }

    pub(crate) fn recovery_required_flag(&self) -> bool {
        self.recovery_required
    }
}

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TransactionError {}

// 按事务阶段拆分实现，类型和对外入口仍由本模块统一提供。
#[path = "transactions/cleanup.rs"]
mod cleanup;
#[path = "transactions/commit.rs"]
mod commit;
#[path = "transactions/filesystem.rs"]
mod filesystem;
#[path = "transactions/process_lock.rs"]
mod process_lock;
#[path = "transactions/recovery.rs"]
mod recovery;
#[path = "transactions/targets.rs"]
mod targets;
#[path = "transactions/validation.rs"]
mod validation;

use cleanup::*;
use filesystem::*;
pub(crate) use recovery::{ensure_no_unrecovered_transaction, recover_all};
use validation::*;
