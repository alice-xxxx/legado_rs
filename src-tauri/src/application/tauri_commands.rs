//! Tauri IPC 命令适配层：维持前端命令名，并把业务委托给 Rust 服务。

use super::*;
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};

#[cfg(target_os = "ios")]
struct SecurityScopedFileAccess<'a, R: tauri::Runtime> {
    app: &'a tauri::AppHandle<R>,
    selected: FilePath,
}

#[cfg(target_os = "ios")]
impl<R: tauri::Runtime> Drop for SecurityScopedFileAccess<'_, R> {
    fn drop(&mut self) {
        use tauri_plugin_fs::FsExt;
        if let Err(error) = self
            .app
            .fs()
            .stop_accessing_security_scoped_resource(self.selected.clone())
        {
            eprintln!("[external-file-open] cannot release iOS file access: {error}");
        }
    }
}

pub async fn stage_external_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    selected: FilePath,
) -> Result<PickerFile, String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if let Ok(path) = selected.clone().into_path() {
        return stage_desktop_external_path(
            app,
            path,
            &["txt", "epub", "cbz", "pdf"],
            MAX_PICKER_FILE_BYTES,
        )
        .await;
    }

    selected_to_local(app, selected, &["txt", "epub", "cbz", "pdf"]).await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn stage_desktop_external_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    path: std::path::PathBuf,
    allowed_extensions: &[&str],
    max_bytes: u64,
) -> Result<PickerFile, String> {
    let identity_path = std::fs::canonicalize(&path)
        .map_err(|error| format!("Cannot open selected book file: {error}"))?;
    validate_picker_extension(&identity_path, allowed_extensions)?;
    validate_picker_size(&identity_path, allowed_extensions, max_bytes)?;

    let extension = identity_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "Selected file has no supported extension".to_owned())?;
    let display_name = identity_path.file_name().and_then(|name| name.to_str());
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("private-data")
        .join("picker-imports");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("Cannot prepare external file staging directory: {error}"))?;
    let staging_directory = directory.join(uuid::Uuid::new_v4().simple().to_string());
    tokio::fs::create_dir(&staging_directory)
        .await
        .map_err(|error| format!("Cannot create external file staging folder: {error}"))?;
    let staged_path = staging_directory.join(safe_picker_file_name(display_name, &extension));
    let staging_owner = Arc::new(PickerFileCleanupOwner {
        path: staged_path.clone(),
        cleanup_dir: Some(staging_directory.clone()),
    });

    if let Err(error) = tokio::fs::copy(&identity_path, &staged_path).await {
        let _ = tokio::fs::remove_dir_all(&staging_directory).await;
        return Err(format!("Cannot copy external book into managed staging: {error}"));
    }

    Ok(PickerFile {
        path: staged_path,
        identity_path,
        _cleanup: Some(staging_owner),
    })
}

async fn pick_file(
    app: &tauri::AppHandle,
    label: &'static str,
    extensions: &'static [&'static str],
) -> Result<Option<PickerFile>, String> {
    pick_file_with_limit(app, label, extensions, MAX_PICKER_FILE_BYTES).await
}

async fn pick_file_with_limit(
    app: &tauri::AppHandle,
    label: &'static str,
    extensions: &'static [&'static str],
    max_bytes: u64,
) -> Result<Option<PickerFile>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(label, extensions)
        .pick_file(move |selected| {
            let _ = sender.send(selected);
        });
    let Some(selected) = receiver
        .await
        .map_err(|error| format!("File picker failed: {error}"))?
    else {
        return Ok(None);
    };
    selected_to_local_with_limit(app, selected, extensions, max_bytes)
        .await
        .map(Some)
}

async fn pick_save_file(
    app: &tauri::AppHandle,
    label: &'static str,
    extensions: &'static [&'static str],
) -> Result<Option<FilePath>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(label, extensions)
        .save_file(move |selected| {
            let _ = sender.send(selected);
        });
    let Some(selected) = receiver
        .await
        .map_err(|error| format!("Save picker failed: {error}"))?
    else {
        return Ok(None);
    };
    Ok(Some(selected))
}

async fn selected_to_local<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    selected: FilePath,
    allowed_extensions: &[&str],
) -> Result<PickerFile, String> {
    selected_to_local_with_limit(app, selected, allowed_extensions, MAX_PICKER_FILE_BYTES).await
}

async fn selected_to_local_with_limit<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    selected: FilePath,
    allowed_extensions: &[&str],
    max_bytes: u64,
) -> Result<PickerFile, String> {
    let selected_uri = selected.to_string();
    let display_name = picker_display_name(&selected_uri, None);

    #[cfg(target_os = "ios")]
    let _security_scope = selected_uri
        .starts_with("file:")
        .then(|| SecurityScopedFileAccess {
            app,
            selected: selected.clone(),
        });

    // Desktop paths can be used directly. On Android/iOS, always go
    // through the fs plugin so content URIs and security-scoped URLs are
    // opened while the native permission is active.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if let Ok(path) = selected.clone().into_path() {
        validate_picker_extension(&path, allowed_extensions)?;
        validate_picker_size(&path, allowed_extensions, max_bytes)?;
        return Ok(PickerFile::from_existing_path(path));
    }

    use tauri_plugin_fs::FsExt;
    let mut options = tauri_plugin_fs::OpenOptions::new();
    options.read(true);
    let source = app
        .fs()
        .open(selected.clone(), options)
        .map_err(|error| format!("Cannot read selected file: {error}"))?;
    if source
        .metadata()
        .ok()
        .is_some_and(|metadata| metadata.len() > max_bytes)
    {
        return Err(format!(
            "Selected file exceeds the {} MiB import limit",
            max_bytes / (1024 * 1024)
        ));
    }
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("private-data")
        .join("picker-imports");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let staging_directory = directory.join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir(&staging_directory)
        .map_err(|error| format!("Cannot create selected file staging folder: {error}"))?;
    let raw_path = staging_directory.join("source.bin");
    // Keep an owner in both the command and blocking copy task. If the
    // command future is cancelled, cleanup waits for the copy task to end
    // before removing its staged directory.
    let staging_owner = Arc::new(PickerFileCleanupOwner {
        path: raw_path.clone(),
        cleanup_dir: Some(staging_directory.clone()),
    });
    let copy_staging_owner = staging_owner.clone();
    let copy_path = raw_path.clone();
    let copy_result = tokio::task::spawn_blocking(move || {
        let _staging_owner = copy_staging_owner;
        use std::io;
        let mut destination = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&copy_path)?;
        let copied = copy_stream_limited(source, &mut destination, max_bytes)?;
        destination.sync_all()?;
        Ok::<u64, io::Error>(copied)
    })
    .await;
    let copy_result = match copy_result {
        Ok(result) => result,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&staging_directory).await;
            return Err(format!("Cannot stage selected file: {error}"));
        }
    };
    let file_size = match copy_result {
        Ok(size) => size,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&staging_directory).await;
            if error.kind() == std::io::ErrorKind::InvalidData {
                return Err(format!(
                    "Selected file exceeds the {} MiB import limit",
                    max_bytes / (1024 * 1024)
                ));
            }
            return Err(format!("Cannot stage selected file: {error}"));
        }
    };
    let extension = match infer_picker_extension(&selected_uri, None, &raw_path, allowed_extensions)
    {
        Ok(extension) => extension,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&staging_directory).await;
            return Err(error);
        }
    };
    if extension == "json" && file_size > MAX_PICKER_SOURCE_JSON_BYTES {
        let _ = tokio::fs::remove_dir_all(&staging_directory).await;
        return Err(format!(
            "Selected source file exceeds the {} MiB import limit",
            MAX_PICKER_SOURCE_JSON_BYTES / (1024 * 1024)
        ));
    }
    let path = staging_directory.join(safe_picker_file_name(display_name.as_deref(), &extension));
    if let Err(error) = std::fs::rename(&raw_path, &path) {
        let _ = tokio::fs::remove_dir_all(&staging_directory).await;
        return Err(format!("Cannot finalize selected file: {error}"));
    }
    Ok(PickerFile {
        identity_path: path.clone(),
        path,
        _cleanup: Some(staging_owner),
    })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn validate_picker_size(
    path: &Path,
    allowed_extensions: &[&str],
    max_bytes: u64,
) -> Result<(), String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("Cannot inspect selected file: {error}"))?
        .len();
    if size > max_bytes {
        return Err(format!(
            "Selected file exceeds the {} MiB import limit",
            max_bytes / (1024 * 1024)
        ));
    }
    if allowed_extensions.contains(&"json")
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        && size > MAX_PICKER_SOURCE_JSON_BYTES
    {
        return Err(format!(
            "Selected source file exceeds the {} MiB import limit",
            MAX_PICKER_SOURCE_JSON_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn validate_picker_extension(path: &Path, allowed: &[&str]) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .ok_or_else(|| "Selected file has no supported extension".to_owned())?;
    if allowed.contains(&extension.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "Selected file extension .{extension} is not supported"
        ))
    }
}

fn write_selected_file(
    app: &tauri::AppHandle,
    selected: FilePath,
    bytes: &[u8],
) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    let _security_scope =
        selected
            .to_string()
            .starts_with("file:")
            .then(|| SecurityScopedFileAccess {
                app,
                selected: selected.clone(),
            });

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if let Ok(path) = selected.clone().into_path() {
        return std::fs::write(path, bytes)
            .map_err(|error| format!("Cannot write selected file: {error}"));
    }
    use tauri_plugin_fs::FsExt;
    let mut options = tauri_plugin_fs::OpenOptions::new();
    options.read(false).write(true).create(true).truncate(true);
    let mut file = app
        .fs()
        .open(selected, options)
        .map_err(|error| format!("Cannot open selected destination: {error}"))?;
    std::io::Write::write_all(&mut file, bytes)
        .map_err(|error| format!("Cannot write selected destination: {error}"))
}

#[path = "tauri_commands/backup.rs"]
mod backup;
#[path = "tauri_commands/book_import.rs"]
mod book_import;
#[path = "tauri_commands/books.rs"]
mod books;
#[path = "tauri_commands/content.rs"]
mod content;
#[path = "tauri_commands/core.rs"]
mod core;
#[path = "tauri_commands/reader_shelf.rs"]
mod reader_shelf;
#[path = "tauri_commands/sources_search.rs"]
mod sources_search;
#[path = "tauri_commands/tasks.rs"]
mod tasks;
#[path = "tauri_commands/tts.rs"]
mod tts;

pub use backup::*;
pub use book_import::*;
pub use books::*;
pub use content::*;
pub use core::*;
pub use reader_shelf::*;
pub use sources_search::*;
pub use tasks::*;
pub use tts::*;
