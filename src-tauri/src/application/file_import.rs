//! 本地文件导入和受保护 PDF 导入的短期状态管理。

use super::*;

pub(super) struct PickerFileCleanupOwner {
    pub(super) path: PathBuf,
    pub(super) cleanup_dir: Option<PathBuf>,
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
    pub(super) path: PathBuf,
    /// Path used only as an identity input for legacy-compatible local book IDs.
    /// Parsing never reads this path after the file has been staged.
    pub(super) identity_path: PathBuf,
    pub(super) _cleanup: Option<Arc<PickerFileCleanupOwner>>,
}

impl PickerFile {
    /// Wrap a caller-owned file path for the shared PDF challenge flow.
    /// Native picker flows use a temporary staged path and transfer cleanup
    /// ownership into the same type before calling the service.
    pub fn from_existing_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            identity_path: path.clone(),
            path,
            _cleanup: None,
        }
    }

    pub(super) fn temporary(path: impl Into<PathBuf>, cleanup_dir: Option<PathBuf>) -> Self {
        let path = path.into();
        Self {
            identity_path: path.clone(),
            _cleanup: Some(Arc::new(PickerFileCleanupOwner {
                path: path.clone(),
                cleanup_dir,
            })),
            path,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingExternalFileDescriptor {
    pub token: String,
    pub filename: String,
    pub format: String,
}

#[derive(Clone)]
pub(super) struct PendingExternalFile {
    pub(super) descriptor: PendingExternalFileDescriptor,
    pub(super) file: PickerFile,
    pub(super) created_at: Instant,
    pub(super) importing: bool,
}

#[derive(Default)]
struct PendingExternalFileState {
    entries: Vec<PendingExternalFile>,
}

/// Short-lived Rust-owned grants for native file-open requests. The WebView
/// sees only an opaque token and safe display metadata, never the source path.
#[derive(Clone)]
pub struct PendingExternalFileRegistry {
    state: Arc<std::sync::Mutex<PendingExternalFileState>>,
}

impl PendingExternalFileRegistry {
    pub fn new() -> Self {
        Self {
            state: Arc::new(std::sync::Mutex::new(PendingExternalFileState::default())),
        }
    }

    pub fn enqueue_picker_file(
        &self,
        file: PickerFile,
    ) -> Result<Option<PendingExternalFileDescriptor>, String> {
        let path = &file.path;
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            return Ok(None);
        };
        let format = match extension.to_ascii_lowercase().as_str() {
            "txt" => "txt",
            "epub" => "epub",
            "cbz" => "cbz",
            "pdf" => "pdf",
            _ => return Ok(None),
        };
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("Cannot inspect selected book file: {error}"))?;
        if !metadata.is_file() {
            return Ok(None);
        }
        if metadata.len() > MAX_PICKER_FILE_BYTES {
            return Err("Selected book file exceeds the 512 MiB limit".to_owned());
        }
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| {
                value
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(160)
                    .collect::<String>()
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("book.{format}"));

        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending external file queue is unavailable".to_owned())?;
        state.entries.retain(|entry| {
            entry.importing
                || now
                    .checked_duration_since(entry.created_at)
                    .unwrap_or_default()
                    < PENDING_EXTERNAL_FILE_IMPORT_TTL
        });
        if let Some(existing) = state.entries.iter().find(|entry| entry.file.path == *path) {
            return Ok(Some(existing.descriptor.clone()));
        }
        if state.entries.len() >= MAX_PENDING_EXTERNAL_FILE_IMPORTS {
            return Err("There are too many pending book files; process one first".to_owned());
        }
        let descriptor = PendingExternalFileDescriptor {
            token: uuid::Uuid::new_v4().simple().to_string(),
            filename,
            format: format.to_owned(),
        };
        state.entries.push(PendingExternalFile {
            descriptor: descriptor.clone(),
            file,
            created_at: now,
            importing: false,
        });
        Ok(Some(descriptor))
    }

    pub fn list(&self) -> Result<Vec<PendingExternalFileDescriptor>, String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending external file queue is unavailable".to_owned())?;
        state.entries.retain(|entry| {
            entry.importing
                || now
                    .checked_duration_since(entry.created_at)
                    .unwrap_or_default()
                    < PENDING_EXTERNAL_FILE_IMPORT_TTL
        });
        Ok(state
            .entries
            .iter()
            .map(|entry| entry.descriptor.clone())
            .collect())
    }

    // Only the Tauri file-open IPC uses claim lifecycle operations.
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(super) fn claim(&self, token: &str) -> Result<Option<PendingExternalFile>, String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending external file queue is unavailable".to_owned())?;
        state.entries.retain(|entry| {
            entry.importing
                || now
                    .checked_duration_since(entry.created_at)
                    .unwrap_or_default()
                    < PENDING_EXTERNAL_FILE_IMPORT_TTL
        });
        let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.descriptor.token == token)
        else {
            return Ok(None);
        };
        if entry.importing {
            return Err("This book file is already being imported".to_owned());
        }
        entry.importing = true;
        Ok(Some(entry.clone()))
    }

    // Only the Tauri file-open IPC uses claim lifecycle operations.
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(super) fn finish(&self, token: &str) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending external file queue is unavailable".to_owned())?;
        state
            .entries
            .retain(|entry| entry.descriptor.token != token);
        Ok(())
    }

    // Only the Tauri file-open IPC uses claim lifecycle operations.
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(super) fn release(&self, token: &str) -> Result<(), String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending external file queue is unavailable".to_owned())?;
        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.descriptor.token == token)
        {
            entry.importing = false;
            entry.created_at = now;
        }
        Ok(())
    }

    // Only the Tauri file-open IPC uses claim lifecycle operations.
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(super) fn discard(&self, token: &str) -> Result<(), String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending external file queue is unavailable".to_owned())?;
        if state
            .entries
            .iter()
            .any(|entry| entry.descriptor.token == token && entry.importing)
        {
            return Err("This book file is already being imported".to_owned());
        }
        state.entries.retain(|entry| {
            entry.descriptor.token != token
                && now
                    .checked_duration_since(entry.created_at)
                    .unwrap_or_default()
                    < PENDING_EXTERNAL_FILE_IMPORT_TTL
        });
        Ok(())
    }

    pub fn expire(&self, token: &str) -> Result<bool, String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending external file queue is unavailable".to_owned())?;
        let before = state.entries.len();
        state.entries.retain(|entry| {
            entry.descriptor.token != token
                || entry.importing
                || now
                    .checked_duration_since(entry.created_at)
                    .unwrap_or_default()
                    < PENDING_EXTERNAL_FILE_IMPORT_TTL
        });
        Ok(state.entries.len() != before)
    }
}

pub fn schedule_pending_external_file_cleanup(
    registry: PendingExternalFileRegistry,
    token: String,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(async move {
        tokio::time::sleep(PENDING_EXTERNAL_FILE_IMPORT_TTL).await;
        let _ = registry.expire(&token);
    });
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(super) struct PendingExternalFileImportClaim {
    registry: PendingExternalFileRegistry,
    token: String,
    finished: bool,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl PendingExternalFileImportClaim {
    pub(super) fn new(registry: PendingExternalFileRegistry, token: String) -> Self {
        Self {
            registry,
            token,
            finished: false,
        }
    }

    pub(super) fn finish(&mut self) -> Result<(), String> {
        self.registry.finish(&self.token)?;
        self.finished = true;
        Ok(())
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl Drop for PendingExternalFileImportClaim {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Err(error) = self.registry.release(&self.token) {
            eprintln!("[external-file-open] cannot release pending import claim: {error}");
        } else {
            schedule_pending_external_file_cleanup(self.registry.clone(), self.token.clone());
        }
    }
}

struct PendingPdfImport {
    file: PickerFile,
    created_at: Instant,
}

/// GUI-independent owner for temporary protected-PDF picker files.
#[derive(Clone)]
pub(super) struct PendingPdfImportRegistry {
    state: Arc<std::sync::Mutex<PendingPdfImportState>>,
}

#[derive(Default)]
struct PendingPdfImportState {
    active_token: Option<String>,
    created_at: Option<Instant>,
    pending: Option<PendingPdfImport>,
}

impl PendingPdfImportRegistry {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new(std::sync::Mutex::new(PendingPdfImportState::default())),
        }
    }

    fn insert(&self, file: PickerFile) -> Result<String, String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        // A service has one active password challenge. Starting another
        // protected-file import invalidates and cleans up the older picker.
        let token = uuid::Uuid::new_v4().simple().to_string();
        let replaced = state.pending.replace(PendingPdfImport {
            file,
            created_at: now,
        });
        state.active_token = Some(token.clone());
        state.created_at = Some(now);
        drop(state);
        drop(replaced);
        Ok(token)
    }

    fn take(&self, token: &str) -> Result<Option<PendingPdfImport>, String> {
        self.prune()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        if state.active_token.as_deref() != Some(token) {
            return Ok(None);
        }
        Ok(state.pending.take())
    }

    /// Reinsert an attempted import after a recoverable parse/password error.
    /// Expiry remains anchored to the original picker selection time.
    fn reinsert(&self, token: String, pending: PendingPdfImport) -> Result<(), String> {
        let now = Instant::now();
        if now
            .checked_duration_since(pending.created_at)
            .unwrap_or_default()
            >= PENDING_PDF_IMPORT_TTL
        {
            self.expire(&token)?;
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        if state.active_token.as_deref() == Some(token.as_str()) && state.pending.is_none() {
            state.pending = Some(pending);
        }
        Ok(())
    }

    fn expire(&self, token: &str) -> Result<bool, String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        let expired = state.active_token.as_deref() == Some(token)
            && state.created_at.is_some_and(|created_at| {
                now.checked_duration_since(created_at).unwrap_or_default() >= PENDING_PDF_IMPORT_TTL
            });
        if expired {
            state.active_token = None;
            state.created_at = None;
            state.pending = None;
        }
        Ok(expired)
    }

    fn cancel(&self, token: &str) -> Result<bool, String> {
        self.prune()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        if state.active_token.as_deref() != Some(token) {
            return Ok(false);
        }
        state.active_token = None;
        state.created_at = None;
        state.pending = None;
        Ok(true)
    }

    pub(super) fn clear(&self) -> Result<(), String> {
        let old = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
            std::mem::take(&mut *state)
        };
        drop(old);
        Ok(())
    }

    fn prune(&self) -> Result<(), String> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Pending PDF import registry is unavailable".to_owned())?;
        let expired = state.created_at.is_some_and(|created_at| {
            now.checked_duration_since(created_at).unwrap_or_default() >= PENDING_PDF_IMPORT_TTL
        });
        if expired {
            *state = PendingPdfImportState::default();
        }
        Ok(())
    }
}

fn schedule_pending_pdf_cleanup(registry: PendingPdfImportRegistry, token: String) {
    tokio::spawn(async move {
        tokio::time::sleep(PENDING_PDF_IMPORT_TTL).await;
        let _ = registry.expire(&token);
    });
}

impl ApplicationService {
    async fn stage_local_import(&self, source: &PickerFile) -> Result<PickerFile, String> {
        let extension = source
            .path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|value| matches!(value.as_str(), "txt" | "epub" | "cbz" | "pdf"))
            .ok_or_else(|| "Unsupported local book format".to_owned())?;
        let metadata = tokio::fs::symlink_metadata(&source.path)
            .await
            .map_err(|error| format!("Cannot inspect selected book file: {error}"))?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err("Selected book must be a regular file".to_owned());
        }
        if metadata.len() == 0 || metadata.len() > MAX_PICKER_FILE_BYTES {
            return Err("Selected book file size is outside the supported range".to_owned());
        }

        let staging_directory = self
            .private_root
            .join("picker-imports")
            .join(uuid::Uuid::new_v4().simple().to_string());
        tokio::fs::create_dir_all(&staging_directory)
            .await
            .map_err(|error| format!("Cannot create managed import staging directory: {error}"))?;
        let display_name = source
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("book.{extension}"));
        let staged_path = staging_directory.join(display_name);
        if let Err(error) = tokio::fs::copy(&source.path, &staged_path).await {
            let _ = tokio::fs::remove_dir_all(&staging_directory).await;
            return Err(format!("Cannot copy selected book into managed storage: {error}"));
        }
        let identity_path = std::fs::canonicalize(&source.identity_path)
            .unwrap_or_else(|_| source.identity_path.clone());
        Ok(PickerFile {
            path: staged_path.clone(),
            identity_path,
            _cleanup: Some(Arc::new(PickerFileCleanupOwner {
                path: staged_path,
                cleanup_dir: Some(staging_directory),
            })),
        })
    }

    pub async fn import_local_book(
        &self,
        selected_path: &Path,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let selected = PickerFile::from_existing_path(selected_path.to_path_buf());
        let managed = self.stage_local_import(&selected).await?;
        self.import_local_book_unlocked(&managed, options).await
    }

    async fn import_local_book_unlocked(
        &self,
        selected_file: &PickerFile,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<Value, String> {
        let imported = crate::local_books::import_local_book_with_outcome(
            &self.store,
            &selected_file.path,
            &selected_file.identity_path,
            &options,
        )
        .await?;
        let book = imported.book;
        if let Err(error) = self.upsert_shelf(&book.id).await {
            if imported.created {
                if let Err(cleanup_error) = self.store.remove_book_resources(&book.id).await {
                    return Err(format!(
                        "{error}; additionally failed to clean the new local book: {cleanup_error}"
                    ));
                }
            }
            return Err(error);
        }
        let book_ref = self
            .store
            .book_ref(&book.id)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "book": self.resource_descriptor(&book_ref),
            "shelf": self.resource_descriptor(&self.store.shelf_ref()),
        }))
    }

    /// Import a selected local file using the same recoverable encrypted-PDF
    /// challenge registry that the native picker commands use.
    pub async fn import_local_book_with_challenge(
        &self,
        picked_file: PickerFile,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let managed_file = self.stage_local_import(&picked_file).await?;
        let pdf_password_supplied = options.pdf_password.is_some();
        let is_pdf = managed_file
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
        match self
            .import_local_book_unlocked(&managed_file, options)
            .await
        {
            Ok(result) => Ok(result),
            Err(error)
                if is_pdf && !pdf_password_supplied && error.contains("provide pdfPassword") =>
            {
                let import_token = self.pending_pdf_imports.insert(managed_file)?;
                schedule_pending_pdf_cleanup(
                    self.pending_pdf_imports.clone(),
                    import_token.clone(),
                );
                Ok(json!({ "passwordRequired": true, "importToken": import_token }))
            }
            Err(error) => Err(error),
        }
    }

    /// Retry a pending protected PDF import. Recoverable errors put the same
    /// temporary file back in the registry, preserving its original expiry.
    pub async fn retry_pending_pdf_import(
        &self,
        import_token: &str,
        password: String,
    ) -> Result<Value, String> {
        validate_id(import_token, "importToken")?;
        let _operation = self.operation_read().await;
        let Some(pending) = self.pending_pdf_imports.take(import_token)? else {
            return Err("PDF import expired; select the file again".to_owned());
        };
        if !tokio::fs::try_exists(&pending.file.path)
            .await
            .unwrap_or(false)
        {
            return Err(
                "PDF import file expired after app data was restored; select the file again"
                    .to_owned(),
            );
        }
        let options = crate::local_books::LocalImportOptions {
            pdf_password: Some(password),
            ..Default::default()
        };
        match self
            .import_local_book_unlocked(&pending.file, options)
            .await
        {
            Ok(result) => Ok(result),
            Err(error) => {
                self.pending_pdf_imports
                    .reinsert(import_token.to_owned(), pending)?;
                Err(error)
            }
        }
    }

    pub async fn cancel_pending_pdf_import(&self, import_token: &str) -> Result<bool, String> {
        validate_id(import_token, "importToken")?;
        let _operation = self.operation_read().await;
        self.pending_pdf_imports.cancel(import_token)
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(super) fn picker_display_name(uri: &str, metadata_name: Option<&str>) -> Option<String> {
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
pub(super) fn infer_picker_extension(
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
pub(super) fn read_file_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
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
pub(super) fn copy_stream_limited<R: std::io::Read, W: std::io::Write>(
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
pub(super) fn safe_picker_file_name(display_name: Option<&str>, extension: &str) -> String {
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
pub(super) fn looks_like_picker_text(bytes: &[u8]) -> bool {
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
pub(super) fn looks_like_picker_json(bytes: &[u8]) -> bool {
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
