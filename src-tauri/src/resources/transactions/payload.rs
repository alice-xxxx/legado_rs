//! 写入并读取事务前后快照数据。

use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};

pub(super) fn write_payload(
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

pub(super) fn read_payload(
    stage_dir: &Path,
    snapshot: &Snapshot,
) -> Result<Vec<u8>, TransactionError> {
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

pub(super) fn read_snapshot_bytes(
    root: &Path,
    path: &Path,
) -> Result<Option<Vec<u8>>, TransactionError> {
    reject_symlink_components(root, path, true)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(TransactionError::new(format!(
                "Cannot inspect transaction target: {error}"
            )));
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
