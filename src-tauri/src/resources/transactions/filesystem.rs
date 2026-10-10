//! 安全创建目录并原子替换、同步事务文件。

use super::*;
use std::{fs, io::Write as _, path::Component};

use atomicwrites::{AllowOverwrite, AtomicFile};
use sha2::{Digest, Sha256};

pub(super) fn canonical_root(root: &Path) -> Result<PathBuf, TransactionError> {
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

pub(super) fn validate_guard_root(
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

pub(super) fn validate_target_path(
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

pub(super) fn ensure_safe_directory(root: &Path, directory: &Path) -> Result<(), TransactionError> {
    reject_symlink_components(root, directory, true)?;
    fs::create_dir_all(directory).map_err(|error| {
        TransactionError::new(format!("Cannot create transaction directory: {error}"))
    })?;
    reject_symlink_components(root, directory, false)?;
    sync_parent_chain(root, directory)
}

pub(super) fn validate_existing_path_no_symlinks(
    root: &Path,
    path: &Path,
) -> Result<(), TransactionError> {
    reject_symlink_components(root, path, false)
}

pub(super) fn reject_symlink_components(
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
                return Ok(());
            }
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect transaction path: {error}"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn atomic_replace(
    root: &Path,
    path: &Path,
    bytes: &[u8],
) -> Result<(), TransactionError> {
    reject_symlink_components(root, path, true)?;
    let parent = path
        .parent()
        .ok_or_else(|| TransactionError::new("Transaction target has no parent"))?;
    ensure_parent_directories(root, parent)?;
    atomic_replace_path(path, bytes)?;
    sync_parent_chain(root, parent)
}

pub(super) fn ensure_parent_directories(
    root: &Path,
    directory: &Path,
) -> Result<(), TransactionError> {
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
                )));
            }
        }
    }
    reject_symlink_components(root, directory, false)
}

pub(super) fn sync_parent_chain(root: &Path, directory: &Path) -> Result<(), TransactionError> {
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

pub(super) fn atomic_replace_path(path: &Path, bytes: &[u8]) -> Result<(), TransactionError> {
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

pub(super) fn sync_parent(root: &Path, path: &Path) -> Result<(), TransactionError> {
    if !path.starts_with(root) {
        return Err(TransactionError::new(
            "Directory sync path escaped app-data root",
        ));
    }
    let mut parent = path
        .parent()
        .ok_or_else(|| TransactionError::new("Transaction path has no parent"))?;
    loop {
        match fs::symlink_metadata(parent) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(TransactionError::new(
                        "Transaction parent is not a regular directory",
                    ));
                }
                reject_symlink_components(root, parent, false)?;
                return sync_directory(parent);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && parent != root => {
                parent = parent.parent().ok_or_else(|| {
                    TransactionError::new("Transaction path has no app-data ancestor")
                })?;
            }
            Err(error) => {
                return Err(TransactionError::new(format!(
                    "Cannot inspect transaction parent directory: {error}"
                )));
            }
        }
    }
}

#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> Result<(), TransactionError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| TransactionError::new(format!("Cannot sync directory metadata: {error}")))
}

#[cfg(not(unix))]
pub(super) fn sync_directory(_path: &Path) -> Result<(), TransactionError> {
    // Rust's portable std API does not expose directory handles with a
    // cross-platform sync contract. Files are still synced before replacement;
    // this is a weaker metadata durability guarantee on these targets.
    Ok(())
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
