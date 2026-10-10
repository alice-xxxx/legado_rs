//! 管理跨多个资源文件的可恢复事务提交与恢复。
//! Recoverable commits for a small set of app-data files.
//
// This module deliberately does not own application locks or ResourceStore.
// Callers must hold the app's single-writer guard for this data root from
// snapshot capture through commit/recovery. The journal makes interrupted
// multi-file commits recoverable at startup; it does not make several files
// appear atomically to concurrent readers, nor promise power-loss durability
// on filesystems that do not honor the file and directory sync operations.

use std::fmt;
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::resources::ResourceRef;

const JOURNAL_FORMAT: &str = "legado-rs-file-transaction";
const JOURNAL_VERSION: u32 = 1;
const TRANSACTIONS_RELATIVE_DIR: &str = "private-data/transactions";
const MAX_OPERATION_BYTES: usize = 96;
const MAX_TARGET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRANSACTION_BYTES: u64 = 128 * 1024 * 1024;

/// Marker for the ResourceStore-wide writer guard required by this helper.
///
/// Implementations must return the canonical app-data root protected by the
/// guard. The application must hold the underlying guard for the complete
/// lifetime of a transaction. A separate per-file lock is not sufficient:
/// ordinary shelf/bookmark/progress writes must be excluded as well.
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
    PrivateBookJson(String),
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

    fn private_book_json(book_id: &str) -> Result<Self, TransactionError> {
        validate_identifier(book_id)?;
        Ok(Self::PrivateBookJson(book_id.to_owned()))
    }

    fn relative_path(&self) -> String {
        match self {
            Self::PublicJson(path) => path.clone(),
            Self::PrivateBookJson(book_id) => format!("private-data/books/{book_id}.json"),
        }
    }

    fn from_journal(domain: TargetDomain, path: &str) -> Result<Self, TransactionError> {
        match domain {
            TargetDomain::Public => {
                let reference = ResourceRef::new(format!("resource://{path}"))
                    .map_err(|_| TransactionError::new("Unsafe public transaction target"))?;
                Self::public_json(reference)
            }
            TargetDomain::PrivateBook => {
                let components = safe_components(path)?;
                if components.len() != 3
                    || components[0] != "private-data"
                    || components[1] != "books"
                    || !components[2].ends_with(".json")
                {
                    return Err(TransactionError::new("Unsafe private transaction target"));
                }
                let book_id = components[2]
                    .strip_suffix(".json")
                    .ok_or_else(|| TransactionError::new("Invalid private book path"))?;
                Self::private_book_json(book_id)
            }
        }
    }

    fn domain(&self) -> TargetDomain {
        match self {
            Self::PublicJson(_) => TargetDomain::Public,
            Self::PrivateBookJson(_) => TargetDomain::PrivateBook,
        }
    }
}

/// A cleanup action limited to one cached chapter or one complete book cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PostCommitDelete {
    path: String,
}

/// A prepared file transaction. The borrowed guard must stay alive until this
/// value is committed or dropped. Dropping it simulates process termination:
/// an already-written prepared journal is intentionally left for recovery.
#[derive(Debug)]
pub(crate) struct FileTransaction<'guard> {
    root: PathBuf,
    stage_dir: PathBuf,
    journal: Journal,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TransactionPhase {
    Prepared,
    Committed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TargetDomain {
    Public,
    PrivateBook,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: String,
    version: u32,
    transaction_id: String,
    operation: String,
    phase: TransactionPhase,
    targets: Vec<JournalTarget>,
    post_commit_deletes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalTarget {
    domain: TargetDomain,
    path: String,
    before: Snapshot,
    after: Snapshot,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    missing: bool,
    blob: Option<String>,
    bytes: u64,
    sha256: Option<String>,
}

impl Snapshot {
    fn missing() -> Self {
        Self {
            missing: true,
            blob: None,
            bytes: 0,
            sha256: None,
        }
    }
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

    fn aborted(message: impl Into<String>) -> Self {
        Self::new(message)
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
#[path = "transactions/journal.rs"]
mod journal;
#[path = "transactions/payload.rs"]
mod payload;
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
use journal::*;
use payload::*;
use recovery::{ensure_no_pending_transaction, restore_snapshot, rollback_prepared};
pub(crate) use recovery::{ensure_no_unrecovered_transaction, recover_all};
use validation::*;
