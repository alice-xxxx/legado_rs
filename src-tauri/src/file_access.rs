//! 配置、备份等文件选择器资源；书籍由 import.rs 直接写入缓存。

use crate::support::stable_hash;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Only picker copies are temporary. Imported originals live in cache/{type}/{id}/.
pub(crate) fn picker_import_directory(root: &Path) -> PathBuf {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let key = stable_hash(&root.to_string_lossy());
    std::env::temp_dir()
        .join("foliver-picker-imports")
        .join(format!("{key:016x}"))
}

pub(crate) struct PickerFileCleanupOwner {
    pub(crate) path: PathBuf,
    pub(crate) cleanup_dir: Option<PathBuf>,
}

impl Drop for PickerFileCleanupOwner {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(directory) = &self.cleanup_dir {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

#[derive(Clone)]
pub struct PickerFile {
    pub(crate) path: PathBuf,
    pub(crate) _cleanup: Option<Arc<PickerFileCleanupOwner>>,
}

impl PickerFile {
    /// 包装调用方拥有的文件，不接管其清理。
    #[cfg(all(
        any(feature = "desktop", feature = "mobile-runtime"),
        not(any(target_os = "android", target_os = "ios"))
    ))]
    pub fn from_existing_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            path,
            _cleanup: None,
        }
    }

    pub(crate) fn temporary(path: impl Into<PathBuf>, cleanup_dir: Option<PathBuf>) -> Self {
        let path = path.into();
        Self {
            _cleanup: Some(Arc::new(PickerFileCleanupOwner {
                path: path.clone(),
                cleanup_dir,
            })),
            path,
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn picker_display_name(uri: &str, metadata_name: Option<&str>) -> Option<String> {
    let encoded_name = metadata_name.or_else(|| {
        uri.split(|character| character == '?' || character == '#')
            .next()?
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|segment| !segment.is_empty())
    })?;
    let decoded = percent_encoding::percent_decode_str(encoded_name).decode_utf8_lossy();
    let name = Path::new(decoded.as_ref())
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)?;
    // Android providers commonly expose a stable opaque document ID as the
    // last `content://` URI segment. Do not mistake it for a display name.
    if metadata_name.is_none()
        && uri.starts_with("content:")
        && Path::new(&name).extension().is_none()
    {
        return None;
    }
    Some(name)
}

/// Determine Android `content://` file types without trusting the opaque URI
/// suffix. Dialog/fs metadata does not expose the provider's MIME type, so use
/// a reported filename first and inspect bounded file signatures/ZIP entries
/// when the provider returns only an opaque document id.
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn infer_picker_extension(
    _uri: &str,
    metadata_name: Option<&str>,
    file_path: &Path,
    allowed_extensions: &[&str],
) -> Result<String, String> {
    // Only trust a provider-supplied filename, never an opaque content URI
    // suffix. Tauri's current dialog API does not expose MIME metadata, so
    // Android content URIs fall through to actual bytes/ZIP structure.
    if let Some(name) = metadata_name {
        if let Some(extension) = Path::new(&name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
        {
            if allowed_extensions.contains(&extension.as_str()) {
                return Ok(extension);
            }
            return Err(format!(
                "Selected file extension .{extension} is not supported"
            ));
        }
    }

    let prefix = read_file_prefix(file_path, 4096)?;
    if allowed_extensions.contains(&"pdf")
        && prefix
            .get(..prefix.len().min(1024))
            .is_some_and(|header| header.windows(5).any(|window| window == b"%PDF-"))
    {
        return Ok("pdf".to_owned());
    }

    if prefix.starts_with(b"PK") {
        let mut archive = zip::ZipArchive::new(
            std::fs::File::open(file_path)
                .map_err(|error| format!("Cannot inspect the selected ZIP archive: {error}"))?,
        )
        .map_err(|_| "Cannot identify the selected ZIP-based book".to_owned())?;
        if archive.len() > 100_000 {
            return Err("Selected archive contains too many entries".to_owned());
        }
        let has_container = archive
            .file_names()
            .any(|name| name == "META-INF/container.xml");
        let mime_type = match archive.by_name("mimetype") {
            Ok(member) if member.size() <= 128 => {
                use std::io::Read;
                let mut content = Vec::new();
                member
                    .take(128)
                    .read_to_end(&mut content)
                    .map_err(|_| "Cannot inspect the selected EPUB archive".to_owned())?;
                String::from_utf8_lossy(&content).trim().to_owned()
            }
            _ => String::new(),
        };
        if has_container
            && mime_type == "application/epub+zip"
            && allowed_extensions.contains(&"epub")
        {
            return Ok("epub".to_owned());
        }
        let contains_comic_image = archive.file_names().any(|name| {
            matches!(
                Path::new(name)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
            )
        });
        if contains_comic_image && allowed_extensions.contains(&"cbz") {
            return Ok("cbz".to_owned());
        }
        return Err("The selected ZIP file is neither a supported EPUB nor a CBZ".to_owned());
    }

    if allowed_extensions.contains(&"json") && looks_like_picker_json(&prefix) {
        return Ok("json".to_owned());
    }
    if allowed_extensions.contains(&"txt") && looks_like_picker_text(&prefix) {
        return Ok("txt".to_owned());
    }
    Err("Cannot determine the selected file type; choose a TXT, EPUB, CBZ, or PDF file".to_owned())
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn read_file_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Cannot inspect the selected file: {error}"))?;
    let mut prefix = Vec::with_capacity(limit);
    file.take(limit as u64)
        .read_to_end(&mut prefix)
        .map_err(|error| format!("Cannot inspect the selected file: {error}"))?;
    Ok(prefix)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn copy_stream_limited<R: std::io::Read, W: std::io::Write>(
    reader: R,
    writer: &mut W,
    limit: u64,
) -> std::io::Result<u64> {
    let mut limited_reader = std::io::Read::take(reader, limit.saturating_add(1));
    let copied = std::io::copy(&mut limited_reader, writer)?;
    if copied > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "selected file exceeds the configured import limit",
        ));
    }
    Ok(copied)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn safe_picker_file_name(display_name: Option<&str>, extension: &str) -> String {
    let stem = display_name
        .and_then(|name| Path::new(name).file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("Imported book");
    let mut safe_stem = stem
        .chars()
        .filter_map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | ' ') {
                Some(if character == ' ' { '_' } else { character })
            } else {
                None
            }
        })
        .take(100)
        .collect::<String>();
    while safe_stem.starts_with('.') {
        safe_stem.remove(0);
    }
    if safe_stem.is_empty() {
        safe_stem = "Imported_book".to_owned();
    }
    format!("{safe_stem}.{extension}")
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn looks_like_picker_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.contains(&0) {
        return false;
    }
    let sample = &bytes[..bytes.len().min(4096)];
    let control_count = sample
        .iter()
        .filter(|byte| **byte < 0x20 && !matches!(**byte, b'\t' | b'\n' | b'\r' | 0x0c))
        .count();
    control_count * 100 <= sample.len()
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn looks_like_picker_json(bytes: &[u8]) -> bool {
    if !looks_like_picker_text(bytes) {
        return false;
    }
    let text = String::from_utf8_lossy(bytes);
    matches!(
        text.trim_start()
            .trim_start_matches('\u{feff}')
            .chars()
            .next(),
        Some('[' | '{')
    )
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod platform {
    use super::*;
    use tauri_plugin_dialog::{DialogExt, FilePath};
    #[cfg(target_os = "ios")]
    pub(crate) struct SecurityScopedFileAccess<'a, R: tauri::Runtime> {
        pub(crate) app: &'a tauri::AppHandle<R>,
        pub(crate) selected: FilePath,
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

    pub(crate) async fn pick_file(
        app: &tauri::AppHandle,
        label: &'static str,
        extensions: &'static [&'static str],
    ) -> Result<Option<PickerFile>, String> {
        pick_file_with_limit(app, label, extensions, MAX_PICKER_FILE_BYTES).await
    }

    pub(crate) async fn pick_file_with_limit(
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

    pub(crate) async fn pick_save_file(
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
        let directory =
            picker_import_directory(&crate::app_data_root(app).map_err(|error| error.to_string())?);
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
        let extension =
            match infer_picker_extension(&selected_uri, None, &raw_path, allowed_extensions) {
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
        let path =
            staging_directory.join(safe_picker_file_name(display_name.as_deref(), &extension));
        if let Err(error) = std::fs::rename(&raw_path, &path) {
            let _ = tokio::fs::remove_dir_all(&staging_directory).await;
            return Err(format!("Cannot finalize selected file: {error}"));
        }
        Ok(PickerFile {
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

    pub(crate) fn write_selected_file(
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
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) use platform::*;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) const MAX_PICKER_FILE_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) const MAX_PICKER_SOURCE_JSON_BYTES: u64 = 32 * 1024 * 1024;
