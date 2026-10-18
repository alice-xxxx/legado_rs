//! 管理应用公开资源文件存储与原子更新。

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ammonia::Builder as HtmlSanitizer;
use atomicwrites::{AllowOverwrite, AtomicFile};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::models::{
    BookDocument, ReaderDefaults, SettingsDocument, ShelfDocument, CURRENT_SCHEMA_VERSION,
};

use super::json::validate_embedded_refs;
use super::reader_html::chapter_document;
use super::validation::{
    check_path_no_symlink, tts_audio_ref, validate_asset_id, validate_id, validate_public_path,
};
use super::{ChapterVideoSource, ResourceError, ResourceRef};

const MAX_TTS_AUDIO_BYTES: usize = 16 * 1024 * 1024;
const MAX_TTS_AUDIO_FILES: usize = 32;

struct StoreInner {
    root: PathBuf,
    database: crate::database::Database,
    asset_lock: Arc<Mutex<()>>,
    local_import_lock: Arc<Mutex<()>>,
    content_types: std::sync::RwLock<HashMap<String, String>>,
}

/// SQLite state storage and filesystem assets consumed by the UI.
/// Complete content documents include private engine state. The HTTP response
/// is a filtered runtime view and never exposes that private state.
#[derive(Clone)]
pub struct ResourceStore {
    inner: Arc<StoreInner>,
}

impl ResourceStore {
    /// Open the application database. Legacy JSON files are not imported.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ResourceError> {
        std::fs::create_dir_all(root.as_ref())
            .map_err(|error| ResourceError::new(format!("Cannot create resource root: {error}")))?;
        let root = std::fs::canonicalize(root.as_ref()).map_err(|error| {
            ResourceError::new(format!("Cannot resolve resource root: {error}"))
        })?;
        ensure_resource_writes_recovered(&root)?;
        let store = Self {
            inner: Arc::new(StoreInner {
                database: crate::database::Database::open(&root)?,
                root,
                asset_lock: Arc::new(Mutex::new(())),
                local_import_lock: Arc::new(Mutex::new(())),
                content_types: std::sync::RwLock::new(HashMap::new()),
            }),
        };

        let mut settings =
            serde_json::to_value(SettingsDocument::default()).expect("serializable default");
        settings["lastBackupAtMs"] = Value::Null;

        for (resource, value) in [
            (store.shelf_ref(), json!({})),
            (store.settings_ref(), settings),
            (
                store.http_tts_configs_ref(),
                crate::http_tts_config::empty_document(),
            ),
        ] {
            if !store.database().exists(resource.path())? {
                store.database().write(resource.path(), &value)?;
            }
        }
        // Cache garbage collection is maintenance, not app-data recovery.
        // A locked, damaged or read-only orphan file must not prevent the
        // user from opening an otherwise valid library.
        Ok(store)
    }

    pub(crate) fn database(&self) -> &crate::database::Database {
        &self.inner.database
    }

    pub(crate) async fn save_chapter_view(&self,book_id:&str,chapter_id:&str,chapter:&Value)->Result<(),ResourceError>{
        let (database,book_id,chapter_id,chapter)=(self.database().clone(),book_id.to_owned(),chapter_id.to_owned(),chapter.clone());
        tokio::task::spawn_blocking(move ||database.save_chapter_view(&book_id,&chapter_id,&chapter))
            .await.map_err(|e|ResourceError::new(e.to_string()))?
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub(crate) async fn local_import_guard(&self) -> OwnedMutexGuard<()> {
        self.inner.local_import_lock.clone().lock_owned().await
    }

    /// Coordinate immutable file publication and cache cleanup. SQLite state
    /// updates use their own transactions and do not take this filesystem guard.
    /// Callers retain the relevant book/chapter guard while preparing metadata.
    pub(crate) async fn transaction_writer_guard(
        &self,
    ) -> Result<ResourceStoreWriterGuard, ResourceError> {
        let guard = Arc::clone(&self.inner.asset_lock).lock_owned().await;
        ensure_resource_writes_recovered(&self.inner.root)?;
        Ok(ResourceStoreWriterGuard {
            store: self.clone(),
            _guard: guard,
        })
    }

    pub fn shelf_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path(super::content::LIBRARY_FILE)
    }

    pub fn settings_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("settings.json")
    }

    pub fn http_tts_configs_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("http-tts.json")
    }

    pub fn source_list_ref(&self) -> Result<ResourceRef, ResourceError> {
        self.document_ref("source-list")
    }

    pub fn history_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("history.json")
    }

    pub fn book_ref(&self, book_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        let kind = self.book_type(book_id)?;
        ResourceRef::new(format!("resource://cache/{kind}/{book_id}/{book_id}.json"))
    }

    pub(crate) fn book_type(&self, book_id: &str) -> Result<String, ResourceError> {
        validate_id(book_id)?;
        if let Some(kind) = self
            .inner
            .content_types
            .read()
            .map_err(|_| ResourceError::new("Content type lock poisoned"))?
            .get(book_id)
        {
            return Ok(kind.clone());
        }
        if let Some(dir) = super::content::find_content_dir(self.root(), book_id)? {
            let kind = dir
                .parent()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .ok_or_else(|| ResourceError::new("Invalid content directory"))?
                .to_owned();
            self.inner
                .content_types
                .write()
                .map_err(|_| ResourceError::new("Content type lock poisoned"))?
                .insert(book_id.to_owned(), kind.clone());
            return Ok(kind);
        }
        Ok(if book_id.starts_with("rss-") {
            "html"
        } else if book_id == "reader-theme-backgrounds" {
            "image"
        } else if book_id == "tts-audio" {
            "audio"
        } else {
            "text"
        }
        .to_owned())
    }

    pub(crate) async fn bind_book_type(
        &self,
        book_id: &str,
        kind: &str,
    ) -> Result<(), ResourceError> {
        let _writer = self.transaction_writer_guard().await?;
        validate_id(book_id)?;
        let kind = super::content::canonical_type(kind);
        if !super::content::CONTENT_TYPES.contains(&kind) {
            return Err(ResourceError::new("Unsupported content type"));
        }
        let current = self.book_type(book_id)?;
        if current != kind {
            let old = self.root().join("cache").join(&current).join(book_id);
            let target = self.root().join("cache").join(kind).join(book_id);
            check_path_no_symlink(self.root(), &old, true)?;
            check_path_no_symlink(self.root(), &target, true)?;
            let old_path = format!("cache/{current}/{book_id}/{book_id}.json");
            let new_path = format!("cache/{kind}/{book_id}/{book_id}.json");
            if let Some(mut document) = self.database().read(&old_path)? {
                if self.database().exists(&new_path)? {
                    return Err(ResourceError::new(
                        "Content already exists in the target type",
                    ));
                }
                if old.is_dir() {
                    std::fs::create_dir_all(target.parent().expect("parent"))?;
                    if target.exists() {
                        return Err(ResourceError::new(
                            "Content assets already exist in the target type",
                        ));
                    }
                    std::fs::rename(&old, &target)?;
                }
                if current == "text" && kind == "html" {
                    for chapter in document["chapters"].as_array_mut().into_iter().flatten() {
                        if let Some(name) = chapter["url"]
                            .as_str()
                            .filter(|name| !name.contains('/') && name.ends_with(".txt"))
                            .map(str::to_owned)
                        {
                            let from = target.join(&name);
                            check_path_no_symlink(self.root(), &from, true)?;
                            if from.is_file() {
                                let text = std::fs::read_to_string(&from)?;
                                let name = format!("{}.html", name.trim_end_matches(".txt"));
                                let to = target.join(&name);
                                check_path_no_symlink(self.root(), &to, true)?;
                                atomic_replace(
                                    &to,
                                    format!("<pre>{}</pre>", quick_xml::escape::escape(&text))
                                        .as_bytes(),
                                )
                                .map_err(ResourceError::new)?;
                                chapter["url"] = json!(name);
                                std::fs::remove_file(from)?;
                            }
                        }
                    }
                }
                self.database().transaction(move |tx| {
                    crate::database::write(tx, &new_path, &document)?;
                    Ok(())
                })?;
            }
        }
        self.inner
            .content_types
            .write()
            .map_err(|_| ResourceError::new("Content type lock poisoned"))?
            .insert(book_id.to_owned(), kind.to_owned());
        Ok(())
    }

    /// Versioned chapter bodies are addressed by identity and content.
    /// Changing a source chapter does not silently retarget older descriptors.
    pub(crate) fn chapter_version_ref(
        &self,
        book_id: &str,
        chapter_id: &str,
        document: &Value,
    ) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        validate_id(chapter_id)?;
        let serialized = serde_json::to_vec(document).map_err(|error| {
            ResourceError::new(format!("Cannot encode chapter payload: {error}"))
        })?;
        let chapter_hash = Sha256::digest(chapter_id.as_bytes());
        let content_hash = Sha256::digest(&serialized);
        let kind = self.book_type(book_id)?;
        let extension = match document["kind"].as_str() {
            Some("text") => "txt",
            Some("richText") => "html",
            _ => {
                return Err(ResourceError::new(
                    "Chapter bodies must be native text or HTML files",
                ))
            }
        };
        ResourceRef::new(format!(
            "resource://cache/{kind}/{book_id}/{chapter_hash:x}-{content_hash:x}.{extension}"
        ))
    }

    pub fn document_ref(&self, document_name: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(document_name)?;
        ResourceRef::new(format!("resource://{document_name}.json"))
    }

    pub fn discovery_ref(&self, discovery_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(discovery_id)?;
        ResourceRef::new(format!("resource://discovery/{discovery_id}.json"))
    }

    pub fn dictionary_ref(&self, dictionary_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(dictionary_id)?;
        ResourceRef::new(format!("resource://dictionary/{dictionary_id}.html"))
    }

    pub fn discovery_favorites_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("discovery-favorites.json")
    }

    pub fn asset_ref(&self, book_id: &str, asset_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        validate_asset_id(asset_id)?;
        let kind = self.book_type(book_id)?;
        ResourceRef::new(format!("resource://cache/{kind}/{book_id}/{asset_id}"))
    }

    pub async fn write_asset(
        &self,
        book_id: &str,
        asset_id: &str,
        bytes: &[u8],
    ) -> Result<ResourceRef, ResourceError> {
        let reference = self.asset_ref(book_id, asset_id)?;
        let root = self.inner.root.clone();
        let bytes = bytes.to_vec();
        let _guard = self.inner.asset_lock.lock().await;
        let write_path = self.path_for(&reference)?;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            check_path_no_symlink(&root, &write_path, true)?;
            atomic_replace(&write_path, &bytes).map_err(|error| {
                ResourceError::new(format!("Cannot atomically write resource asset: {error}"))
            })
        })
        .await
        .map_err(|error| ResourceError::new(format!("Asset storage worker failed: {error}")))??;
        Ok(reference)
    }

    /// Store one bounded HTTP TTS response as a private-lifetime local media file.
    /// The reference namespace is deliberately limited to `cache/audio/tts-audio/`.
    pub async fn write_tts_audio(
        &self,
        audio_id: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<ResourceRef, ResourceError> {
        if bytes.is_empty() || bytes.len() > MAX_TTS_AUDIO_BYTES {
            return Err(ResourceError::new(
                "HTTP TTS audio is empty or exceeds 16 MiB",
            ));
        }
        let reference = tts_audio_ref(audio_id, extension)?;
        let path = self.path_for(&reference)?;
        let root = self.inner.root.clone();
        let bytes = bytes.to_vec();
        let _guard = self.inner.asset_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            check_path_no_symlink(&root, &path, true)?;
            let directory = path
                .parent()
                .ok_or_else(|| ResourceError::new("HTTP TTS resource has no parent directory"))?;
            if directory.exists() {
                let count = std::fs::read_dir(directory)
                    .map_err(|error| {
                        ResourceError::new(format!("Cannot inspect HTTP TTS cache: {error}"))
                    })?
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                    .count();
                if count >= MAX_TTS_AUDIO_FILES {
                    return Err(ResourceError::new(
                        "Too many temporary HTTP TTS audio files; release previous audio first",
                    ));
                }
            }
            atomic_replace(&path, &bytes).map_err(|error| {
                ResourceError::new(format!("Cannot store HTTP TTS audio: {error}"))
            })
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("HTTP TTS storage worker failed: {error}"))
        })??;
        Ok(reference)
    }

    pub(crate) async fn read_tts_audio(
        &self,
        audio_id: &str,
        extension: &str,
    ) -> Result<Vec<u8>, ResourceError> {
        let reference = tts_audio_ref(audio_id, extension)?;
        let path = self.path_for(&reference)?;
        check_path_no_symlink(&self.inner.root, &path, false)?;
        let bytes = tokio::fs::read(path).await?;
        if bytes.is_empty() || bytes.len() > MAX_TTS_AUDIO_BYTES {
            return Err(ResourceError::new(
                "HTTP TTS audio is empty or exceeds 16 MiB",
            ));
        }
        Ok(bytes)
    }

    pub(crate) async fn remove_tts_audio(&self, audio_id: &str) -> Result<bool, ResourceError> {
        // Validate the opaque ID before deriving any local path.
        let _ = tts_audio_ref(audio_id, "mp3")?;
        let writer = self.transaction_writer_guard().await?;
        let mut removed = false;
        for extension in ["mp3", "m4a", "aac", "wav", "ogg", "opus", "flac"] {
            let reference = tts_audio_ref(audio_id, extension)?;
            removed |= writer.remove_asset(&reference)?;
        }
        Ok(removed)
    }

    pub(crate) async fn regular_file_exists(
        &self,
        reference: &ResourceRef,
    ) -> Result<bool, ResourceError> {
        if reference.path().ends_with(".json") {
            return self
                .database()
                .exists(self.canonical_reference(reference)?.path());
        }
        let path = self.path_for(reference)?;
        check_path_no_symlink(&self.inner.root, &path, true)?;
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.file_type().is_file() => Ok(true),
            Ok(metadata) if metadata.file_type().is_symlink() => Err(ResourceError::new(
                "Refusing to use a symbolic link as a resource file",
            )),
            Ok(_) => Err(ResourceError::new("Resource path is not a regular file")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(ResourceError::new(format!(
                "Cannot inspect resource path: {error}"
            ))),
        }
    }

    pub async fn read_json_ref(&self, reference: &ResourceRef) -> Result<Value, ResourceError> {
        if reference.path().ends_with(".json") {
            let store = self.clone();
            let reference = self.canonical_reference(reference)?;
            return tokio::task::spawn_blocking(move || {
                let value = store.database().read(reference.path())?.ok_or_else(|| {
                    ResourceError::new(format!("Resource does not exist: {}", reference.path()))
                })?;
                store.document_view(&reference, value)
            })
            .await
            .map_err(|error| ResourceError::new(error.to_string()))?;
        }
        let path = self.path_for(reference)?;
        check_path_no_symlink(self.root(), &path, false)?;
        let text = tokio::fs::read_to_string(path).await?;
        Ok(if reference.path().ends_with(".html") {
            json!({"kind":"richText","markup":text})
        } else {
            json!({"kind":"text","text":text})
        })
    }

    pub(crate) fn document_view(
        &self,
        reference: &ResourceRef,
        value: Value,
    ) -> Result<Value, ResourceError> {
        if reference.path() == super::content::LIBRARY_FILE {
            return Ok(super::content::library_positions(&value));
        }
        if let Some((_, id, _)) = super::content::content_path(reference.path())
            .filter(|_| super::content::is_content_document(reference.path()))
        {
            return super::content::book_view(&value, &self.book_type(id)?);
        }
        Ok(value)
    }

    pub(crate) fn read_raw_document(
        &self,
        reference: &ResourceRef,
    ) -> Result<Value, ResourceError> {
        let reference = self.canonical_reference(reference)?;
        Ok(self
            .database()
            .read(reference.path())?
            .unwrap_or_else(|| json!({})))
    }

    pub(crate) fn encode_document(
        &self,
        reference: &ResourceRef,
        value: &Value,
        state: Option<&Value>,
    ) -> Result<Value, ResourceError> {
        if reference.path() == super::content::LIBRARY_FILE {
            return super::content::encode_library(value, &self.database().connect()?);
        }
        if let Some((_, id, _)) = super::content::content_path(reference.path())
            .filter(|_| super::content::is_content_document(reference.path()))
        {
            return super::content::encode_book(
                value,
                state,
                &self.read_raw_document(reference)?,
                &self.book_type(id)?,
            );
        }
        Ok(value.clone())
    }

    fn refresh_database_preview(
        &self,
        tx: &rusqlite::Connection,
        reference: &ResourceRef,
        _document: &Value,
    ) -> Result<(), ResourceError> {
        if !super::content::is_content_document(reference.path()) {
            return Ok(());
        }
        let _ = tx; // Shelf previews are joins over books and progress, never stored copies.
        Ok(())
    }

    /// Read app-owned state, initializing a missing database document from
    /// the caller's default. Database corruption remains an explicit error.
    pub async fn read_json_ref_or_default(
        &self,
        reference: &ResourceRef,
        default: Value,
    ) -> Result<Value, ResourceError> {
        let store = self.clone();
        let reference = self.canonical_reference(reference)?;
        tokio::task::spawn_blocking(move || {
            let value = match store.database().read(reference.path())? {
                Some(value) => value,
                None => { store.database().write(reference.path(), &default)?; default }
            };
            store.document_view(&reference, value)
        })
        .await
        .map_err(|error| ResourceError::new(error.to_string()))?
    }

    /// Atomically replace a public JSON document. Stable references embedded in
    /// it remain stable on disk and are expanded only by an HTTP GET response.
    pub async fn write_json_ref(
        &self,
        reference: &ResourceRef,
        value: &Value,
    ) -> Result<(), ResourceError> {
        if !reference.path().ends_with(".json") {
            let body = value["text"]
                .as_str()
                .or_else(|| value["markup"].as_str())
                .ok_or_else(|| ResourceError::new("Native content has no body"))?;
            let _writer = self.transaction_writer_guard().await?;
            let path = self.path_for(reference)?;
            check_path_no_symlink(self.root(), &path, true)?;
            atomic_replace(&path, body.as_bytes())
                .map_err(|error| ResourceError::new(error.to_string()))?;
            return Ok(());
        }
        // Source definitions are opaque Legado input: a nested `src` can be a
        // parsing rule rather than an application resource reference.
        if reference.path() != "sources.json" {
            validate_embedded_refs(value)?;
        }
        if crate::configuration::is_file(reference.path()) {
            let database=self.database().clone();
            let path=reference.path().to_owned();
            let value=value.clone();
            return tokio::task::spawn_blocking(move ||database.write(&path,&value))
                .await.map_err(|e|ResourceError::new(e.to_string()))?;
        }
        let store = self.clone();
        let reference = self.canonical_reference(reference)?;
        let value = value.clone();
        tokio::task::spawn_blocking(move || {
            let database = store.database().clone();
            database.transaction(move |tx| {
                let encoded = if reference.path() == super::content::LIBRARY_FILE {
                    super::content::encode_library(&value, tx)?
                } else {
                    store.encode_document(&reference, &value, None)?
                };
                crate::database::write(tx, reference.path(), &encoded)?;
                store.refresh_database_preview(tx, &reference, &encoded)
            })
        })
        .await
        .map_err(|error| ResourceError::new(error.to_string()))?
    }

    /// Read and modify state in one SQLite transaction. Filesystem operations
    /// use a separate guard; database updates do not acquire that mutex.
    pub async fn update_json_ref<F>(
        &self,
        reference: &ResourceRef,
        update: F,
    ) -> Result<Value, ResourceError>
    where
        F: FnOnce(Value) -> Result<Value, String> + Send + 'static,
    {
        self.update_json_ref_inner(reference, None, update).await
    }

    /// Update app-owned state in a transaction, initializing a missing
    /// document from the supplied default.
    pub async fn update_json_ref_or_default<F>(
        &self,
        reference: &ResourceRef,
        default: Value,
        update: F,
    ) -> Result<Value, ResourceError>
    where
        F: FnOnce(Value) -> Result<Value, String> + Send + 'static,
    {
        self.update_json_ref_inner(reference, Some(default), update)
            .await
    }

    async fn update_json_ref_inner<F>(
        &self,
        reference: &ResourceRef,
        default: Option<Value>,
        update: F,
    ) -> Result<Value, ResourceError>
    where
        F: FnOnce(Value) -> Result<Value, String> + Send + 'static,
    {
        if !reference.is_local() || !reference.path().ends_with(".json") {
            return Err(ResourceError::new(
                "JSON update requires a local .json resource",
            ));
        }
        if crate::configuration::is_file(reference.path()) {
            let root = self.root().to_owned();
            let path = reference.path().to_owned();
            return tokio::task::spawn_blocking(move || {
                crate::configuration::update(&root,&path,default.unwrap_or(Value::Null),move |value| {
                    let value=update(value).map_err(ResourceError::new)?;
                    validate_embedded_refs(&value)?;
                    Ok(value)
                })
            }).await.map_err(|e|ResourceError::new(e.to_string()))?;
        }
        let store = self.clone();
        let reference = self.canonical_reference(reference)?;
        tokio::task::spawn_blocking(move || {
            let database = store.database().clone();
            database.transaction(move |tx| {
                let current = crate::database::read(tx, reference.path())?
                    .unwrap_or_else(|| default.unwrap_or(Value::Null));
                let current = store.document_view(&reference, current)?;
                let updated = update(current).map_err(ResourceError::new)?;
                validate_embedded_refs(&updated)?;
                let encoded = if reference.path() == super::content::LIBRARY_FILE {
                    super::content::encode_library(&updated, tx)?
                } else {
                    store.encode_document(&reference, &updated, None)?
                };
                crate::database::write(tx, reference.path(), &encoded)?;
                store.refresh_database_preview(tx, &reference, &encoded)?;
                Ok(updated)
            })
        })
        .await
        .map_err(|error| ResourceError::new(error.to_string()))?
    }

    /// Serialize and atomically write a typed JSON document at a stable public ref.
    pub async fn write_serializable<T: Serialize + ?Sized>(
        &self,
        reference: &ResourceRef,
        value: &T,
    ) -> Result<(), ResourceError> {
        let value = serde_json::to_value(value)
            .map_err(|error| ResourceError::new(format!("Cannot encode JSON resource: {error}")))?;
        self.write_json_ref(reference, &value).await
    }

    pub async fn write_shelf(&self, shelf: &ShelfDocument) -> Result<(), ResourceError> {
        let database=self.database().clone();
        let shelf=shelf.clone();
        tokio::task::spawn_blocking(move ||database.save_library_positions(shelf))
            .await.map_err(|e|ResourceError::new(e.to_string()))?
    }

    pub async fn write_settings(&self, settings: &SettingsDocument) -> Result<(), ResourceError> {
        self.write_serializable(&self.settings_ref(), settings)
            .await
    }

    pub async fn write_book(&self, book: &BookDocument) -> Result<(), ResourceError> {
        if let Some(kind) = book.content_type.as_deref() {
            self.bind_book_type(&book.id, kind).await?;
        }
        let reference = self.book_ref(&book.id)?;
        self.write_serializable(&reference, book).await
    }

    pub(crate) async fn write_book_state(
        &self,
        book_id: &str,
        view: &Value,
        state: &Value,
    ) -> Result<(), ResourceError> {
        let store = self.clone();
        let reference = self.book_ref(book_id)?;
        let view = view.clone();
        let state = state.clone();
        tokio::task::spawn_blocking(move || {
            let database = store.database().clone();
            database.transaction(move |tx| {
                let encoded = store.encode_document(&reference, &view, Some(&state))?;
                crate::database::write(tx, reference.path(), &encoded)?;
                store.refresh_database_preview(tx, &reference, &encoded)
            })
        })
        .await
        .map_err(|error| ResourceError::new(error.to_string()))?
    }

    /// Store engine-processed plain text as data. Rendering belongs to Vue.
    pub async fn write_chapter_text(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_text: &str,
    ) -> Result<ResourceRef, ResourceError> {
        let (reference, document) =
            self.chapter_text_document(book_id, chapter_id, processed_text)?;
        self.write_json_ref(&reference, &document).await?;
        Ok(reference)
    }

    /// Store engine-processed rich text as typed data. Rust strips executable
    /// markup, while Vue owns the final display document and reader styling.
    pub async fn write_chapter_rich_text(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_html: &str,
        content_base_url: Option<&str>,
    ) -> Result<ResourceRef, ResourceError> {
        let (reference, document) = self.online_chapter_rich_text_document(
            book_id,
            chapter_id,
            processed_html,
            content_base_url,
        )?;
        self.write_json_ref(&reference, &document).await?;
        Ok(reference)
    }

    /// Local EPUB parsing rewrites archive assets to ../assets/<id>. Convert
    /// that importer-private convention into stable resource refs before
    /// publishing the typed rich-text document. The WebView must never depend
    /// on the chapter JSON directory layout to resolve these assets.
    pub async fn write_local_chapter_rich_text(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_html: &str,
    ) -> Result<ResourceRef, ResourceError> {
        let (_, mut document) =
            self.chapter_rich_text_document(book_id, chapter_id, processed_html)?;
        let markup = document
            .get("markup")
            .and_then(Value::as_str)
            .ok_or_else(|| ResourceError::new("Rich-text document has no markup"))?;
        let stable_markup = Self::local_epub_asset_refs(book_id, markup)?;
        document["markup"] = json!(stable_markup);
        let reference = self.chapter_version_ref(book_id, chapter_id, &document)?;
        self.write_json_ref(&reference, &document).await?;
        Ok(reference)
    }

    pub(crate) fn chapter_text_document(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_text: &str,
    ) -> Result<(ResourceRef, Value), ResourceError> {
        if self.book_type(book_id)? == "html" {
            let markup = format!("<pre>{}</pre>", quick_xml::escape::escape(processed_text));
            return self.chapter_rich_text_document(book_id, chapter_id, &markup);
        }
        let document = serde_json::json!({
            "schemaVersion": crate::models::CURRENT_SCHEMA_VERSION,
            "kind": "text",
            "text": processed_text,
        });
        let reference = self.chapter_version_ref(book_id, chapter_id, &document)?;
        Ok((reference, document))
    }

    /// Validate resolved media. Its index belongs in the complete content document.
    pub(crate) async fn write_chapter_media(
        &self,
        book_id: &str,
        chapter_id: &str,
        media_ref: &ResourceRef,
        media_type: &str,
        media_format: &str,
        video_sources: &[ChapterVideoSource],
    ) -> Result<ResourceRef, ResourceError> {
        let (reference, document) = self.chapter_media_document(
            book_id,
            chapter_id,
            media_ref,
            media_type,
            media_format,
            video_sources,
        )?;
        let _ = document;
        Ok(reference)
    }

    pub(crate) fn chapter_media_document(
        &self,
        book_id: &str,
        chapter_id: &str,
        media_ref: &ResourceRef,
        media_type: &str,
        media_format: &str,
        video_sources: &[ChapterVideoSource],
    ) -> Result<(ResourceRef, Value), ResourceError> {
        if !matches!(media_type, "audio" | "video") {
            return Err(ResourceError::new("Unsupported chapter media type"));
        }
        if !matches!(media_format, "direct" | "hls") {
            return Err(ResourceError::new("Unsupported chapter media format"));
        }
        let media_id = media_ref
            .path()
            .strip_prefix("media/")
            .filter(|_| media_ref.is_local())
            .ok_or_else(|| ResourceError::new("Media chapter requires a registered media ref"))?;
        validate_id(media_id)?;
        if media_type == "audio" && !video_sources.is_empty() {
            return Err(ResourceError::new(
                "Audio chapters cannot contain video resolution sources",
            ));
        }

        let mut source_documents = Vec::new();
        if media_type == "video" {
            let mut selected_count = 0usize;
            let mut seen_ids = Vec::new();
            for source in video_sources {
                let source_ref = if let Some(reference) = &source.media_ref {
                    let id = reference
                        .path()
                        .strip_prefix("media/")
                        .filter(|_| reference.is_local())
                        .ok_or_else(|| {
                            ResourceError::new("Video resolution requires a registered media ref")
                        })?;
                    validate_id(id)?;
                    if seen_ids.iter().any(|previous| *previous == id) {
                        return Err(ResourceError::new(
                            "Video resolution media refs must be unique",
                        ));
                    }
                    seen_ids.push(id);
                    if source.selected && reference.as_str() != media_ref.as_str() {
                        return Err(ResourceError::new(
                            "Selected video resolution must match the chapter media ref",
                        ));
                    }
                    Some(reference.as_str())
                } else {
                    None
                };
                if source_ref.is_none() == source.unavailable_reason.is_none() {
                    return Err(ResourceError::new(
                        "Video resolution must be playable or have an unavailable reason",
                    ));
                }
                if source_ref.is_some() != source.media_format.is_some() {
                    return Err(ResourceError::new(
                        "Playable video resolutions require a media format",
                    ));
                }
                if source
                    .media_format
                    .is_some_and(|format| !matches!(format, "direct" | "hls"))
                {
                    return Err(ResourceError::new("Unsupported video resolution format"));
                }
                if source.selected {
                    selected_count += 1;
                    if source_ref.is_none() {
                        return Err(ResourceError::new(
                            "Unavailable video resolutions cannot be selected",
                        ));
                    }
                    if source.media_format != Some(media_format) {
                        return Err(ResourceError::new(
                            "Selected video resolution format must match the chapter media format",
                        ));
                    }
                }
                source_documents.push(json!({
                    "src": source_ref,
                    "label": source.label,
                    "format": source.media_format,
                    "selected": source.selected,
                    "unavailableReason": source.unavailable_reason,
                }));
            }
            if !video_sources.is_empty() && selected_count != 1 {
                return Err(ResourceError::new(
                    "Video resolution list must contain exactly one selected source",
                ));
            }
        }

        let document = json!({
            "schemaVersion": CURRENT_SCHEMA_VERSION,
            "kind": "media",
            "mediaType": media_type,
            "src": media_ref.as_str(),
            "format": media_format,
            "sources": source_documents,
        });
        let _ = (book_id, chapter_id);
        Ok((media_ref.clone(), document))
    }

    /// Write one processed Wiktionary entry as a short-lived reader resource.
    /// This uses a dedicated namespace so dictionary lookups never enter a
    /// book's chapter cache or masquerade as RSS content.
    pub async fn write_dictionary_html(
        &self,
        dictionary_id: &str,
        processed_html: &str,
        source_url: &str,
        defaults: &ReaderDefaults,
    ) -> Result<ResourceRef, ResourceError> {
        let reference = self.dictionary_ref(dictionary_id)?;
        let source = reqwest::Url::parse(source_url)
            .map_err(|_| ResourceError::new("Dictionary source URL is invalid"))?;
        if source.scheme() != "https"
            || !matches!(
                source.host_str(),
                Some("en.wiktionary.org" | "zh.wiktionary.org")
            )
            || !source.username().is_empty()
            || source.password().is_some()
        {
            return Err(ResourceError::new(
                "Dictionary source must be a credential-free HTTPS Wiktionary URL",
            ));
        }
        let mut sanitizer = HtmlSanitizer::default();
        sanitizer
            .add_tags(&["link"])
            .add_tag_attributes("link", &["href", "rel", "type", "media"])
            .add_tag_attribute_values("link", "rel", &["stylesheet"])
            .url_relative(ammonia::UrlRelative::RewriteWithBase(source.clone()));
        let sanitized = sanitizer.clean(processed_html).to_string();
        let document = chapter_document(&sanitized, defaults);
        self.write_html_resource(&reference, &document).await?;
        Ok(reference)
    }

    pub(crate) fn online_chapter_rich_text_document(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_html: &str,
        content_base_url: Option<&str>,
    ) -> Result<(ResourceRef, Value), ResourceError> {
        let mut sanitizer = HtmlSanitizer::default();
        sanitizer
            .add_tags(&["link"])
            .add_tag_attributes("link", &["href", "rel", "type", "media"])
            .add_tag_attribute_values("link", "rel", &["stylesheet"])
            .add_tag_attributes("a", &["data-legado-chapter-id", "data-legado-fragment"]);

        let base_url = content_base_url
            .and_then(|value| reqwest::Url::parse(value).ok())
            .filter(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
            });
        if let Some(base_url) = base_url {
            sanitizer.url_relative(ammonia::UrlRelative::RewriteWithBase(base_url));
        } else {
            // Typed online rich text must never inherit a WebView/localhost
            // base URL. Without a trustworthy source URL, drop relative links
            // and assets instead of publishing ambiguous resource semantics.
            sanitizer.url_relative(ammonia::UrlRelative::Deny);
        }

        let markup = sanitizer.clean(processed_html).to_string();
        let document = json!({
            "schemaVersion": CURRENT_SCHEMA_VERSION,
            "kind": "richText",
            "markup": markup,
        });
        let reference = self.chapter_version_ref(book_id, chapter_id, &document)?;
        Ok((reference, document))
    }

    pub(crate) fn chapter_rich_text_document(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_html: &str,
    ) -> Result<(ResourceRef, Value), ResourceError> {
        let mut sanitizer = HtmlSanitizer::default();
        sanitizer
            .add_tags(&["link"])
            .add_tag_attributes("link", &["href", "rel", "type", "media"])
            .add_tag_attribute_values("link", "rel", &["stylesheet"])
            .add_tag_attributes("a", &["data-legado-chapter-id", "data-legado-fragment"])
            .url_relative(ammonia::UrlRelative::PassThrough);
        let markup = sanitizer.clean(processed_html).to_string();
        let document = json!({
            "schemaVersion": CURRENT_SCHEMA_VERSION,
            "kind": "richText",
            "markup": markup,
        });
        let reference = self.chapter_version_ref(book_id, chapter_id, &document)?;
        Ok((reference, document))
    }

    /// Validate the shared PDF entry. Page descriptors are generated in memory.
    pub async fn write_pdf_page(
        &self,
        book_id: &str,
        chapter_id: &str,
        pdf_asset_id: &str,
        page_index: u32,
        default_zoom: &str,
    ) -> Result<ResourceRef, ResourceError> {
        validate_asset_id(pdf_asset_id)?;
        if !pdf_asset_id.to_ascii_lowercase().ends_with(".pdf") {
            return Err(ResourceError::new("PDF page must refer to a PDF asset"));
        }
        if !matches!(default_zoom, "page-fit" | "page-width" | "actual-size") {
            return Err(ResourceError::new("Unsupported default PDF zoom mode"));
        }
        let pdf_ref = self.asset_ref(book_id, pdf_asset_id)?;
        let pdf_path = self.path_for(&pdf_ref)?;
        check_path_no_symlink(&self.inner.root, &pdf_path, false)?;
        let metadata = tokio::fs::metadata(&pdf_path)
            .await
            .map_err(|error| ResourceError::new(format!("Cannot inspect PDF asset: {error}")))?;
        if !metadata.is_file() {
            return Err(ResourceError::new("PDF asset is not a regular file"));
        }

        let _ = (chapter_id, page_index);
        Ok(pdf_ref)
    }

    fn local_epub_asset_refs(book_id: &str, markup: &str) -> Result<String, ResourceError> {
        validate_id(book_id)?;
        let base = format!("resource://cache/html/{book_id}/");
        let mut output = markup.to_owned();
        for attribute in ["src", "href", "poster"] {
            for quote in ['"', '\''] {
                let from = format!("{attribute}={quote}../assets/");
                let to = format!("{attribute}={quote}{base}");
                output = output.replace(&from, &to);
            }
        }
        Ok(output)
    }

    async fn write_html_resource(
        &self,
        reference: &ResourceRef,
        html: &str,
    ) -> Result<(), ResourceError> {
        let path = self.path_for(reference)?;
        let bytes = html.as_bytes().to_vec();
        let _guard = self.inner.asset_lock.lock().await;
        let root = self.inner.root.clone();
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            check_path_no_symlink(&root, &path, true)?;
            atomic_replace(&path, &bytes).map_err(|error| {
                ResourceError::new(format!("Cannot atomically cache chapter HTML: {error}"))
            })
        })
        .await
        .map_err(|error| ResourceError::new(format!("HTML storage worker failed: {error}")))??;
        Ok(())
    }

    /// Copy a selected local book into the app-managed book directory.
    /// Originals are retained beside the parsed cache. PDF readers use the
    /// original directly; the other readers consume extracted chapter assets.
    pub async fn persist_local_original(
        &self,
        book_id: &str,
        extension: &str,
        source_path: &Path,
    ) -> Result<(), ResourceError> {
        validate_id(book_id)?;
        if !matches!(extension, "txt" | "epub" | "cbz" | "pdf") {
            return Err(ResourceError::new("Unsupported local original format"));
        }
        let extension = String::from(extension);
        let source_metadata = tokio::fs::symlink_metadata(source_path)
            .await
            .map_err(|error| {
                ResourceError::new(format!("Cannot inspect managed import: {error}"))
            })?;
        if !source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
            return Err(ResourceError::new(
                "Managed local original must be a regular file",
            ));
        }

        let book_dir = self
            .inner
            .root
            .join("cache")
            .join(self.book_type(book_id)?)
            .join(book_id);
        let target = book_dir.join(format!("original.{extension}"));
        let root = self.inner.root.clone();
        let source_path = source_path.to_path_buf();
        let _guard = self.inner.asset_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            check_path_no_symlink(&root, &book_dir, true)?;
            std::fs::create_dir_all(&book_dir).map_err(|error| {
                ResourceError::new(format!("Cannot prepare local original directory: {error}"))
            })?;
            check_path_no_symlink(&root, &target, true)?;
            if let Ok(metadata) = std::fs::symlink_metadata(&target) {
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
                    return Ok(());
                }
                return Err(ResourceError::new(
                    "Local original target is not a regular file",
                ));
            }

            AtomicFile::new(&target, AllowOverwrite)
                .write(|output| {
                    let mut input = std::fs::File::open(&source_path)?;
                    std::io::copy(&mut input, output)?;
                    output.sync_all()
                })
                .map_err(|error| {
                    ResourceError::new(format!("Cannot atomically store local original: {error}"))
                })
        })
        .await
        .map_err(|error| ResourceError::new(format!("Local original worker failed: {error}")))??;
        Ok(())
    }

    /// Delete one book's public cache. The caller updates the shelf JSON as
    /// part of the same application operation.
    pub async fn remove_book_resources(&self, book_id: &str) -> Result<(), ResourceError> {
        validate_id(book_id)?;
        let directories = super::content::CONTENT_TYPES
            .iter()
            .map(|kind| self.inner.root.join("cache").join(kind).join(book_id))
            .collect::<Vec<_>>();
        let root = self.inner.root.clone();
        let database = self.database().clone();
        let book_id = book_id.to_owned();
        let _guard = self.inner.asset_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            for kind in super::content::CONTENT_TYPES {
                database.delete_prefix(&format!("cache/{kind}/{book_id}/"))?;
            }
            for book_dir in directories {
                check_path_no_symlink(&root, &book_dir, true)?;
                match std::fs::remove_dir_all(&book_dir) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(ResourceError::new(format!(
                            "Cannot remove book resources: {error}"
                        )));
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|error| ResourceError::new(format!("Book deletion worker failed: {error}")))??;
        Ok(())
    }

    /// Remove only cached RSS typed article bodies and media for a source's private
    /// pseudo-book namespace. Shelf and ordinary book resources are
    /// outside this operation and remain untouched.
    pub async fn remove_rss_chapter_resources(&self, source_id: &str) -> Result<(), ResourceError> {
        validate_id(source_id)?;
        let rss_book_id = format!("rss-{source_id}");
        validate_id(&rss_book_id)?;
        let book_dir = self
            .inner
            .root
            .join("cache")
            .join("html")
            .join(&rss_book_id);
        let directories = [book_dir];
        let root = self.inner.root.clone();
        let database = self.database().clone();
        let _guard = self.inner.asset_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            database.delete_prefix(&format!("cache/html/{rss_book_id}/"))?;
            for directory in directories {
                check_path_no_symlink(&root, &directory, true)?;
                match std::fs::remove_dir_all(&directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(ResourceError::new(format!(
                            "Cannot remove cached RSS resources: {error}"
                        )));
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|error| ResourceError::new(format!("RSS cleanup worker failed: {error}")))??;
        Ok(())
    }

    pub(crate) fn canonical_reference(
        &self,
        reference: &ResourceRef,
    ) -> Result<ResourceRef, ResourceError> {
        if let Some((_, id, name)) = super::content::content_path(reference.path()) {
            let kind = self.book_type(id)?;
            return ResourceRef::new(format!("resource://cache/{kind}/{id}/{name}"));
        }
        Ok(reference.clone())
    }

    pub(crate) fn reset_content_types(&self) {
        if let Ok(mut types) = self.inner.content_types.write() {
            types.clear();
        }
    }

    pub(super) fn path_for(&self, reference: &ResourceRef) -> Result<PathBuf, ResourceError> {
        // Revalidate even though ResourceRef itself is validated: deserialized
        // data and future constructors should never weaken the filesystem guard.
        if !reference.is_local() {
            return Err(ResourceError::new(
                "External HTTP(S) resource URLs do not map to local files",
            ));
        }
        validate_public_path(reference.path())?;
        Ok(self
            .inner
            .root
            .join(self.canonical_reference(reference)?.path()))
    }
}

/// An owned filesystem permit retained while publishing or removing assets.
/// Metadata writes are committed together by SQLite.
pub(crate) struct ResourceStoreWriterGuard {
    store: ResourceStore,
    _guard: OwnedMutexGuard<()>,
}

impl crate::resource_transactions::ResourceWriterGuard for ResourceStoreWriterGuard {
    fn data_root(&self) -> &Path {
        self.store.root()
    }
}

impl ResourceStoreWriterGuard {
    pub(crate) fn data_root(&self) -> &Path {
        self.store.root()
    }

    pub(crate) fn asset_exists(&self, reference: &ResourceRef) -> Result<bool, ResourceError> {
        if reference.path().ends_with(".json") {
            return self
                .store
                .database()
                .exists(self.store.canonical_reference(reference)?.path());
        }
        let path = self.store.path_for(reference)?;
        check_path_no_symlink(self.store.root(), &path, true)?;
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => Ok(true),
            Ok(_) => Err(ResourceError::new("Resource asset is not a regular file")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(ResourceError::new(format!(
                "Cannot inspect resource asset: {error}"
            ))),
        }
    }

    /// Remove one already-validated asset while retaining this store-wide
    /// writer lock. Callers must check public references before deleting it.
    pub(crate) fn remove_asset(&self, reference: &ResourceRef) -> Result<bool, ResourceError> {
        let path = self.store.path_for(reference)?;
        check_path_no_symlink(self.store.root(), &path, true)?;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(ResourceError::new(format!(
                    "Cannot inspect resource asset: {error}"
                )));
            }
        };
        if !metadata.file_type().is_file() {
            return Err(ResourceError::new("Resource asset is not a regular file"));
        }
        std::fs::remove_file(&path).map_err(|error| {
            ResourceError::new(format!("Cannot remove resource asset: {error}"))
        })?;
        Ok(true)
    }

    pub(crate) fn read_json_ref(&self, reference: &ResourceRef) -> Result<Value, ResourceError> {
        let path = self.store.path_for(reference)?;
        check_path_no_symlink(self.store.root(), &path, true)?;
        let value = self.store.read_raw_document(reference)?;
        self.store.document_view(reference, value)
    }

    pub(crate) fn read_optional_json_ref(
        &self,
        reference: &ResourceRef,
    ) -> Result<Option<Value>, ResourceError> {
        if !self.asset_exists(reference)? {
            return Ok(None);
        }
        self.read_json_ref(reference).map(Some)
    }

    pub(crate) fn public_json_replacement(
        &self,
        reference: &ResourceRef,
        value: &Value,
    ) -> Result<crate::resource_transactions::Replacement, ResourceError> {
        if !reference.is_local() {
            return Err(ResourceError::new("Transaction target must be local"));
        }
        self.store.path_for(reference)?;
        if !reference.path().ends_with(".json") {
            let body = value["text"]
                .as_str()
                .or_else(|| value["markup"].as_str())
                .ok_or_else(|| ResourceError::new("Native content has no body"))?;
            return crate::resource_transactions::Replacement::public_bytes(
                self.store.canonical_reference(reference)?,
                body.as_bytes().to_vec(),
            )
            .map_err(|error| ResourceError::new(error.to_string()));
        }
        let encoded = self.store.encode_document(reference, value, None)?;
        let bytes =
            serde_json::to_vec(&encoded).map_err(|error| ResourceError::new(error.to_string()))?;
        crate::resource_transactions::Replacement::public_json(
            self.store.canonical_reference(reference)?,
            bytes,
        )
        .map_err(|error| ResourceError::new(error.to_string()))
    }

    pub(crate) fn content_replacement(
        &self,
        reference: &ResourceRef,
        view: &Value,
        state: &Value,
    ) -> Result<crate::resource_transactions::Replacement, ResourceError> {
        let encoded = self.store.encode_document(reference, view, Some(state))?;
        let bytes =
            serde_json::to_vec(&encoded).map_err(|error| ResourceError::new(error.to_string()))?;
        crate::resource_transactions::Replacement::public_json(
            self.store.canonical_reference(reference)?,
            bytes,
        )
        .map_err(|error| ResourceError::new(error.to_string()))
    }
}

fn ensure_resource_writes_recovered(root: &Path) -> Result<(), ResourceError> {
    crate::resource_transactions::ensure_no_unrecovered_transaction(root).map_err(|error| {
        ResourceError::new(format!(
            "Resource writes require transaction recovery: {error}"
        ))
    })
}

pub(super) fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "resource path has no parent".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("cannot create parent: {error}"))?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .map_err(|error| format!("atomic file write failed: {error}"))
}
