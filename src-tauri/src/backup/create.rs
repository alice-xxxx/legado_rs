//! 收集公开资源、校验快照并创建备份归档。

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::models::{
    CURRENT_SCHEMA_VERSION, SettingsDocument,
};
use crate::resources::ResourceStore;

use super::{
    BACKED_SOURCE_STORAGE, BACKUP_FORMAT, BACKUP_VERSION, BackupEntry, BackupManifest,
    MANIFEST_PATH, MAX_ENTRIES, MAX_FILE_BYTES, MAX_MANIFEST_BYTES, MAX_TOTAL_BYTES, hex_digest,
};

#[derive(Clone, Debug)]
pub(super) struct SnapshotFile {
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

pub(super) fn collect_snapshot_files(root: &Path) -> Result<Vec<SnapshotFile>, String> {
    let mut files = Vec::new();
    for required in [crate::resources::content::LIBRARY_FILE, "settings.json", "http-tts.json", "history.json"] {
        let path = root.join(required);
        if !path.is_file() {
            return Err(format!("Required app data file is missing: {required}"));
        }
        add_snapshot_file(&mut files, root, &path, required.to_owned())?;
    }
    for required in [
        "sources.json",
        "replacement-rules.json",
        "txt-toc-rules.json",
        "tasks.json",
        "rss-state.json",
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
    for directory in ["cache"] {
        collect_tree(root, &root.join(directory), directory, &mut files)?;
    }

    let source_engine = root.join("source-engine");
    ensure_regular_directory_if_present(root, &source_engine, "source-engine")?;
    collect_tree(
        root,
        &source_engine.join("discovery-categories"),
        "source-engine/discovery-categories",
        &mut files,
    )?;
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
        if archive_prefix == "search"
            && name.starts_with("search-snapshot-")
            && name.ends_with(".json")
        {
            continue;
        }
        let path = entry.path();
        let archive_path = format!("{archive_prefix}/{name}");
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Cannot inspect app data entry {archive_path}: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("App data contains a symbolic link: {archive_path}"));
        }
        if metadata.is_dir() {
            if archive_prefix == "books" && matches!(name.as_str(), "debug-source" | "tts-audio") {
                continue;
            }
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

pub(super) fn validate_snapshot_json(files: &[SnapshotFile]) -> Result<(), String> {
    for file in files {
        let bytes = fs::read(&file.source_path).map_err(|error| {
            format!("Cannot read {} for validation: {error}", file.archive_path)
        })?;
        match file.archive_path.as_str() {
            crate::resources::content::LIBRARY_FILE => {
                let library: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                crate::resources::content::validate_library(&library).map_err(|error| error.to_string())?;
            }
            "settings.json" => {
                let document: SettingsDocument = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid settings JSON: {error}"))?;
                crate::discovery::home_config::validate_home_document(&document.home)?;
            }
            "history.json" => {
                let document: Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid history JSON: {error}"))?;
                crate::reading_tools::validate_history_document(&document)
                    .map_err(|error| format!("Invalid history JSON: {error}"))?;
            }
            "replacement-rules.json" => {
                let _: crate::reading_tools::ReplacementRuleDocument =
                    serde_json::from_slice(&bytes)
                        .map_err(|error| format!("Invalid replacement rules JSON: {error}"))?;
            }
            "txt-toc-rules.json" => {
                let document: crate::local_books::txt_toc_rules::TxtTocRulesDocument =
                    serde_json::from_slice(&bytes)
                        .map_err(|error| format!("Invalid TXT TOC rules JSON: {error}"))?;
                crate::local_books::txt_toc_rules::validate_document(&document)?;
            }
            "rss-state.json" => {
                let document: Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid RSS state JSON: {error}"))?;
                crate::rss::validate_rss_state(&document)?;
            }
            "sources.json" => {
                let records: Vec<crate::application::SourceRecord> = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid source JSON: {error}"))?;
                crate::application::validate_source_records(&records)?;
            }
            path if crate::resources::content::is_content_document(path) => {
                let document: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                let (kind, id, _) = crate::resources::content::content_path(path).ok_or("Invalid content path")?;
                if document["id"].as_str() != Some(id) { return Err(format!("Content ID does not match path: {path}")); }
                crate::resources::content::book_view(&document, kind).map_err(|error| error.to_string())?;
                crate::resources::validate_persistable_json_bytes(&bytes).map_err(|error| error.to_string())?;
            }
            "tasks.json" => {
                let document: Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid tasks JSON: {error}"))?;
                if !document["tasks"].is_array() || document["schemaVersion"].as_u64() != Some(CURRENT_SCHEMA_VERSION as u64) {
                    return Err("Invalid tasks document".into());
                }
            }
            "http-tts.json" => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid HTTP TTS configuration: {error}"))?;
                if !value.is_object() {
                    return Err("HTTP TTS configuration must be a JSON object".to_owned());
                }
                crate::http_tts_config::list_configs(&value)?;
            }
            path if path.starts_with("source-engine/discovery-categories/") && path.ends_with(".json") => {
                let value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if !value.is_object() { return Err(format!("Invalid discovery categories: {path}")); }
            }
            path if crate::resources::content::content_path(path).is_some() => {
                if path.ends_with(".txt") || path.ends_with(".html") || path.ends_with(".m3u8") {
                    std::str::from_utf8(&bytes).map_err(|_| format!("Native text is not UTF-8: {path}"))?;
                }
            }
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

pub(super) fn is_allowed_snapshot_path(path: &str) -> bool {
    if crate::resources::content::content_path(path).is_some_and(|(_, id, _)| id == "debug-source" || id == "tts-audio")
    {
        return false;
    }
    if matches!(
        path,
        crate::resources::content::LIBRARY_FILE
            | "settings.json"
            | "http-tts.json"
            | "history.json"
            | "replacement-rules.json"
            | "sources.json"
            | "txt-toc-rules.json"
            | "tasks.json"
            | "rss-state.json"
    ) {
        return true;
    }
    let parts = path.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["cache", kind, id, name] => crate::resources::content::CONTENT_TYPES.contains(kind)
            && valid_resource_id(id)
            && (name == &format!("{id}.json") || crate::resources::ResourceRef::new(format!("resource://{path}")).is_ok()),
        ["source-engine", "discovery-categories", source_file] => source_file
            .strip_suffix(".json").is_some_and(valid_source_record_id),
        ["source-engine", "storage", storage_file] => BACKED_SOURCE_STORAGE
            .iter()
            .any(|namespace| storage_file == &format!("{namespace}.json")),
        _ => false,
    }
}

fn valid_local_original_file(file_name: &str) -> bool {
    matches!(
        file_name,
        "original.txt" | "original.epub" | "original.cbz" | "original.pdf"
    )
}

fn is_local_original_archive_path(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    matches!(
        parts.as_slice(),
        ["cache", kind, book_id, original_file]
            if crate::resources::content::CONTENT_TYPES.contains(kind) && valid_resource_id(book_id) && valid_local_original_file(original_file)
    )
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

fn valid_source_record_id(id: &str) -> bool {
    id.strip_prefix("source-").is_some_and(|suffix| {
        suffix.len() == 16
            && suffix
                .bytes()
                .all(|character| character.is_ascii_digit() || (b'a'..=b'f').contains(&character))
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
