//! 校验归档内容，安全恢复资源并处理中断恢复。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path};

use atomicwrites::{AllowOverwrite, AtomicFile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::models::CURRENT_SCHEMA_VERSION;
use crate::resource_transactions::{CommitState, TransactionError};
use crate::resources::ResourceStore;

use super::create::{
    collect_snapshot_files, is_allowed_snapshot_path, validate_archive_path, validate_snapshot_json,
};
use super::{
    BACKUP_FORMAT, BACKUP_VERSION, BackupManifest, DEVICE_SOURCE_CACHES, MANIFEST_PATH,
    MAX_ARCHIVE_BYTES, MAX_ENTRIES, MAX_FILE_BYTES, MAX_MANIFEST_BYTES, MAX_TOTAL_BYTES,
    hex_digest,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RestoreJournal {
    stage_name: String,
    displaced_name: String,
}

/// Validate and restore a complete snapshot. All ZIP entries are checked and
/// extracted into a sibling staging directory before the current data root is
/// moved, so malformed or incomplete archives leave the current data intact.
/// Callers must hold the application's exclusive operation guard for the
/// duration of restore.
pub(crate) async fn restore_backup(
    store: &ResourceStore,
    archive_path: impl AsRef<Path>,
) -> Result<(), TransactionError> {
    let root = store.root().to_path_buf();
    let archive_path = archive_path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || restore_backup_sync(&root, &archive_path))
        .await
        .map_err(|error| {
            TransactionError::recovery_required(
                format!("Restore worker stopped before reporting its result: {error}"),
                CommitState::Indeterminate,
            )
        })?
}

/// Finish or roll back a restore interrupted between directory renames. Call
/// this before opening `ResourceStore`, which otherwise initializes a missing
/// root as a new empty library.
pub fn recover_interrupted_restore(root: impl AsRef<Path>) -> Result<(), String> {
    let requested_root = root.as_ref();
    let requested_parent = requested_root
        .parent()
        .ok_or_else(|| "App data root has no parent folder".to_owned())?;
    let parent = match fs::canonicalize(requested_parent) {
        Ok(parent) => parent,
        // A fresh installation may not have created even Library/Application
        // Support yet. A restore journal cannot exist in a nonexistent parent,
        // so recovery is a no-op and normal startup will create the app root.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Cannot resolve app data parent: {error}")),
    };
    let root_name = requested_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "App data root has an invalid folder name".to_owned())?;
    let root = parent.join(root_name);
    let journal_path = parent.join(format!(".legado-restore-{root_name}.json"));
    let journal_bytes = match fs::read(&journal_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Cannot read restore recovery record: {error}")),
    };
    let journal: RestoreJournal = serde_json::from_slice(&journal_bytes)
        .map_err(|error| format!("Restore recovery record is invalid: {error}"))?;
    let stage_name = validate_recovery_name(&journal.stage_name, "legado-restore-")?;
    let displaced_name = validate_recovery_name(&journal.displaced_name, ".legado-previous-")?;
    let stage = parent.join(stage_name);
    let displaced = parent.join(displaced_name);

    let root_exists = recovery_path_exists(&root)?;
    let stage_exists = recovery_path_exists(&stage)?;
    let displaced_exists = recovery_path_exists(&displaced)?;
    if root_exists {
        ensure_recovery_directory(&root)?;
        if displaced_exists {
            ensure_recovery_directory(&displaced)?;
            // The staged tree became active; this is the crash window after
            // the second rename. Keep the new tree and finish old-tree cleanup.
            let _ = fs::remove_dir_all(&displaced);
        }
        if stage_exists {
            ensure_recovery_directory(&stage)?;
            let _ = fs::remove_dir_all(&stage);
        }
        remove_recovery_journal(&journal_path)?;
        return Ok(());
    }

    if stage_exists {
        ensure_recovery_directory(&stage)?;
        match validate_extracted_snapshot(&stage) {
            Ok(()) => {
                fs::rename(&stage, &root)
                    .map_err(|error| format!("Cannot finish interrupted restore: {error}"))?;
                if displaced_exists {
                    ensure_recovery_directory(&displaced)?;
                    let _ = fs::remove_dir_all(&displaced);
                }
                remove_recovery_journal(&journal_path)?;
                return Ok(());
            }
            Err(stage_error) if displaced_exists => {
                ensure_recovery_directory(&displaced)?;
                fs::rename(&displaced, &root).map_err(|error| {
                    format!("Restore stage is invalid ({stage_error}); cannot roll back: {error}")
                })?;
                let _ = fs::remove_dir_all(&stage);
                remove_recovery_journal(&journal_path)?;
                return Err(format!(
                    "Restore stage was invalid and the previous data was recovered: {stage_error}"
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Restore stage is invalid and no previous data exists: {error}"
                ));
            }
        }
    }

    if displaced_exists {
        ensure_recovery_directory(&displaced)?;
        fs::rename(&displaced, &root)
            .map_err(|error| format!("Cannot roll back interrupted restore: {error}"))?;
        remove_recovery_journal(&journal_path)?;
        return Ok(());
    }
    Err("Restore recovery record points to no recoverable app data".to_owned())
}

fn recovery_path_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot inspect restore recovery path: {error}")),
    }
}

fn restore_backup_sync(root: &Path, archive_path: &Path) -> Result<(), TransactionError> {
    let root = fs::canonicalize(root)
        .map_err(|error| TransactionError::new(format!("Cannot resolve app data root: {error}")))?;
    let archive_path = fs::canonicalize(archive_path).map_err(|error| {
        TransactionError::new(format!("Cannot resolve backup archive: {error}"))
    })?;
    let archive_metadata = fs::metadata(&archive_path).map_err(|error| {
        TransactionError::new(format!("Cannot inspect backup archive: {error}"))
    })?;
    if !archive_metadata.is_file() || archive_metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(TransactionError::new(
            "Backup archive is not a regular file or exceeds the supported size limit",
        ));
    }

    let parent = root
        .parent()
        .ok_or_else(|| TransactionError::new("App data root has no parent folder"))?;
    let stage_dir = tempfile::Builder::new()
        .prefix("legado-restore-")
        .tempdir_in(parent)
        .map_err(|error| {
            TransactionError::new(format!("Cannot create restore staging folder: {error}"))
        })?;
    let stage_path = stage_dir.path().to_path_buf();
    extract_and_validate(&archive_path, &stage_path).map_err(TransactionError::new)?;
    copy_runtime_source_engine(
        &root.join("source-engine"),
        &stage_path.join("source-engine"),
    )
    .map_err(TransactionError::new)?;

    let stage_path = stage_dir.keep();
    commit_restore(&root, &stage_path)
}

fn commit_restore(root: &Path, stage: &Path) -> Result<(), TransactionError> {
    let parent = root
        .parent()
        .ok_or_else(|| TransactionError::new("App data root has no parent folder"))?;
    if stage.parent() != Some(parent) {
        return Err(TransactionError::new(
            "Restore stage must be a sibling of the app data folder",
        ));
    }
    let displaced = parent.join(format!(
        ".legado-previous-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| TransactionError::new("App data root has an invalid folder name"))?;
    let journal_path = parent.join(format!(".legado-restore-{root_name}.json"));
    let journal = RestoreJournal {
        stage_name: stage
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| TransactionError::new("Restore stage has an invalid folder name"))?
            .to_owned(),
        displaced_name: displaced
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| TransactionError::new("Restore rollback folder has an invalid name"))?
            .to_owned(),
    };
    let bytes =
        serde_json::to_vec(&journal).map_err(|error| TransactionError::new(error.to_string()))?;
    if let Err(error) =
        AtomicFile::new(&journal_path, AllowOverwrite).write(|file| file.write_all(&bytes))
    {
        let _ = fs::remove_dir_all(stage);
        return Err(TransactionError::new(format!(
            "Cannot record restore recovery state: {error}"
        )));
    }

    if let Err(error) = fs::rename(root, &displaced) {
        let journal_cleanup = remove_recovery_journal(&journal_path);
        let _ = fs::remove_dir_all(stage);
        return match journal_cleanup {
            Ok(()) => Err(TransactionError::new(format!(
                "Cannot stage existing app data for restore: {error}"
            ))),
            Err(cleanup_error) => Err(TransactionError::recovery_required(
                format!(
                    "Cannot stage existing app data for restore ({error}); restore journal cleanup is pending: {cleanup_error}"
                ),
                CommitState::NotCommitted,
            )),
        };
    }
    if let Err(error) = fs::rename(stage, root) {
        if let Err(rollback_error) = fs::rename(&displaced, root) {
            return Err(TransactionError::recovery_required(
                format!(
                    "Restore activation failed ({error}); previous data remains at {} and recovery will retry ({rollback_error})",
                    displaced.display()
                ),
                CommitState::Indeterminate,
            ));
        }
        let journal_cleanup = remove_recovery_journal(&journal_path);
        let stage_cleanup = fs::remove_dir_all(stage);
        let detail = match (journal_cleanup, stage_cleanup) {
            (Ok(()), Ok(())) => {
                return Err(TransactionError::new(format!(
                    "Cannot activate restored app data: {error}"
                )));
            }
            (journal, stage) => format!(
                "Cannot activate restored app data ({error}); rollback cleanup is pending: journal={}, stage={}",
                journal
                    .err()
                    .map_or_else(|| "ok".to_owned(), |error| error.to_string()),
                stage
                    .err()
                    .map_or_else(|| "ok".to_owned(), |error| error.to_string()),
            ),
        };
        return Err(TransactionError::recovery_required(
            detail,
            CommitState::NotCommitted,
        ));
    }

    // The active ResourceStore and HTTP server refer to the stable root path,
    // so they immediately read the restored tree after the directory swap.
    // Rebuildable engine caches were copied forward; archived user namespaces
    // replace their existing on-device values.
    if let Err(error) = fs::remove_dir_all(&displaced) {
        return Err(TransactionError::recovery_required(
            format!("Restored data is active, but previous-data cleanup is pending: {error}"),
            CommitState::Committed,
        ));
    }
    if let Err(error) = remove_recovery_journal(&journal_path) {
        return Err(TransactionError::recovery_required(
            format!("Restored data is active, but recovery-journal cleanup is pending: {error}"),
            CommitState::Committed,
        ));
    }
    Ok(())
}

fn validate_recovery_name<'a>(name: &'a str, prefix: &str) -> Result<&'a str, String> {
    if !name.starts_with(prefix)
        || name.len() > 200
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(char::is_control)
        || Path::new(name)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Restore recovery record contains an unsafe temporary path".to_owned());
    }
    Ok(name)
}

fn ensure_recovery_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect restore recovery folder: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Restore recovery path is not a regular directory".to_owned());
    }
    Ok(())
}

fn remove_recovery_journal(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot clear restore recovery record: {error}")),
    }
}

fn extract_and_validate(archive_path: &Path, stage: &Path) -> Result<(), String> {
    let file =
        File::open(archive_path).map_err(|error| format!("Cannot open backup archive: {error}"))?;
    let mut archive =
        ZipArchive::new(file).map_err(|_| "Backup is not a valid ZIP archive".to_owned())?;
    if archive.len() == 0 || archive.len() > MAX_ENTRIES + 1 {
        return Err("Backup contains an invalid number of ZIP entries".to_owned());
    }
    let mut entries = std::collections::HashMap::new();
    let mut total_declared_size = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| "Cannot inspect backup ZIP entry".to_owned())?;
        if entry.is_dir() {
            return Err("Backup ZIP must not contain directory entries".to_owned());
        }
        let name = entry.name().to_owned();
        validate_archive_path(&name)?;
        if entry.unix_mode().is_some_and(|mode| {
            let kind = mode & 0o170000;
            kind != 0 && kind != 0o100000
        }) {
            return Err(format!("Backup contains a non-regular ZIP entry: {name}"));
        }
        if entries.insert(name.clone(), index).is_some() {
            return Err(format!("Backup contains duplicate ZIP path: {name}"));
        }
        total_declared_size = total_declared_size.saturating_add(entry.size());
        if entry.size() > MAX_FILE_BYTES
            || total_declared_size > MAX_TOTAL_BYTES + MAX_MANIFEST_BYTES
        {
            return Err("Backup ZIP expands beyond the supported size limit".to_owned());
        }
    }

    let manifest_index = *entries
        .get(MANIFEST_PATH)
        .ok_or_else(|| "Backup manifest is missing".to_owned())?;
    let manifest_file = archive
        .by_index(manifest_index)
        .map_err(|_| "Cannot open backup manifest".to_owned())?;
    if manifest_file.size() > MAX_MANIFEST_BYTES {
        return Err("Backup manifest exceeds the supported size limit".to_owned());
    }
    let mut manifest_bytes = Vec::new();
    manifest_file
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest_bytes)
        .map_err(|error| format!("Cannot read backup manifest: {error}"))?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("Backup manifest is invalid JSON: {error}"))?;
    if manifest.format != BACKUP_FORMAT
        || manifest.backup_version != BACKUP_VERSION
        || manifest.app_schema_version != CURRENT_SCHEMA_VERSION
    {
        return Err("Backup format or application schema version is not supported".to_owned());
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_ENTRIES {
        return Err("Backup manifest has an invalid file count".to_owned());
    }

    let mut expected = std::collections::HashMap::new();
    let mut total_size = 0u64;
    for file in &manifest.files {
        validate_archive_path(&file.path)?;
        if file.path == MANIFEST_PATH || !is_allowed_snapshot_path(&file.path) {
            return Err(format!(
                "Backup contains a forbidden data path: {}",
                file.path
            ));
        }
        if file.size > MAX_FILE_BYTES
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(format!(
                "Backup manifest has invalid metadata for {}",
                file.path
            ));
        }
        total_size = total_size.saturating_add(file.size);
        if total_size > MAX_TOTAL_BYTES {
            return Err("Backup manifest exceeds the supported expanded size limit".to_owned());
        }
        if expected.insert(file.path.as_str(), file).is_some() {
            return Err(format!("Backup manifest repeats file {}", file.path));
        }
    }
    if entries.len() != expected.len() + 1
        || entries
            .keys()
            .any(|path| path != MANIFEST_PATH && !expected.contains_key(path.as_str()))
    {
        return Err("Backup ZIP entries do not match its manifest".to_owned());
    }

    for file in &manifest.files {
        let index = *entries
            .get(&file.path)
            .ok_or_else(|| format!("Backup file is missing: {}", file.path))?;
        let mut input = archive
            .by_index(index)
            .map_err(|_| format!("Cannot open backup file: {}", file.path))?;
        if input.size() != file.size {
            return Err(format!(
                "Backup file size does not match its manifest: {}",
                file.path
            ));
        }
        let output_path = stage.join(&file.path);
        let parent = output_path
            .parent()
            .ok_or_else(|| "Backup path has no parent".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create restore folder: {error}"))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|error| format!("Cannot stage restored file {}: {error}", file.path))?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| format!("Cannot decompress backup file {}: {error}", file.path))?;
            if read == 0 {
                break;
            }
            size = size.saturating_add(read as u64);
            if size > file.size || size > MAX_FILE_BYTES {
                return Err(format!(
                    "Backup file expands beyond its declared size: {}",
                    file.path
                ));
            }
            hasher.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|error| format!("Cannot stage restored file {}: {error}", file.path))?;
        }
        let digest = hasher.finalize();
        if size != file.size || hex_digest(&digest) != file.sha256.to_ascii_lowercase() {
            return Err(format!("Backup checksum validation failed: {}", file.path));
        }
        output
            .sync_all()
            .map_err(|error| format!("Cannot flush restored file {}: {error}", file.path))?;
    }
    validate_extracted_snapshot(stage)?;
    Ok(())
}

fn validate_extracted_snapshot(stage: &Path) -> Result<(), String> {
    for required in ["shelf.json", "settings.json", "http-tts.json"] {
        if !stage.join(required).is_file() {
            return Err(format!("Backup is missing required app data: {required}"));
        }
    }
    let files = collect_snapshot_files(stage)?;
    validate_snapshot_json(&files)
}

fn copy_runtime_source_engine(source_root: &Path, destination_root: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(source_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Cannot inspect runtime source-engine data: {error}"
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Runtime source-engine data is not a regular directory".to_owned());
    }

    let storage_source = source_root.join("storage");
    let storage_metadata = match fs::symlink_metadata(&storage_source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Cannot inspect source storage folder: {error}")),
    };
    if storage_metadata.file_type().is_symlink() || !storage_metadata.is_dir() {
        return Err("Source storage folder is not a regular directory".to_owned());
    }

    for namespace in DEVICE_SOURCE_CACHES {
        let from = storage_source.join(format!("{namespace}.json"));
        let metadata = match fs::symlink_metadata(&from) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot inspect source cache {namespace}: {error}")),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("Source cache {namespace} is not a regular file"));
        }
        let destination = destination_root
            .join("storage")
            .join(format!("{namespace}.json"));
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot preserve source cache directory: {error}"))?;
        }
        fs::copy(from, destination)
            .map_err(|error| format!("Cannot preserve source cache {namespace}: {error}"))?;
    }
    Ok(())
}
