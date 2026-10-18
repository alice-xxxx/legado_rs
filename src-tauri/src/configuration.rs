//! Application preferences remain in settings.json, independently of business data.
use crate::{resources::ResourceError, storage_owner::Owner};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub(crate) type FileOwner = Owner<PathBuf>;
static OWNERS: OnceLock<Mutex<HashMap<PathBuf, Weak<FileOwner>>>> = OnceLock::new();

pub(crate) fn is_file(path: &str) -> bool {
    path == "settings.json"
}

pub(crate) fn owner(root: &Path) -> Result<Arc<FileOwner>, ResourceError> {
    let root = std::fs::canonicalize(root)?;
    let mut owners = OWNERS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| ResourceError::new("Configuration owner registry stopped"))?;
    if let Some(owner) = owners.get(&root).and_then(Weak::upgrade) {
        return Ok(owner);
    }
    owners.retain(|_, owner| owner.strong_count() > 0);
    let owner = Arc::new(Owner::new("configuration-writer", root.clone())?);
    owners.insert(root, Arc::downgrade(&owner));
    Ok(owner)
}

pub(crate) fn read(root: &Path, path: &str) -> Result<Option<Value>, ResourceError> {
    if !is_file(path) {
        return Err(ResourceError::new(
            "Only settings.json is a file configuration",
        ));
    }
    let target = root.join(path);
    crate::resources::check_path_no_symlink(root, &target, true)?;
    match std::fs::read(target) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| ResourceError::new(e.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn update<F>(
    root: &Path,
    path: &str,
    default: Value,
    change: F,
) -> Result<Value, ResourceError>
where
    F: FnOnce(Value) -> Result<Value, ResourceError> + Send + 'static,
{
    if !is_file(path) {
        return Err(ResourceError::new(
            "Only settings.json is a file configuration",
        ));
    }
    let path = path.to_owned();
    owner(root)?.call(move |root| {
        let value = change(read(root, &path)?.unwrap_or(default))?;
        let target = root.join(&path);
        crate::resources::check_path_no_symlink(root, &target, true)?;
        let parent = target
            .parent()
            .ok_or_else(|| ResourceError::new("Configuration has no directory"))?;
        std::fs::create_dir_all(parent)?;
        let bytes =
            serde_json::to_vec_pretty(&value).map_err(|e| ResourceError::new(e.to_string()))?;
        atomicwrites::AtomicFile::new(&target, atomicwrites::AllowOverwrite)
            .write(|file| {
                use std::io::Write;
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .map_err(|e| ResourceError::new(e.to_string()))?;
        Ok(value)
    })
}

pub(crate) fn write(root: &Path, path: &str, value: Value) -> Result<(), ResourceError> {
    update(root, path, Value::Null, move |_| Ok(value)).map(|_| ())
}
