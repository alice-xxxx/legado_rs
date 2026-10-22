//! 归档数据库快照、设置和资源文件。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::models::{CURRENT_SCHEMA_VERSION};
use crate::resources::ResourceStore;

use super::{
    hex_digest, BackupEntry, BackupManifest, BACKUP_FORMAT, BACKUP_VERSION,
    MANIFEST_PATH, MAX_ENTRIES, MAX_FILE_BYTES, MAX_MANIFEST_BYTES, MAX_TOTAL_BYTES,
};

#[derive(Clone, Debug)]
pub(super) struct SnapshotFile {
    pub(super) archive_path: String,
    pub(super) source_path: PathBuf,
    size: u64,
    sha256: String,
}

/// 调用方持有应用独占操作门，归档期间资源文件不变。
pub async fn create_backup(
    store: &ResourceStore,
    destination: impl AsRef<Path>,
) -> Result<(), String> {
    let database=store.database().clone();
    let root = store.root().to_path_buf();
    let destination = destination.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || create_backup_sync(&root, &destination, &database))
        .await
        .map_err(|error| format!("Backup worker failed: {error}"))?
}

fn create_backup_sync(root: &Path, destination: &Path, database:&crate::database::Database) -> Result<(), String> {
    let root =
        fs::canonicalize(root).map_err(|error| format!("Cannot resolve app data root: {error}"))?;
    let destination_parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(destination_parent)
        .map_err(|error| format!("Cannot resolve backup destination folder: {error}"))?;
    let destination_name = destination
        .file_name()
        .ok_or_else(|| "Backup destination must be a file path".to_owned())?;
    let destination = parent.join(destination_name);
    if destination.starts_with(&root) {
        return Err("Backup destination must be outside the application data folder".to_owned());
    }

    let snapshot = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database_file = snapshot.path().join(crate::database::FILE_NAME);
    database.backup_file(database_file.clone()).and_then(crate::database::Pending::wait).map_err(|e| e.to_string())?;
    let files = collect_snapshot_files(&root, &database_file)?;
    if files.len() > MAX_ENTRIES {
        return Err("Application data contains too many files for one backup".to_owned());
    }
    let mut total_size = 0u64;
    for file in &files {
        total_size = total_size.saturating_add(file.size);
        if file.size > MAX_FILE_BYTES || total_size > MAX_TOTAL_BYTES {
            return Err("Application data exceeds the supported backup size limit".to_owned());
        }
    }

    let manifest = BackupManifest {
        format: BACKUP_FORMAT.to_owned(),
        backup_version: BACKUP_VERSION,
        app_schema_version: CURRENT_SCHEMA_VERSION,
        files: files
            .iter()
            .map(|file| BackupEntry {
                path: file.archive_path.clone(),
                size: file.size,
                sha256: file.sha256.clone(),
            })
            .collect(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("Cannot encode backup manifest: {error}"))?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("Backup manifest exceeds the supported size limit".to_owned());
    }

    AtomicFile::new(&destination, AllowOverwrite)
        .write(|output| {
            let mut writer = ZipWriter::new(output);
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .large_file(true)
                .unix_permissions(0o600);
            writer.start_file(MANIFEST_PATH, options)?;
            writer.write_all(&manifest_bytes)?;
            for entry in &files {
                writer.start_file(&entry.archive_path, options)?;
                let mut input = File::open(&entry.source_path)?;
                let copied = std::io::copy(&mut input, &mut writer)?;
                if copied != entry.size {
                    return Err(std::io::Error::other(
                        "Application data changed while backup was being written",
                    ));
                }
            }
            let output = writer.finish()?;
            output.flush()?;
            output.sync_all()
        })
        .map_err(|error| format!("Cannot atomically write backup archive: {error}"))?;
    Ok(())
}

fn collect_snapshot_files(root: &Path, database_file: &Path) -> Result<Vec<SnapshotFile>, String> {
    let mut files = Vec::new();
    add_snapshot_file(&mut files, database_file.parent().unwrap(), database_file, crate::database::FILE_NAME.into())?;
    add_snapshot_file(&mut files, root, &root.join("settings.json"), "settings.json".into())?;
    collect_tree(root, &root.join("cache"), "cache", &mut files)?;
    files.sort_by(|a,b| a.archive_path.cmp(&b.archive_path));
    Ok(files)
}

fn collect_tree(
    root: &Path,
    directory: &Path,
    archive_prefix: &str,
    files: &mut Vec<SnapshotFile>,
) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Cannot inspect app data folder: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "App data folder is not a regular directory: {archive_prefix}"
        ));
    }
    for entry in
        fs::read_dir(directory).map_err(|error| format!("Cannot list app data folder: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Cannot inspect app data entry: {error}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "App data contains a non-UTF-8 filename".to_owned())?;
        let path = entry.path();
        let archive_path = format!("{archive_prefix}/{name}");
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Cannot inspect app data entry {archive_path}: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("App data contains a symbolic link: {archive_path}"));
        }
        if metadata.is_dir() {
            collect_tree(root, &path, &archive_path, files)?;
        } else if metadata.is_file() {
            add_snapshot_file(files, root, &path, archive_path)?;
        } else {
            return Err(format!(
                "App data contains a non-regular file: {archive_path}"
            ));
        }
    }
    Ok(())
}

fn add_snapshot_file(
    files: &mut Vec<SnapshotFile>,
    root: &Path,
    path: &Path,
    archive_path: String,
) -> Result<(), String> {
    validate_archive_path(&archive_path)?;
    if !is_allowed_snapshot_path(&archive_path) {
        return Err(format!(
            "Unsupported app data file in backup: {archive_path}"
        ));
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect app data file {archive_path}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || !path.starts_with(root) {
        return Err(format!(
            "App data file is not a regular in-root file: {archive_path}"
        ));
    }
    let (size, sha256) = hash_file(path)?;
    files.push(SnapshotFile {
        archive_path,
        source_path: path.to_path_buf(),
        size,
        sha256,
    });
    Ok(())
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file =
        File::open(path).map_err(|error| format!("Cannot read app data file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Cannot hash app data file: {error}"))?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok((size, hex_digest(&digest)))
}

pub(super) fn is_allowed_snapshot_path(path: &str) -> bool {
    path == crate::database::FILE_NAME || path == "settings.json" || path.starts_with("cache/")
}

pub(super) fn validate_archive_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return Err(format!("Backup contains an unsafe path: {path:?}"));
    }
    let candidate = Path::new(path);
    if candidate
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(format!("Backup contains an unsafe path: {path:?}"));
    }
    Ok(())
}

