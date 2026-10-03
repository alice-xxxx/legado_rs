//! Versioned, checksummed ZIP snapshots for the resource-oriented data model.
//!
//! The archive contains the public reader/app resources and the private source
//! records needed to reopen search, RSS, and discovery cards. User-authored source
//! cookies/variables are included; rebuildable caches and logs stay on-device.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::models::{
    BookDocument, ProgressDocument, SettingsDocument, ShelfDocument, CURRENT_SCHEMA_VERSION,
};
use crate::resources::ResourceStore;

const BACKUP_FORMAT: &str = "legado-rs-resource-backup";
const BACKUP_VERSION: u32 = 1;
const MANIFEST_PATH: &str = "manifest.json";
const MAX_ENTRIES: usize = 100_000;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const BACKED_SOURCE_STORAGE: &[&str] = &["cookies", "book-variables", "explore-kinds"];
const DEVICE_SOURCE_CACHES: &[&str] = &["source-cache", "file-cache", "persistent-file-cache"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    format: String,
    backup_version: u32,
    app_schema_version: u32,
    files: Vec<BackupEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupEntry {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RestoreJournal {
    stage_name: String,
    displaced_name: String,
}

#[derive(Clone, Debug)]
struct SnapshotFile {
    archive_path: String,
    source_path: PathBuf,
    size: u64,
    sha256: String,
}

/// Write a consistent JSON/resource snapshot atomically to a user-selected
/// destination. Callers must hold the application's exclusive operation guard
/// so state cannot change between the file scan and archive write.
pub async fn create_backup(
    store: &ResourceStore,
    destination: impl AsRef<Path>,
) -> Result<(), String> {
    let root = store.root().to_path_buf();
    let destination = destination.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || create_backup_sync(&root, &destination))
        .await
        .map_err(|error| format!("Backup worker failed: {error}"))?
}

/// Validate and restore a complete snapshot. All ZIP entries are checked and
/// extracted into a sibling staging directory before the current data root is
/// moved, so malformed or incomplete archives leave the current data intact.
/// Callers must hold the application's exclusive operation guard for the
/// duration of restore.
pub async fn restore_backup(
    store: &ResourceStore,
    archive_path: impl AsRef<Path>,
) -> Result<(), String> {
    let root = store.root().to_path_buf();
    let archive_path = archive_path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || restore_backup_sync(&root, &archive_path))
        .await
        .map_err(|error| format!("Restore worker failed: {error}"))?
}

/// Finish or roll back a restore interrupted between directory renames. Call
/// this before opening `ResourceStore`, which otherwise initializes a missing
/// root as a new empty library.
pub fn recover_interrupted_restore(root: impl AsRef<Path>) -> Result<(), String> {
    let requested_root = root.as_ref();
    let requested_parent = requested_root
        .parent()
        .ok_or_else(|| "App data root has no parent folder".to_owned())?;
    let parent = fs::canonicalize(requested_parent)
        .map_err(|error| format!("Cannot resolve app data parent: {error}"))?;
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

fn create_backup_sync(root: &Path, destination: &Path) -> Result<(), String> {
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

    let files = collect_snapshot_files(&root)?;
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
    validate_snapshot_json(&files)?;

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

fn collect_snapshot_files(root: &Path) -> Result<Vec<SnapshotFile>, String> {
    let mut files = Vec::new();
    for required in ["shelf.json", "settings.json"] {
        let path = root.join(required);
        if !path.is_file() {
            return Err(format!("Required app data file is missing: {required}"));
        }
        add_snapshot_file(&mut files, root, &path, required.to_owned())?;
    }
    for required in [
        "bookmarks.json",
        "reading-history.json",
        "replacement-rules.json",
    ] {
        let path = root.join(required);
        match fs::symlink_metadata(&path) {
            Ok(_) => add_snapshot_file(&mut files, root, &path, required.to_owned())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Cannot inspect optional app data file {required}: {error}"
                ));
            }
        }
    }
    for directory in ["books", "progress", "search", "discovery", "reading"] {
        collect_tree(root, &root.join(directory), directory, &mut files)?;
    }

    let private_root = root.join("private-data");
    ensure_regular_directory_if_present(root, &private_root, "private-data")?;
    let sources = private_root.join("sources.json");
    match fs::symlink_metadata(&sources) {
        Ok(_) => add_snapshot_file(
            &mut files,
            root,
            &sources,
            "private-data/sources.json".to_owned(),
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot inspect private sources file: {error}")),
    }
    for directory in [
        "books",
        "search-results",
        "discovery-categories",
        "media-maps",
    ] {
        collect_tree(
            root,
            &private_root.join(directory),
            &format!("private-data/{directory}"),
            &mut files,
        )?;
    }

    let source_engine = root.join("source-engine");
    ensure_regular_directory_if_present(root, &source_engine, "source-engine")?;
    let storage_dir = source_engine.join("storage");
    ensure_regular_directory_if_present(root, &storage_dir, "source-engine/storage")?;
    for namespace in BACKED_SOURCE_STORAGE {
        let path = storage_dir.join(format!("{namespace}.json"));
        match fs::symlink_metadata(&path) {
            Ok(_) => add_snapshot_file(
                &mut files,
                root,
                &path,
                format!("source-engine/storage/{namespace}.json"),
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Cannot inspect source storage {namespace}: {error}"
                ));
            }
        }
    }
    files.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));
    Ok(files)
}

fn ensure_regular_directory_if_present(
    root: &Path,
    path: &Path,
    label: &str,
) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || !path.starts_with(root) =>
        {
            Err(format!(
                "App data folder is not a regular in-root directory: {label}"
            ))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot inspect app data folder {label}: {error}")),
    }
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

fn validate_snapshot_json(files: &[SnapshotFile]) -> Result<(), String> {
    for file in files {
        let bytes = fs::read(&file.source_path).map_err(|error| {
            format!("Cannot read {} for validation: {error}", file.archive_path)
        })?;
        match file.archive_path.as_str() {
            "shelf.json" => {
                serde_json::from_slice::<ShelfDocument>(&bytes)
                    .map_err(|error| format!("Invalid shelf JSON: {error}"))?;
            }
            "settings.json" => {
                serde_json::from_slice::<SettingsDocument>(&bytes)
                    .map_err(|error| format!("Invalid settings JSON: {error}"))?;
            }
            "bookmarks.json" => {
                let _: crate::reading_tools::BookmarkDocument = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid bookmarks JSON: {error}"))?;
            }
            "reading-history.json" => {
                let _: crate::reading_tools::ReadingHistoryDocument =
                    serde_json::from_slice(&bytes)
                        .map_err(|error| format!("Invalid reading history JSON: {error}"))?;
            }
            "replacement-rules.json" => {
                let _: crate::reading_tools::ReplacementRuleDocument =
                    serde_json::from_slice(&bytes)
                        .map_err(|error| format!("Invalid replacement rules JSON: {error}"))?;
            }
            "reading/txt-toc-rules.json" => {
                let document: crate::local_books::txt_toc_rules::TxtTocRulesDocument =
                    serde_json::from_slice(&bytes)
                        .map_err(|error| format!("Invalid TXT TOC rules JSON: {error}"))?;
                crate::local_books::txt_toc_rules::validate_document(&document)?;
            }
            "reading/home-tabs.json" => {
                let document: Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid home tabs JSON: {error}"))?;
                crate::discovery::home_config::validate_home_document(&document)?;
            }
            "reading/rss-state.json" => {
                let document: Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid RSS state JSON: {error}"))?;
                crate::rss::validate_rss_state(&document)?;
            }
            "private-data/sources.json" => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid private source JSON: {error}"))?;
                if !value.is_array() {
                    return Err("Private source JSON must contain a list".to_owned());
                }
            }
            path if path.starts_with("books/") && path.ends_with("/book.json") => {
                let book: BookDocument = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid book JSON at {path}: {error}"))?;
                let expected_id = path
                    .strip_prefix("books/")
                    .and_then(|value| value.strip_suffix("/book.json"))
                    .ok_or_else(|| format!("Invalid book resource path: {path}"))?;
                if book.id != expected_id {
                    return Err(format!("Book id does not match resource path: {path}"));
                }
            }
            path if path.starts_with("progress/") && path.ends_with(".json") => {
                let progress: ProgressDocument = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid progress JSON at {path}: {error}"))?;
                let expected_id = path
                    .strip_prefix("progress/")
                    .and_then(|value| value.strip_suffix(".json"))
                    .ok_or_else(|| format!("Invalid progress resource path: {path}"))?;
                if progress.book_id != expected_id {
                    return Err(format!(
                        "Progress book id does not match resource path: {path}"
                    ));
                }
            }
            path if path.starts_with("search/") && path.ends_with(".json") => {
                let document: crate::models::SearchResultsDocument = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid search resource at {path}: {error}"))?;
                if document.schema_version != CURRENT_SCHEMA_VERSION {
                    return Err(format!("Search resource has an unsupported schema: {path}"));
                }
            }
            path if path.starts_with("discovery/") && path.ends_with(".json") => {
                let document: crate::discovery::DiscoveryCategoriesDocument =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        format!("Invalid discovery resource at {path}: {error}")
                    })?;
                let expected_source = path
                    .strip_prefix("discovery/")
                    .and_then(|value| value.strip_suffix(".json"))
                    .ok_or_else(|| format!("Invalid discovery resource path: {path}"))?;
                if document.source_id != expected_source {
                    return Err(format!(
                        "Discovery source id does not match its resource path: {path}"
                    ));
                }
            }
            path if path.starts_with("reading/") && path.ends_with(".json") => {
                let document: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid reading resource at {path}: {error}"))?;
                if !document.is_object() {
                    return Err(format!("Reading resource must be an object: {path}"));
                }
                if path == "reading/tasks.json"
                    && (!document["tasks"].is_array()
                        || document["schemaVersion"].as_u64()
                            != Some(CURRENT_SCHEMA_VERSION as u64))
                {
                    return Err("Task state has an invalid schema or task list".to_owned());
                }
                if path == "reading/tasks.json" {
                    for (index, task) in document["tasks"].as_array().unwrap().iter().enumerate() {
                        let valid = task.is_object()
                            && task["id"].as_str().is_some_and(valid_resource_id)
                            && task["kind"].as_str().is_some_and(|value| !value.is_empty())
                            && task["status"]
                                .as_str()
                                .is_some_and(|value| !value.is_empty())
                            && task["createdAtMs"].as_u64().is_some()
                            && task["updatedAtMs"].as_u64().is_some();
                        if !valid {
                            return Err(format!("Task record {index} has invalid fields"));
                        }
                    }
                }
            }
            path if path.starts_with("private-data/media-maps/") && path.ends_with(".json") => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct MediaMapping {
                    schema_version: u32,
                    id: String,
                    upstream_url: String,
                    headers: Vec<MediaHeader>,
                }
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct MediaHeader {
                    name: String,
                    value: String,
                }
                let mapping: MediaMapping = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid private media mapping at {path}: {error}"))?;
                let expected_id = path
                    .strip_prefix("private-data/media-maps/")
                    .and_then(|value| value.strip_suffix(".json"))
                    .ok_or_else(|| format!("Invalid private media mapping path: {path}"))?;
                let upstream_url = reqwest::Url::parse(&mapping.upstream_url)
                    .map_err(|_| format!("Private media mapping has an invalid URL: {path}"))?;
                if mapping.schema_version != 1
                    || mapping.id != expected_id
                    || !valid_resource_id(&mapping.id)
                    || !matches!(upstream_url.scheme(), "http" | "https")
                    || upstream_url.host_str().is_none()
                    || !upstream_url.username().is_empty()
                    || upstream_url.password().is_some()
                    || mapping.headers.len() > 64
                {
                    return Err(format!("Private media mapping has invalid fields: {path}"));
                }
                for header in &mapping.headers {
                    let name = reqwest::header::HeaderName::from_bytes(header.name.as_bytes())
                        .map_err(|_| {
                            format!("Private media mapping has an invalid header: {path}")
                        })?;
                    reqwest::header::HeaderValue::from_str(&header.value).map_err(|_| {
                        format!("Private media mapping has an invalid header: {path}")
                    })?;
                    if header.name.len() > 256
                        || header.value.len() > 8 * 1024
                        || matches!(
                            name.as_str(),
                            "connection"
                                | "content-length"
                                | "host"
                                | "keep-alive"
                                | "proxy-authenticate"
                                | "proxy-authorization"
                                | "te"
                                | "trailer"
                                | "transfer-encoding"
                                | "upgrade"
                        )
                    {
                        return Err(format!(
                            "Private media mapping has an unsupported header: {path}"
                        ));
                    }
                }
            }
            path if path.starts_with("private-data/") && path.ends_with(".json") => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid private JSON data at {path}: {error}"))?;
                if !value.is_object() {
                    return Err(format!("Private resource must be a JSON object: {path}"));
                }
            }
            path if path.starts_with("books/") && path.ends_with(".html") => {
                std::str::from_utf8(&bytes)
                    .map_err(|_| format!("Cached chapter is not UTF-8 HTML: {path}"))?;
            }
            path if path.starts_with("books/") && path.contains("/assets/") => {}
            path if path.starts_with("source-engine/storage/") && path.ends_with(".json") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct StorageSnapshot {
                    entries: HashMap<String, StorageSnapshotEntry>,
                }
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct StorageSnapshotEntry {
                    value: String,
                    #[serde(default)]
                    expires_at_ms: Option<u64>,
                }
                let snapshot: StorageSnapshot =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        format!("Invalid private source storage at {path}: {error}")
                    })?;
                if snapshot.entries.keys().any(|key| key.len() > 16_384) {
                    return Err(format!(
                        "Private source storage entry exceeds its limit: {path}"
                    ));
                }
                let _stored_value_bytes = snapshot
                    .entries
                    .values()
                    .map(|entry| entry.value.len())
                    .sum::<usize>();
                let _expiry_records = snapshot
                    .entries
                    .values()
                    .filter_map(|entry| entry.expires_at_ms)
                    .count();
            }
            _ => {
                return Err(format!(
                    "Unsupported app data file in backup: {}",
                    file.archive_path
                ));
            }
        }
    }
    Ok(())
}

fn restore_backup_sync(root: &Path, archive_path: &Path) -> Result<(), String> {
    let root =
        fs::canonicalize(root).map_err(|error| format!("Cannot resolve app data root: {error}"))?;
    let archive_path = fs::canonicalize(archive_path)
        .map_err(|error| format!("Cannot resolve backup archive: {error}"))?;
    let archive_metadata = fs::metadata(&archive_path)
        .map_err(|error| format!("Cannot inspect backup archive: {error}"))?;
    if !archive_metadata.is_file() || archive_metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(
            "Backup archive is not a regular file or exceeds the supported size limit".to_owned(),
        );
    }

    let parent = root
        .parent()
        .ok_or_else(|| "App data root has no parent folder".to_owned())?;
    let stage_dir = tempfile::Builder::new()
        .prefix("legado-restore-")
        .tempdir_in(parent)
        .map_err(|error| format!("Cannot create restore staging folder: {error}"))?;
    let stage_path = stage_dir.path().to_path_buf();
    extract_and_validate(&archive_path, &stage_path)?;
    copy_runtime_source_engine(
        &root.join("source-engine"),
        &stage_path.join("source-engine"),
    )?;

    let stage_path = stage_dir.keep();
    commit_restore(&root, &stage_path)
}

fn commit_restore(root: &Path, stage: &Path) -> Result<(), String> {
    let parent = root
        .parent()
        .ok_or_else(|| "App data root has no parent folder".to_owned())?;
    if stage.parent() != Some(parent) {
        return Err("Restore stage must be a sibling of the app data folder".to_owned());
    }
    let displaced = parent.join(format!(
        ".legado-previous-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "App data root has an invalid folder name".to_owned())?;
    let journal_path = parent.join(format!(".legado-restore-{root_name}.json"));
    let journal = RestoreJournal {
        stage_name: stage
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Restore stage has an invalid folder name".to_owned())?
            .to_owned(),
        displaced_name: displaced
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Restore rollback folder has an invalid name".to_owned())?
            .to_owned(),
    };
    let bytes = serde_json::to_vec(&journal).map_err(|error| error.to_string())?;
    if let Err(error) =
        AtomicFile::new(&journal_path, AllowOverwrite).write(|file| file.write_all(&bytes))
    {
        let _ = fs::remove_dir_all(stage);
        return Err(format!("Cannot record restore recovery state: {error}"));
    }

    if let Err(error) = fs::rename(root, &displaced) {
        let _ = fs::remove_file(&journal_path);
        let _ = fs::remove_dir_all(stage);
        return Err(format!(
            "Cannot stage existing app data for restore: {error}"
        ));
    }
    if let Err(error) = fs::rename(stage, root) {
        if let Err(rollback_error) = fs::rename(&displaced, root) {
            return Err(format!(
                "Restore activation failed ({error}); previous data remains at {} and recovery will retry ({rollback_error})",
                displaced.display()
            ));
        }
        let _ = fs::remove_file(&journal_path);
        let _ = fs::remove_dir_all(stage);
        return Err(format!("Cannot activate restored app data: {error}"));
    }

    // The active ResourceStore and HTTP server refer to the stable root path,
    // so they immediately read the restored tree after the directory swap.
    // Rebuildable engine caches were copied forward; archived user namespaces
    // replace their existing on-device values.
    let _ = fs::remove_dir_all(displaced);
    remove_recovery_journal(&journal_path)?;
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
    for required in ["shelf.json", "settings.json"] {
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

fn is_allowed_snapshot_path(path: &str) -> bool {
    if matches!(
        path,
        "shelf.json"
            | "settings.json"
            | "bookmarks.json"
            | "reading-history.json"
            | "replacement-rules.json"
            | "private-data/sources.json"
    ) {
        return true;
    }
    let parts = path.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["books", book_id, "book.json"] => valid_resource_id(book_id),
        ["books", book_id, "chapters", chapter_file] => {
            valid_resource_id(book_id)
                && chapter_file
                    .strip_suffix(".html")
                    .is_some_and(valid_resource_id)
        }
        ["books", book_id, "assets", asset_file] => {
            valid_resource_id(book_id) && valid_asset_file(asset_file)
        }
        ["progress", progress_file] => progress_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["search", search_file] => search_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["discovery", discovery_file] => discovery_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["reading", state_file] => state_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["private-data", "books", private_book_file] => private_book_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["private-data", "search-results", result_file] => result_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["private-data", "discovery-categories", source_file] => source_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["private-data", "media-maps", media_file] => media_file
            .strip_suffix(".json")
            .is_some_and(valid_resource_id),
        ["source-engine", "storage", storage_file] => BACKED_SOURCE_STORAGE
            .iter()
            .any(|namespace| storage_file == &format!("{namespace}.json")),
        _ => false,
    }
}

fn valid_resource_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id != "."
        && id != ".."
        && !id.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
}

fn valid_asset_file(filename: &str) -> bool {
    let Some((id, extension)) = filename.rsplit_once('.') else {
        return false;
    };
    valid_resource_id(id)
        && matches!(
            extension.to_ascii_lowercase().as_str(),
            "css"
                | "png"
                | "jpg"
                | "jpeg"
                | "webp"
                | "gif"
                | "pdf"
                | "bmp"
                | "woff"
                | "woff2"
                | "ttf"
                | "otf"
                | "mp3"
                | "m4a"
                | "aac"
                | "wav"
                | "ogg"
                | "opus"
                | "mp4"
                | "webm"
        )
}

fn validate_archive_path(path: &str) -> Result<(), String> {
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

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        create_backup, extract_and_validate, recover_interrupted_restore, restore_backup,
        RestoreJournal,
    };
    use crate::models::{
        BookDocument, ChapterDescriptor, ProgressDocument, ProgressSummary, ReaderDefaults,
        CURRENT_SCHEMA_VERSION,
    };
    use crate::resources::ResourceStore;
    use base64::Engine;
    use serde_json::{json, Value};
    use std::io::Write;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    async fn start_media_upstream() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut request = [0u8; 2048];
                    let count = stream.read(&mut request).await.unwrap_or(0);
                    let path = String::from_utf8_lossy(&request[..count]);
                    let body = if path.contains("/media/snapshot") {
                        b"snapshot-media".as_slice()
                    } else {
                        b"later-media".as_slice()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (address, task)
    }

    async fn add_book(store: &ResourceStore, id: &str, title: &str) -> BookDocument {
        let pixel = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==")
            .unwrap();
        let cover = store.write_asset(id, "pixel.png", &pixel).await.unwrap();
        let chapter_src = store
            .write_chapter_html(
                id,
                "chapter-00001",
                r#"<p>Restored chapter</p><img src="../assets/pixel.png">"#,
                &ReaderDefaults::default(),
            )
            .await
            .unwrap();
        let book = BookDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: id.to_owned(),
            title: title.to_owned(),
            author: "Fixture Author".to_owned(),
            cover_src: Some(cover),
            chapter_count: 1,
            latest_chapter: Some("Chapter 1".to_owned()),
            progress: ProgressSummary::default(),
            chapters: vec![ChapterDescriptor {
                id: "chapter-00001".to_owned(),
                title: "Chapter 1".to_owned(),
                index: 0,
                src: Some(chapter_src),
            }],
        };
        store.write_book(&book).await.unwrap();
        let shelf_ref = store.shelf_ref();
        let mut shelf = store.read_json_ref(&shelf_ref).await.unwrap();
        shelf["books"].as_array_mut().unwrap().push(json!({
            "id": id,
            "title": title,
            "author": "Fixture Author",
            "coverSrc": book.cover_src,
            "chapterCount": 1,
            "latestChapter": "Chapter 1",
            "progress": {},
            "groups": [],
        }));
        store.write_json_ref(&shelf_ref, &shelf).await.unwrap();
        book
    }

    #[tokio::test]
    async fn backup_restores_removed_books_and_media_and_removes_later_books() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("app-data");
        let store = ResourceStore::open(&root).unwrap();
        let original = add_book(&store, "snapshot-book", "Snapshot Book").await;
        store
            .write_progress(
                &original.id,
                &ProgressDocument {
                    schema_version: CURRENT_SCHEMA_VERSION,
                    book_id: original.id.clone(),
                    chapter_id: Some("chapter-00001".to_owned()),
                    chapter_index: 0,
                    offset: 91,
                    updated_at_ms: 1234,
                },
            )
            .await
            .unwrap();

        let private_root = root.join("private-data");
        std::fs::create_dir_all(private_root.join("books")).unwrap();
        std::fs::write(
            private_root.join("sources.json"),
            serde_json::to_vec(&json!([{
                "id": "source-private",
                "source": { "bookSourceUrl": "https://example.test/source", "ruleBookInfo": { "name": "fixture" } }
            }]))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            private_root.join("books/snapshot-book.json"),
            br#"{"engineBook":{"url":"https://example.test/book"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(private_root.join("search-results")).unwrap();
        std::fs::write(
            private_root.join("search-results/search-1.json"),
            br#"{"sourceId":"source-private","result":{"bookUrl":"https://example.test/book"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(private_root.join("discovery-categories")).unwrap();
        std::fs::write(
            private_root.join("discovery-categories/source-private.json"),
            br#"{"schemaVersion":1,"sourceId":"source-private","categories":[{"categoryId":"category-1","title":"Latest","url":"https://example.test/list","kind":"engine"}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("search")).unwrap();
        std::fs::write(
            root.join("search/search-1.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": CURRENT_SCHEMA_VERSION,
                "keyword": "restore fixture",
                "page": 2,
                "results": [{ "id": "result-1" }],
                "errors": [],
                "complete": true,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("discovery")).unwrap();
        std::fs::write(
            root.join("discovery/source-private.json"),
            br#"{"schemaVersion":1,"sourceId":"source-private","categories":[{"categoryId":"category-1","title":"Latest","kind":"feed"}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("reading")).unwrap();
        std::fs::write(
            root.join("reading/tasks.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": CURRENT_SCHEMA_VERSION,
                "tasks": [{
                    "id": "task-restore-1",
                    "kind": "chapter-preload",
                    "status": "interrupted",
                    "bookId": "snapshot-book",
                    "sourceIds": ["source-private"],
                    "keyword": "restore fixture",
                    "page": 1,
                    "fromIndex": 2,
                    "total": 8,
                    "completed": 3,
                    "checkOnly": false,
                    "searchId": "search-1",
                    "result": { "cursor": 3 },
                    "createdAtMs": 1,
                    "updatedAtMs": 2,
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("reading/home-tabs.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "tabs": [{
                    "id": "tab-main",
                    "title": "Main",
                    "sortOrder": 0,
                    "sections": [{
                        "id": "section-latest",
                        "title": "Latest",
                        "sourceId": "source-private",
                        "sourceName": "Private source",
                        "categoryId": "category-new",
                        "categoryName": "Latest",
                        "style": 0,
                        "sortOrder": 0,
                        "coverVideo": false,
                    }],
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("reading/rss-state.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "subscriptions": [{
                    "sourceId": "rss-one",
                    "filter": "unread",
                }],
                "articles": [{
                    "sourceId": "rss-one",
                    "articleId": "article-0123456789abcdef0123456789abcdef",
                    "isRead": false,
                    "isFavorite": true,
                    "updatedAtMs": 17,
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("reading/txt-toc-rules.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "rules": [{
                    "id": "backup-toc-rule",
                    "name": "Saved chapter pattern",
                    "rule": "^(Chapter [0-9]+)$",
                    "example": "Chapter 1",
                    "serialNumber": 3,
                    "enable": true,
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("bookmarks.json"),
            br#"{"schemaVersion":1,"bookmarks":[]}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("reading-history.json"),
            br#"{"schemaVersion":1,"sessions":[],"books":[],"days":[],"totalDurationMs":0,"totalSessions":0}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("replacement-rules.json"),
            br#"{"schemaVersion":1,"rules":[]}"#,
        )
        .unwrap();

        let storage = root.join("source-engine/storage");
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(
            storage.join("cookies.json"),
            br#"{"entries":{"https://example.test":{"value":"backup-cookie"}}}"#,
        )
        .unwrap();
        std::fs::write(
            storage.join("book-variables.json"),
            br#"{"entries":{"book-key":{"value":"backup-variable"}}}"#,
        )
        .unwrap();
        std::fs::write(
            storage.join("explore-kinds.json"),
            br#"{"entries":{"source-key":{"value":"backup-category"}}}"#,
        )
        .unwrap();
        std::fs::write(
            storage.join("source-cache.json"),
            br#"{"entries":{"cache-key":{"value":"device-cache"}}}"#,
        )
        .unwrap();

        let (upstream_addr, upstream_task) = start_media_upstream().await;
        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let media_ref = server
            .register_media_with_id(
                "media-1",
                &format!("http://{upstream_addr}/media/snapshot"),
                &[],
            )
            .await
            .unwrap();
        let mapping: Value = serde_json::from_slice(
            &std::fs::read(private_root.join("media-maps/media-1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(mapping["schemaVersion"], 1);
        assert_eq!(mapping["id"], "media-1");
        assert_eq!(
            mapping["upstreamUrl"],
            format!("http://{upstream_addr}/media/snapshot")
        );
        assert!(mapping["headers"].as_array().unwrap().is_empty());

        let archive_path = temporary.path().join("snapshot.legado.zip");
        create_backup(&store, &archive_path).await.unwrap();

        store.remove_book_resources(&original.id).await.unwrap();
        add_book(&store, "later-book", "Later Book").await;
        server
            .register_media_with_id(
                "media-1",
                &format!("http://{upstream_addr}/media/later"),
                &[],
            )
            .await
            .unwrap();
        std::fs::write(private_root.join("sources.json"), b"[]").unwrap();
        std::fs::write(
            storage.join("cookies.json"),
            br#"{"entries":{"https://example.test":{"value":"device-cookie"}}}"#,
        )
        .unwrap();
        std::fs::write(
            storage.join("source-cache.json"),
            br#"{"entries":{"cache-key":{"value":"new-device-cache"}}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("reading/tasks.json"),
            br#"{"schemaVersion":1,"tasks":[]}"#,
        )
        .unwrap();

        restore_backup(&store, &archive_path).await.unwrap();
        server.reload_private_media_mappings().await.unwrap();
        let restored_ref = store.book_ref("snapshot-book").unwrap();
        let restored: BookDocument =
            serde_json::from_value(store.read_json_ref(&restored_ref).await.unwrap()).unwrap();
        assert_eq!(restored.title, "Snapshot Book");
        assert!(!root.join("books/later-book/book.json").exists());
        assert!(!root.join("progress/later-book.json").exists());
        let progress = store
            .read_json_ref(&store.progress_ref("snapshot-book").unwrap())
            .await
            .unwrap();
        assert_eq!(progress["offset"], 91);
        let sources: Value =
            serde_json::from_slice(&std::fs::read(private_root.join("sources.json")).unwrap())
                .unwrap();
        assert_eq!(sources[0]["id"], "source-private");
        assert!(private_root.join("books/snapshot-book.json").is_file());
        assert!(private_root.join("search-results/search-1.json").is_file());
        assert!(private_root
            .join("discovery-categories/source-private.json")
            .is_file());
        assert!(private_root.join("media-maps/media-1.json").is_file());
        assert!(root.join("search/search-1.json").is_file());
        assert!(root.join("discovery/source-private.json").is_file());
        assert_eq!(
            store
                .read_json_ref(&store.reading_ref("home-tabs").unwrap())
                .await
                .unwrap()["tabs"][0]["sections"][0]["categoryId"],
            "category-new"
        );
        assert_eq!(
            store
                .read_json_ref(&store.reading_ref("rss-state").unwrap())
                .await
                .unwrap()["articles"][0]["isFavorite"],
            true
        );
        assert_eq!(
            crate::local_books::txt_toc_rules::read_document(&store)
                .await
                .unwrap()
                .rules[0]
                .id,
            "backup-toc-rule"
        );
        let tasks: Value =
            serde_json::from_slice(&std::fs::read(root.join("reading/tasks.json")).unwrap())
                .unwrap();
        assert_eq!(tasks["tasks"][0]["status"], "interrupted");
        assert_eq!(tasks["tasks"][0]["fromIndex"], 2);
        assert_eq!(tasks["tasks"][0]["completed"], 3);
        assert_eq!(tasks["tasks"][0]["searchId"], "search-1");
        assert_eq!(
            std::fs::read(storage.join("cookies.json")).unwrap(),
            br#"{"entries":{"https://example.test":{"value":"backup-cookie"}}}"#
        );
        assert_eq!(
            std::fs::read(storage.join("book-variables.json")).unwrap(),
            br#"{"entries":{"book-key":{"value":"backup-variable"}}}"#
        );
        assert_eq!(
            std::fs::read(storage.join("explore-kinds.json")).unwrap(),
            br#"{"entries":{"source-key":{"value":"backup-category"}}}"#
        );
        assert_eq!(
            std::fs::read(storage.join("source-cache.json")).unwrap(),
            br#"{"entries":{"cache-key":{"value":"new-device-cache"}}}"#
        );

        // The still-running resource server resolves the restored chapter and
        // image using its original root path.
        let chapter_url =
            reqwest::Url::parse(&server.url_for(restored.chapters[0].src.as_ref().unwrap()))
                .unwrap();
        let chapter_html = reqwest::get(chapter_url.clone())
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(chapter_html.contains("Restored chapter"));
        let cover_url =
            reqwest::Url::parse(&server.url_for(restored.cover_src.as_ref().unwrap())).unwrap();
        let relative_image = chapter_html
            .split("src=\"../assets/")
            .nth(1)
            .unwrap()
            .split('\"')
            .next()
            .unwrap();
        let image_url = chapter_url
            .join(&format!("../assets/{relative_image}"))
            .unwrap();
        assert_eq!(image_url, cover_url);
        let image_response = reqwest::get(image_url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let image = image_response.bytes().await.unwrap();
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(u32::from_be_bytes(image[16..20].try_into().unwrap()), 1);
        assert_eq!(u32::from_be_bytes(image[20..24].try_into().unwrap()), 1);

        let media_response = reqwest::get(server.url_for(&media_ref))
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(
            media_response.bytes().await.unwrap().as_ref(),
            b"snapshot-media"
        );
        upstream_task.abort();
    }

    #[tokio::test]
    async fn invalid_zip_slip_backup_does_not_change_existing_app_data() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("app-data");
        let store = ResourceStore::open(&root).unwrap();
        let book = add_book(&store, "keep-book", "Keep Book").await;
        let before_book = std::fs::read(root.join("books/keep-book/book.json")).unwrap();
        let before_shelf = std::fs::read(root.join("shelf.json")).unwrap();

        let bad_path = temporary.path().join("unsafe.zip");
        let file = std::fs::File::create(&bad_path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file(
            "../escaped.json",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();

        assert!(restore_backup(&store, &bad_path).await.is_err());
        assert_eq!(
            std::fs::read(root.join("books/keep-book/book.json")).unwrap(),
            before_book
        );
        assert_eq!(
            std::fs::read(root.join("shelf.json")).unwrap(),
            before_shelf
        );
        assert_eq!(book.id, "keep-book");
        assert!(!temporary.path().join("escaped.json").exists());
    }

    #[tokio::test]
    async fn interrupted_restore_recovers_after_either_directory_rename() {
        for restore_was_activated in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().join("app-data");
            let store = ResourceStore::open(&root).unwrap();
            add_book(&store, "snapshot-book", "Snapshot Book").await;
            let archive = temporary.path().join("snapshot.zip");
            create_backup(&store, &archive).await.unwrap();

            add_book(&store, "later-book", "Later Book").await;
            let stage_dir = tempfile::Builder::new()
                .prefix("legado-restore-")
                .tempdir_in(temporary.path())
                .unwrap();
            extract_and_validate(&archive, stage_dir.path()).unwrap();
            let stage = stage_dir.keep();
            let displaced = temporary.path().join(format!(
                ".legado-previous-{}",
                uuid::Uuid::new_v4().simple()
            ));
            let journal_path = temporary.path().join(".legado-restore-app-data.json");
            std::fs::write(
                &journal_path,
                serde_json::to_vec(&RestoreJournal {
                    stage_name: stage.file_name().unwrap().to_string_lossy().into_owned(),
                    displaced_name: displaced
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                })
                .unwrap(),
            )
            .unwrap();
            std::fs::rename(&root, &displaced).unwrap();
            if restore_was_activated {
                std::fs::rename(&stage, &root).unwrap();
            }

            recover_interrupted_restore(&root).unwrap();
            assert!(root.join("books/snapshot-book/book.json").is_file());
            assert!(!root.join("books/later-book/book.json").exists());
            assert!(!displaced.exists());
            assert!(!stage.exists());
            assert!(!journal_path.exists());
        }
    }
}
