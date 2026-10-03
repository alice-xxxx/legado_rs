//! Recoverable commits for a small set of app-data files.
//!
//! This module deliberately does not own application locks or `ResourceStore`.
//! Callers must hold the app's single-writer guard for this data root from
//! snapshot capture through commit/recovery. The journal makes interrupted
//! multi-file commits recoverable at startup; it does not make several files
//! appear atomically to concurrent readers, nor promise power-loss durability
//! on filesystems that do not honor the file and directory sync operations.

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::{Component, Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

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
    bytes: Vec<u8>,
}

impl Replacement {
    pub(crate) fn public_json(
        reference: ResourceRef,
        bytes: Vec<u8>,
    ) -> Result<Self, TransactionError> {
        let target = TransactionTarget::public_json(reference)?;
        validate_public_json_bytes(&bytes)?;
        Ok(Self { target, bytes })
    }

    pub(crate) fn private_book_json(
        book_id: &str,
        bytes: Vec<u8>,
    ) -> Result<Self, TransactionError> {
        let target = TransactionTarget::private_book_json(book_id)?;
        // Private book data may contain source-engine fields named `src` or
        // `*Src`; only require valid JSON here, not public resource semantics.
        validate_json_bytes(&bytes)?;
        Ok(Self { target, bytes })
    }
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
        // `ResourceRef::new` applies the same public-resource allowlist used by
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
            Self::PrivateBookJson(book_id) => {
                format!("private-data/books/{book_id}.json")
            }
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

/// A cleanup action that can only name a cached chapter HTML resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PostCommitDelete {
    path: String,
}

impl PostCommitDelete {
    pub(crate) fn chapter(reference: ResourceRef) -> Result<Self, TransactionError> {
        if !reference.is_local() || !is_chapter_html_path(reference.path()) {
            return Err(TransactionError::new(
                "Post-commit cleanup accepts only chapter HTML resources",
            ));
        }
        ResourceRef::new(format!("resource://{}", reference.path()))
            .map_err(|_| TransactionError::new("Invalid chapter cleanup path"))?;
        Ok(Self {
            path: reference.path().to_owned(),
        })
    }
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
                if replacement.bytes.len() as u64 > MAX_TARGET_BYTES {
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
                        validate_target_json(&target, &bytes)?;
                        add_snapshot_size(&mut total_bytes, bytes.len() as u64)?;
                        write_payload(&stage_dir, "before", index, &bytes)?
                    }
                    None => Snapshot::missing(),
                };
                add_snapshot_size(&mut total_bytes, replacement.bytes.len() as u64)?;
                let after_snapshot = write_payload(&stage_dir, "after", index, &replacement.bytes)?;
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
    pub(crate) fn commit(mut self) -> Result<(), TransactionError> {
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
            )))
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
                )))
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

/// An advisory process-wide lock stored beside, rather than inside, the data
/// root. It does not create the root, so it can be acquired before interrupted
/// backup-restore recovery. Keeping its inode outside the root lets restore
/// rename the active root without releasing the app's single-writer lock.
pub(crate) struct AppDataProcessLock {
    _file: File,
    canonical_root: PathBuf,
}

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
                )))
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
                )))
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
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            commit_state: CommitState::NotCommitted,
            recovery_required: false,
        }
    }

    fn aborted(message: impl Into<String>) -> Self {
        Self::new(message)
    }

    fn recovery_required(message: impl Into<String>, commit_state: CommitState) -> Self {
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

fn rollback_prepared(
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

fn restore_snapshot(
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
            Ok(()) => sync_parent(&absolute_path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(TransactionError::new(format!(
                "Cannot remove transaction target {relative_path}: {error}"
            ))),
        }
    } else {
        let bytes = read_validated_payload(stage_dir, target, snapshot)?;
        atomic_replace(&root, &absolute_path, &bytes)
    }
}

fn apply_post_commit_deletes(root: &Path, journal: &Journal) -> Result<(), TransactionError> {
    for relative_path in &journal.post_commit_deletes {
        let reference = ResourceRef::new(format!("resource://{relative_path}"))
            .map_err(|_| TransactionError::new("Unsafe post-commit cleanup reference"))?;
        if !is_chapter_html_path(reference.path()) {
            return Err(TransactionError::new(
                "Post-commit cleanup path is not a chapter HTML resource",
            ));
        }
        let path = root.join(reference.path());
        reject_symlink_components(root, &path, true)?;
        match fs::remove_file(&path) {
            Ok(()) => sync_parent(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot delete cached chapter {}: {error}",
                    reference.path()
                )))
            }
        }
    }
    Ok(())
}

fn finish_committed_transaction(root: &Path, stage_dir: &Path) -> Result<(), TransactionError> {
    finish_transaction_directory(root, stage_dir)
}

fn finish_rolled_back_transaction(root: &Path, stage_dir: &Path) -> Result<(), TransactionError> {
    finish_transaction_directory(root, stage_dir)
}

fn finish_transaction_directory(root: &Path, stage_dir: &Path) -> Result<(), TransactionError> {
    let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
    validate_existing_path_no_symlinks(root, stage_dir)?;
    let journal_path = stage_dir.join("journal.json");
    match fs::remove_file(&journal_path) {
        Ok(()) => sync_directory(stage_dir)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot remove completed transaction journal: {error}"
            )))
        }
    }
    fs::remove_dir_all(stage_dir).map_err(|error| {
        TransactionError::new(format!(
            "Cannot remove completed transaction journal: {error}"
        ))
    })?;
    sync_directory(&transactions_dir)
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
            )))
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
                return Err(TransactionError::new(
                    "A previous file transaction needs recovery before resource writes",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect previous transaction journal: {error}"
                )))
            }
        }
    }
    Ok(())
}

fn ensure_no_pending_transaction(root: &Path) -> Result<(), TransactionError> {
    let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
    match fs::symlink_metadata(&transactions_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot inspect transaction staging directory: {error}"
            )))
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
                    return Err(TransactionError::new(
                        "A previous file transaction needs recovery before writing",
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

fn write_payload(
    stage_dir: &Path,
    area: &str,
    index: usize,
    bytes: &[u8],
) -> Result<Snapshot, TransactionError> {
    let area_dir = stage_dir.join(area);
    fs::create_dir_all(&area_dir).map_err(|error| {
        TransactionError::new(format!(
            "Cannot create transaction payload directory: {error}"
        ))
    })?;
    let filename = format!("{index:06}.bin");
    let blob = format!("{area}/{filename}");
    let path = area_dir.join(filename);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| {
            TransactionError::new(format!("Cannot stage transaction bytes: {error}"))
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            TransactionError::new(format!("Cannot sync transaction bytes: {error}"))
        })?;
    sync_directory(&area_dir)?;
    sync_directory(stage_dir)?;
    Ok(Snapshot {
        missing: false,
        blob: Some(blob),
        bytes: bytes.len() as u64,
        sha256: Some(hex_digest(bytes)),
    })
}

fn read_payload(stage_dir: &Path, snapshot: &Snapshot) -> Result<Vec<u8>, TransactionError> {
    if snapshot.missing {
        return Err(TransactionError::new("Missing snapshot has no payload"));
    }
    let blob = snapshot
        .blob
        .as_deref()
        .ok_or_else(|| TransactionError::new("Snapshot payload path is missing"))?;
    let components = safe_components(blob)?;
    if components.len() != 2
        || !matches!(components[0], "before" | "after")
        || !components[1].ends_with(".bin")
    {
        return Err(TransactionError::new("Invalid transaction payload path"));
    }
    let path = stage_dir.join(blob);
    reject_symlink_components(stage_dir, &path, false)?;
    let metadata = fs::metadata(&path).map_err(|error| {
        TransactionError::new(format!("Cannot inspect transaction payload: {error}"))
    })?;
    if !metadata.is_file() || metadata.len() != snapshot.bytes || metadata.len() > MAX_TARGET_BYTES
    {
        return Err(TransactionError::new("Transaction payload size is invalid"));
    }
    let mut file = File::open(&path).map_err(|error| {
        TransactionError::new(format!("Cannot open transaction payload: {error}"))
    })?;
    let mut bytes = Vec::with_capacity(snapshot.bytes as usize);
    file.read_to_end(&mut bytes).map_err(|error| {
        TransactionError::new(format!("Cannot read transaction payload: {error}"))
    })?;
    if bytes.len() as u64 != snapshot.bytes
        || snapshot.sha256.as_deref() != Some(hex_digest(&bytes).as_str())
    {
        return Err(TransactionError::new(
            "Transaction payload checksum does not match its journal",
        ));
    }
    Ok(bytes)
}

fn read_snapshot_bytes(root: &Path, path: &Path) -> Result<Option<Vec<u8>>, TransactionError> {
    reject_symlink_components(root, path, true)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot inspect transaction target: {error}"
            )))
        }
    };
    if !metadata.is_file() || metadata.len() > MAX_TARGET_BYTES {
        return Err(TransactionError::new(
            "Transaction target must be a regular file within the size limit",
        ));
    }
    fs::read(path).map(Some).map_err(|error| {
        TransactionError::new(format!("Cannot snapshot transaction target: {error}"))
    })
}

fn write_journal(stage_dir: &Path, journal: &Journal) -> Result<(), TransactionError> {
    validate_journal(journal, &journal.transaction_id)?;
    let bytes = serde_json::to_vec_pretty(journal).map_err(|error| {
        TransactionError::new(format!("Cannot encode transaction journal: {error}"))
    })?;
    let journal_path = stage_dir.join("journal.json");
    atomic_replace_path(&journal_path, &bytes)?;
    sync_directory(stage_dir)
}

fn read_journal(stage_dir: &Path, transaction_id: &str) -> Result<Journal, TransactionError> {
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

fn validate_journal(journal: &Journal, expected_id: &str) -> Result<(), TransactionError> {
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
        if entry.after.missing {
            return Err(TransactionError::new(
                "Transaction after-image must contain exact bytes",
            ));
        }
    }
    let mut deletes = HashSet::new();
    for path in &journal.post_commit_deletes {
        let reference = ResourceRef::new(format!("resource://{path}"))
            .map_err(|_| TransactionError::new("Unsafe journal cleanup path"))?;
        if !is_chapter_html_path(reference.path()) || !deletes.insert(path) {
            return Err(TransactionError::new(
                "Journal cleanup path is invalid or duplicated",
            ));
        }
    }
    Ok(())
}

fn validate_snapshot(
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

fn validate_unique_entries(
    replacements: &[Replacement],
    deletes: &[PostCommitDelete],
) -> Result<(), TransactionError> {
    let mut targets = HashSet::new();
    for replacement in replacements {
        let path = replacement.target.relative_path();
        if !targets.insert(path) {
            return Err(TransactionError::new("Duplicate transaction target"));
        }
    }
    let mut delete_paths = HashSet::new();
    for delete in deletes {
        if !delete_paths.insert(&delete.path) {
            return Err(TransactionError::new("Duplicate post-commit cleanup path"));
        }
    }
    Ok(())
}

fn validate_json_bytes(bytes: &[u8]) -> Result<(), TransactionError> {
    validate_json_size(bytes)?;
    serde_json::from_slice::<serde_json::Value>(bytes)
        .map(|_| ())
        .map_err(|error| {
            TransactionError::new(format!("Transaction target is not valid JSON: {error}"))
        })
}

fn validate_public_json_bytes(bytes: &[u8]) -> Result<(), TransactionError> {
    validate_json_size(bytes)?;
    crate::resources::validate_persistable_json_bytes(bytes)
        .map_err(|error| TransactionError::new(format!("Invalid public transaction JSON: {error}")))
}

fn validate_json_size(bytes: &[u8]) -> Result<(), TransactionError> {
    if bytes.len() as u64 > MAX_TARGET_BYTES {
        return Err(TransactionError::new(
            "A transaction JSON document exceeds the 64 MiB safety limit",
        ));
    }
    Ok(())
}

fn validate_target_json(target: &TransactionTarget, bytes: &[u8]) -> Result<(), TransactionError> {
    match target {
        TransactionTarget::PublicJson(_) => validate_public_json_bytes(bytes),
        TransactionTarget::PrivateBookJson(_) => validate_json_bytes(bytes),
    }
}

fn read_validated_payload(
    stage_dir: &Path,
    target: &TransactionTarget,
    snapshot: &Snapshot,
) -> Result<Vec<u8>, TransactionError> {
    let bytes = read_payload(stage_dir, snapshot)?;
    validate_target_json(target, &bytes)?;
    Ok(bytes)
}

fn validate_operation(operation: &str) -> Result<(), TransactionError> {
    if operation.is_empty()
        || operation.len() > MAX_OPERATION_BYTES
        || !operation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(TransactionError::new("Invalid transaction operation name"));
    }
    Ok(())
}

fn validate_transaction_id(transaction_id: &str) -> Result<(), TransactionError> {
    let Some(uuid) = transaction_id.strip_prefix("txn-") else {
        return Err(TransactionError::new("Invalid transaction identifier"));
    };
    if uuid.len() != 32 || !uuid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TransactionError::new("Invalid transaction identifier"));
    }
    Ok(())
}

fn validate_identifier(id: &str) -> Result<(), TransactionError> {
    if id.is_empty()
        || id.len() > 255
        || id == "."
        || id == ".."
        || id.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err(TransactionError::new("Invalid transaction book identifier"));
    }
    Ok(())
}

fn is_chapter_html_path(path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    components.len() == 4
        && components[0] == "books"
        && valid_identifier(components[1])
        && components[2] == "chapters"
        && components[3]
            .strip_suffix(".html")
            .is_some_and(valid_identifier)
}

fn valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id != "."
        && id != ".."
        && !id.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

fn safe_components(path: &str) -> Result<Vec<&str>, TransactionError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(TransactionError::new("Unsafe transaction path"));
    }
    let components: Vec<_> = path.split('/').collect();
    if components.iter().any(|component| {
        component.is_empty()
            || *component == "."
            || *component == ".."
            || component.ends_with(['.', ' '])
            || is_windows_reserved_component(component)
    }) {
        return Err(TransactionError::new("Unsafe transaction path component"));
    }
    let parsed = Path::new(path);
    if parsed
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(TransactionError::new("Transaction path must be relative"));
    }
    Ok(components)
}

fn is_windows_reserved_component(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit()
        && stem.as_bytes()[3] != b'0')
}

fn add_snapshot_size(total: &mut u64, next: u64) -> Result<(), TransactionError> {
    *total = total
        .checked_add(next)
        .ok_or_else(|| TransactionError::new("Transaction snapshot size overflow"))?;
    if *total > MAX_TRANSACTION_BYTES {
        return Err(TransactionError::new(
            "Transaction snapshots exceed the 128 MiB safety limit",
        ));
    }
    Ok(())
}

fn canonical_root(root: &Path) -> Result<PathBuf, TransactionError> {
    fs::create_dir_all(root)
        .map_err(|error| TransactionError::new(format!("Cannot create app-data root: {error}")))?;
    let canonical = fs::canonicalize(root)
        .map_err(|error| TransactionError::new(format!("Cannot resolve app-data root: {error}")))?;
    let metadata = fs::metadata(&canonical)
        .map_err(|error| TransactionError::new(format!("Cannot inspect app-data root: {error}")))?;
    if !metadata.is_dir() {
        return Err(TransactionError::new("App-data root is not a directory"));
    }
    Ok(canonical)
}

fn validate_guard_root(
    guard: &dyn ResourceWriterGuard,
    expected_root: &Path,
) -> Result<(), TransactionError> {
    let guard_root = fs::canonicalize(guard.data_root()).map_err(|error| {
        TransactionError::new(format!("Cannot resolve writer-guard root: {error}"))
    })?;
    if guard_root != expected_root {
        return Err(TransactionError::new(
            "Resource writer guard belongs to a different app-data root",
        ));
    }
    Ok(())
}

fn validate_target_path(
    root: &Path,
    target: &TransactionTarget,
    allow_missing: bool,
) -> Result<(), TransactionError> {
    let relative = target.relative_path();
    let components = safe_components(&relative)?;
    if components.is_empty() {
        return Err(TransactionError::new("Transaction target path is empty"));
    }
    reject_symlink_components(root, &root.join(relative), allow_missing)
}

fn ensure_safe_directory(root: &Path, directory: &Path) -> Result<(), TransactionError> {
    reject_symlink_components(root, directory, true)?;
    fs::create_dir_all(directory).map_err(|error| {
        TransactionError::new(format!("Cannot create transaction directory: {error}"))
    })?;
    reject_symlink_components(root, directory, false)?;
    sync_parent_chain(root, directory)
}

fn validate_existing_path_no_symlinks(root: &Path, path: &Path) -> Result<(), TransactionError> {
    reject_symlink_components(root, path, false)
}

fn reject_symlink_components(
    root: &Path,
    path: &Path,
    allow_missing_tail: bool,
) -> Result<(), TransactionError> {
    if !path.starts_with(root) {
        return Err(TransactionError::new(
            "Transaction path escaped app-data root",
        ));
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| TransactionError::new("Transaction path escaped app-data root"))?;
    let components: Vec<_> = relative.components().collect();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(TransactionError::new(
                "Transaction path has an invalid component",
            ));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(TransactionError::new(
                        "Transaction path contains a symbolic link",
                    ));
                }
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err(TransactionError::new(
                        "Transaction path parent is not a directory",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing_tail => {
                return Ok(())
            }
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect transaction path: {error}"
                )))
            }
        }
    }
    Ok(())
}

fn atomic_replace(root: &Path, path: &Path, bytes: &[u8]) -> Result<(), TransactionError> {
    reject_symlink_components(root, path, true)?;
    let parent = path
        .parent()
        .ok_or_else(|| TransactionError::new("Transaction target has no parent"))?;
    ensure_parent_directories(root, parent)?;
    atomic_replace_path(path, bytes)?;
    sync_parent_chain(root, parent)
}

fn ensure_parent_directories(root: &Path, directory: &Path) -> Result<(), TransactionError> {
    if !directory.starts_with(root) {
        return Err(TransactionError::new(
            "Transaction target parent escaped app-data root",
        ));
    }
    let relative = directory
        .strip_prefix(root)
        .map_err(|_| TransactionError::new("Transaction target parent escaped app-data root"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(TransactionError::new("Invalid transaction target parent"));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(TransactionError::new(
                        "Transaction target parent is not a regular directory",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|create_error| {
                    TransactionError::new(format!(
                        "Cannot create transaction target parent: {create_error}"
                    ))
                })?;
                let parent = current.parent().ok_or_else(|| {
                    TransactionError::new("Transaction target parent has no parent")
                })?;
                sync_directory(parent)?;
            }
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect transaction target parent: {error}"
                )))
            }
        }
    }
    reject_symlink_components(root, directory, false)
}

fn sync_parent_chain(root: &Path, directory: &Path) -> Result<(), TransactionError> {
    if !directory.starts_with(root) {
        return Err(TransactionError::new(
            "Directory sync path escaped app-data root",
        ));
    }
    let mut current = directory.to_path_buf();
    loop {
        sync_directory(&current)?;
        if current == root {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| TransactionError::new("Directory sync path has no app-data root"))?
            .to_path_buf();
    }
    Ok(())
}

fn atomic_replace_path(path: &Path, bytes: &[u8]) -> Result<(), TransactionError> {
    let parent = path
        .parent()
        .ok_or_else(|| TransactionError::new("Transaction target has no parent"))?;
    fs::create_dir_all(parent)
        .map_err(|error| TransactionError::new(format!("Cannot create target parent: {error}")))?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .map_err(|error| {
            TransactionError::new(format!("Atomic transaction write failed: {error}"))
        })?;
    sync_directory(parent)
}

fn sync_parent(path: &Path) -> Result<(), TransactionError> {
    let parent = path
        .parent()
        .ok_or_else(|| TransactionError::new("Transaction path has no parent"))?;
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), TransactionError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| TransactionError::new(format!("Cannot sync directory metadata: {error}")))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), TransactionError> {
    // Rust's portable std API does not expose directory handles with a
    // cross-platform sync contract. Files are still synced before replacement;
    // this is a weaker metadata durability guarantee on these targets.
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROCESS_LOCK_CHILD_ROOT_ENV: &str = "LEGADO_RS_PROCESS_LOCK_CHILD_ROOT";

    struct TestGuard(PathBuf);

    impl ResourceWriterGuard for TestGuard {
        fn data_root(&self) -> &Path {
            &self.0
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf, TestGuard) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app-data");
        fs::create_dir_all(&root).unwrap();
        let canonical = fs::canonicalize(&root).unwrap();
        let guard = TestGuard(canonical.clone());
        (dir, canonical, guard)
    }

    fn public_json(path: &str, value: serde_json::Value) -> Replacement {
        Replacement::public_json(
            ResourceRef::new(format!("resource://{path}")).unwrap(),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap()
    }

    fn book_target(book_id: &str, value: serde_json::Value) -> Replacement {
        Replacement::private_book_json(book_id, serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn book_path(root: &Path, book_id: &str) -> PathBuf {
        root.join("books").join(book_id).join("book.json")
    }

    fn transaction<'a>(
        root: &Path,
        guard: &'a TestGuard,
        replacements: Vec<Replacement>,
        deletes: Vec<PostCommitDelete>,
    ) -> FileTransaction<'a> {
        FileTransaction::prepare(root, "test-transaction", replacements, deletes, guard).unwrap()
    }

    fn write_old_state(root: &Path) {
        fs::create_dir_all(root.join("books/fixture-book")).unwrap();
        fs::create_dir_all(root.join("progress")).unwrap();
        fs::create_dir_all(root.join("private-data/books")).unwrap();
        fs::write(book_path(root, "fixture-book"), br#"{"generation":"old"}"#).unwrap();
        fs::write(
            root.join("progress/fixture-book.json"),
            br#"{"generation":"old"}"#,
        )
        .unwrap();
        fs::write(
            root.join("private-data/books/fixture-book.json"),
            br#"{"generation":"old"}"#,
        )
        .unwrap();
    }

    fn read_generation(path: &Path) -> String {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        value["generation"].as_str().unwrap().to_owned()
    }

    #[test]
    fn prepared_transaction_rolls_back_partial_writes_after_restart() {
        for completed_writes in 0..=3 {
            let (_dir, root, guard) = setup();
            write_old_state(&root);
            let transaction = transaction(
                &root,
                &guard,
                vec![
                    public_json("books/fixture-book/book.json", json!({"generation":"new"})),
                    public_json("progress/fixture-book.json", json!({"generation":"new"})),
                    book_target("fixture-book", json!({"generation":"new"})),
                ],
                vec![],
            );

            for index in 0..completed_writes {
                transaction.apply_target(index).unwrap();
            }
            drop(transaction); // Abrupt stop before the durable commit marker.

            recover_all(&root).unwrap();
            assert_eq!(read_generation(&book_path(&root, "fixture-book")), "old");
            assert_eq!(
                read_generation(&root.join("progress/fixture-book.json")),
                "old"
            );
            assert_eq!(
                read_generation(&root.join("private-data/books/fixture-book.json")),
                "old"
            );
            assert!(fs::read_dir(root.join(TRANSACTIONS_RELATIVE_DIR))
                .unwrap()
                .next()
                .is_none());
        }
    }

    #[test]
    fn committed_transaction_redoes_all_targets_and_repeats_cleanup() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let old_chapter =
            ResourceRef::new("resource://books/fixture-book/chapters/old.html").unwrap();
        let chapter_path = root.join(old_chapter.path());
        let second_chapter = root.join("books/fixture-book/chapters/older.html");
        fs::create_dir_all(chapter_path.parent().unwrap()).unwrap();
        fs::write(&chapter_path, "old cache").unwrap();
        fs::write(&second_chapter, "older cache").unwrap();
        let transaction = transaction(
            &root,
            &guard,
            vec![
                public_json("books/fixture-book/book.json", json!({"generation":"new"})),
                public_json("progress/fixture-book.json", json!({"generation":"new"})),
                book_target("fixture-book", json!({"generation":"new"})),
            ],
            vec![
                PostCommitDelete::chapter(old_chapter).unwrap(),
                PostCommitDelete::chapter(
                    ResourceRef::new("resource://books/fixture-book/chapters/older.html").unwrap(),
                )
                .unwrap(),
            ],
        );
        for index in 0..transaction.journal.targets.len() {
            transaction.apply_target(index).unwrap();
        }
        let mut committed = transaction.journal.clone();
        committed.phase = TransactionPhase::Committed;
        write_journal(&transaction.stage_dir, &committed).unwrap();
        fs::remove_file(&chapter_path).unwrap();
        drop(transaction); // Crash after partial post-commit cleanup.

        recover_all(&root).unwrap();
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "new");
        assert_eq!(
            read_generation(&root.join("progress/fixture-book.json")),
            "new"
        );
        assert!(!chapter_path.exists());
        assert!(!second_chapter.exists());
        assert!(fs::read_dir(root.join(TRANSACTIONS_RELATIVE_DIR))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn missing_before_image_removes_new_file_on_rollback() {
        let (_dir, root, guard) = setup();
        let transaction = transaction(
            &root,
            &guard,
            vec![public_json("shelf.json", json!({"generation":"new"}))],
            vec![],
        );
        transaction.apply_target(0).unwrap();
        drop(transaction);
        recover_all(&root).unwrap();
        assert!(!root.join("shelf.json").exists());
    }

    #[test]
    fn unjournaled_stage_is_swept_but_pending_journal_blocks_new_writes() {
        let (_dir, root, guard) = setup();
        let transactions_dir = root.join(TRANSACTIONS_RELATIVE_DIR);
        fs::create_dir_all(&transactions_dir).unwrap();
        let orphan = transactions_dir.join(format!("txn-{}", Uuid::new_v4().simple()));
        fs::create_dir(&orphan).unwrap();
        fs::write(orphan.join("after.bin"), b"unused").unwrap();
        recover_all(&root).unwrap();
        assert!(!orphan.exists());

        let transaction = transaction(
            &root,
            &guard,
            vec![public_json("shelf.json", json!({"generation":"new"}))],
            vec![],
        );
        let error = FileTransaction::prepare(
            &root,
            "another-write",
            vec![public_json("settings.json", json!({}))],
            vec![],
            &guard,
        )
        .unwrap_err();
        assert!(error.to_string().contains("needs recovery"));
        drop(transaction);
    }

    #[test]
    fn stage_left_after_journal_removal_is_swept_on_startup() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let transaction = transaction(
            &root,
            &guard,
            vec![public_json(
                "books/fixture-book/book.json",
                json!({"generation":"new"}),
            )],
            vec![],
        );
        transaction.apply_target(0).unwrap();
        let mut committed = transaction.journal.clone();
        committed.phase = TransactionPhase::Committed;
        write_journal(&transaction.stage_dir, &committed).unwrap();
        let stage_dir = transaction.stage_dir.clone();
        fs::remove_file(stage_dir.join("journal.json")).unwrap();
        sync_directory(&stage_dir).unwrap();
        drop(transaction);

        recover_all(&root).unwrap();
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "new");
        assert!(!stage_dir.exists());
    }

    #[test]
    fn checksum_mismatch_fails_closed_and_preserves_journal() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let transaction = transaction(
            &root,
            &guard,
            vec![
                public_json("books/fixture-book/book.json", json!({"generation":"new"})),
                public_json("progress/fixture-book.json", json!({"generation":"new"})),
            ],
            vec![],
        );
        for index in 0..transaction.journal.targets.len() {
            transaction.apply_target(index).unwrap();
        }
        let before_blob = transaction.stage_dir.join("before/000001.bin");
        fs::write(&before_blob, b"tampered").unwrap();
        let stage = transaction.stage_dir.clone();
        drop(transaction);

        let error = recover_all(&root).unwrap_err();
        assert!(
            error.to_string().contains("size is invalid") || error.to_string().contains("checksum")
        );
        assert!(stage.join("journal.json").is_file());
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "new");
        assert_eq!(
            read_generation(&root.join("progress/fixture-book.json")),
            "new",
            "recovery must verify every before-image before restoring any target"
        );
    }

    #[test]
    fn committed_after_checksum_mismatch_does_not_partially_redo() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let transaction = transaction(
            &root,
            &guard,
            vec![
                public_json("books/fixture-book/book.json", json!({"generation":"new"})),
                public_json("progress/fixture-book.json", json!({"generation":"new"})),
            ],
            vec![],
        );
        for index in 0..transaction.journal.targets.len() {
            transaction.apply_target(index).unwrap();
        }
        let mut committed = transaction.journal.clone();
        committed.phase = TransactionPhase::Committed;
        write_journal(&transaction.stage_dir, &committed).unwrap();
        let after_blob = transaction.stage_dir.join("after/000001.bin");
        fs::write(&after_blob, b"tampered").unwrap();
        let stage = transaction.stage_dir.clone();
        drop(transaction);

        let error = recover_all(&root).unwrap_err();
        assert!(error.to_string().contains("size is invalid"));
        assert!(stage.join("journal.json").is_file());
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "new");
        assert_eq!(
            read_generation(&root.join("progress/fixture-book.json")),
            "new",
            "recovery must verify every after-image before replaying any target"
        );
    }

    #[test]
    fn recovery_rejects_rechecksummed_runtime_urls_in_public_snapshots() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let transaction = transaction(
            &root,
            &guard,
            vec![public_json(
                "books/fixture-book/book.json",
                json!({"generation":"new"}),
            )],
            vec![],
        );
        let runtime_url =
            "http://127.0.0.1:34123/r/0123456789abcdef0123456789abcdef/books/a/cover.png";
        let bytes = format!(r#"{{"coverSrc":"{runtime_url}"}}"#).into_bytes();
        fs::write(transaction.stage_dir.join("after/000000.bin"), &bytes).unwrap();
        let mut committed = transaction.journal.clone();
        committed.phase = TransactionPhase::Committed;
        committed.targets[0].after = Snapshot {
            missing: false,
            blob: Some("after/000000.bin".to_owned()),
            bytes: bytes.len() as u64,
            sha256: Some(hex_digest(&bytes)),
        };
        write_journal(&transaction.stage_dir, &committed).unwrap();
        let stage = transaction.stage_dir.clone();
        drop(transaction);

        let error = recover_all(&root).unwrap_err();
        assert!(error.to_string().contains("cannot be persisted"));
        assert!(stage.join("journal.json").is_file());
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "old");
    }

    #[test]
    fn commit_write_failure_rolls_back_and_startup_finishes_recovery() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let transaction = FileTransaction::prepare(
            &root,
            "injected-write-failure",
            vec![
                public_json("books/fixture-book/book.json", json!({"generation":"new"})),
                public_json("books/late/book.json", json!({"generation":"new"})),
            ],
            vec![],
            &guard,
        )
        .unwrap();
        fs::write(root.join("books/late"), b"blocks target parent").unwrap();

        let error = transaction.commit().unwrap_err();
        assert_eq!(error.commit_state(), CommitState::NotCommitted);
        assert!(error.recovery_required_flag());
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "old");

        fs::remove_file(root.join("books/late")).unwrap();
        recover_all(&root).unwrap();
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "old");
        assert!(!root.join("books/late/book.json").exists());
        assert!(fs::read_dir(root.join(TRANSACTIONS_RELATIVE_DIR))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn damaged_after_image_aborts_before_writes_and_cleans_prepared_journal() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let transaction = FileTransaction::prepare(
            &root,
            "damaged-after-image",
            vec![public_json(
                "books/fixture-book/book.json",
                json!({"generation":"new"}),
            )],
            vec![],
            &guard,
        )
        .unwrap();
        let stage_dir = transaction.stage_dir.clone();
        fs::write(stage_dir.join("after/000000.bin"), b"tampered").unwrap();

        let error = transaction.commit().unwrap_err();
        assert_eq!(error.commit_state(), CommitState::NotCommitted);
        assert!(!error.recovery_required_flag());
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "old");
        assert!(!stage_dir.exists());
    }

    #[test]
    fn committed_cleanup_failure_keeps_commit_and_recovery_retries() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let chapter = root.join("books/fixture-book/chapters/blocked.html");
        fs::create_dir_all(&chapter).unwrap();
        let transaction = FileTransaction::prepare(
            &root,
            "injected-cleanup-failure",
            vec![public_json(
                "books/fixture-book/book.json",
                json!({"generation":"new"}),
            )],
            vec![PostCommitDelete::chapter(
                ResourceRef::new("resource://books/fixture-book/chapters/blocked.html").unwrap(),
            )
            .unwrap()],
            &guard,
        )
        .unwrap();

        let error = transaction.commit().unwrap_err();
        assert_eq!(error.commit_state(), CommitState::Committed);
        assert!(error.recovery_required_flag());
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "new");

        fs::remove_dir(&chapter).unwrap();
        recover_all(&root).unwrap();
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "new");
        assert!(fs::read_dir(root.join(TRANSACTIONS_RELATIVE_DIR))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn typed_paths_reject_traversal_external_targets_and_symlink_parents() {
        assert!(ResourceRef::new("resource://books/../shelf.json").is_err());
        assert!(Replacement::public_json(
            ResourceRef::new("https://example.test/book.json").unwrap(),
            br#"{}"#.to_vec()
        )
        .is_err());
        assert!(Replacement::private_book_json("../escape", br#"{}"#.to_vec()).is_err());

        #[cfg(unix)]
        {
            let (_dir, root, guard) = setup();
            let outside = root.parent().unwrap().join("outside");
            fs::create_dir_all(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, root.join("books")).unwrap();
            let error = FileTransaction::prepare(
                &root,
                "symlink-test",
                vec![public_json("books/fixture-book/book.json", json!({}))],
                vec![],
                &guard,
            )
            .unwrap_err();
            assert!(error.to_string().contains("symbolic link"));
        }
    }

    #[test]
    fn transaction_json_rejects_runtime_capability_urls() {
        let runtime_url =
            "http://127.0.0.1:34123/r/0123456789abcdef0123456789abcdef/books/a/assets/cover.png";
        let error = Replacement::public_json(
            ResourceRef::new("resource://shelf.json").unwrap(),
            format!(r#"{{"coverSrc":"{runtime_url}"}}"#).into_bytes(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot be persisted"));
        assert!(Replacement::public_json(
            ResourceRef::new("resource://shelf.json").unwrap(),
            br#"{"coverSrc":"https://cdn.example/cover.png"}"#.to_vec(),
        )
        .is_ok());
        let source_private_json = br#"{"src":"engine-rule://source/internal"}"#;
        assert!(Replacement::private_book_json("book-a", source_private_json.to_vec()).is_ok());
        assert!(Replacement::public_json(
            ResourceRef::new("resource://books/book-a/book.json").unwrap(),
            source_private_json.to_vec(),
        )
        .is_err());
    }

    #[test]
    fn writer_guard_must_match_the_canonical_data_root() {
        let (_dir, root, _guard) = setup();
        let other = root.parent().unwrap().join("other");
        fs::create_dir_all(&other).unwrap();
        let wrong_guard = TestGuard(other);
        let error = FileTransaction::prepare(
            &root,
            "wrong-root",
            vec![public_json("shelf.json", json!({}))],
            vec![],
            &wrong_guard,
        )
        .unwrap_err();
        assert!(error.to_string().contains("different app-data root"));
    }

    #[test]
    fn runtime_abort_restores_before_images_and_reports_not_committed() {
        let (_dir, root, guard) = setup();
        write_old_state(&root);
        let transaction = FileTransaction::prepare(
            &root,
            "runtime-rollback",
            vec![
                public_json("books/fixture-book/book.json", json!({"generation":"new"})),
                public_json("progress/fixture-book.json", json!({"generation":"new"})),
            ],
            vec![],
            &guard,
        )
        .unwrap();
        transaction.apply_target(0).unwrap();
        let error = transaction
            .abort_prepared(TransactionError::new("injected pre-commit failure"))
            .unwrap_err();
        assert_eq!(error.commit_state(), CommitState::NotCommitted);
        assert!(!error.recovery_required_flag());
        assert_eq!(read_generation(&book_path(&root, "fixture-book")), "old");
        assert_eq!(
            read_generation(&root.join("progress/fixture-book.json")),
            "old"
        );
    }

    #[test]
    fn process_lock_is_outside_root_and_exclusive_for_live_processes() {
        let (_dir, root, _guard) = setup();
        let first = AppDataProcessLock::acquire(&root).unwrap();
        assert_eq!(first.data_root(), root);
        let second = AppDataProcessLock::acquire(&root);
        assert!(second.is_err());

        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("resource_transactions::tests::process_lock_conflict_child")
            .arg("--nocapture")
            .env(PROCESS_LOCK_CHILD_ROOT_ENV, &root)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "child process lock assertion failed: {}",
            String::from_utf8_lossy(&child.stderr)
        );

        // Backup recovery can replace the active root directory. The sibling
        // lock remains on the stable path and still excludes another opener.
        let displaced = root.with_file_name("app-data-displaced");
        fs::rename(&root, &displaced).unwrap();
        assert!(AppDataProcessLock::acquire(&root).is_err());
        assert!(
            !root.exists(),
            "acquiring the startup lock must not recreate a root before restore recovery"
        );
        fs::create_dir(&root).unwrap();
        assert!(AppDataProcessLock::acquire(&root).is_err());
        drop(first);
        assert!(AppDataProcessLock::acquire(&root).is_ok());
        assert!(root.parent().unwrap().read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".legado-app-data-")
        }));
    }

    #[test]
    fn process_lock_conflict_child() {
        let Ok(root) = std::env::var(PROCESS_LOCK_CHILD_ROOT_ENV) else {
            return;
        };
        let Err(error) = AppDataProcessLock::acquire(Path::new(&root)) else {
            panic!("a second process acquired the same app-data lock");
        };
        assert!(error.to_string().contains("already in use"));
    }
}
