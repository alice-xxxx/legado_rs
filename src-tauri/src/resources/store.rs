//! 管理应用公开资源文件存储与原子更新。

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
    BookDocument, ProgressDocument, ReaderDefaults, SettingsDocument, ShelfDocument,
    CURRENT_SCHEMA_VERSION,
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
    update_lock: Arc<Mutex<()>>,
}

/// JSON-file storage for public app state and the resources consumed by the UI.
/// Private engine data should live in another directory and is never routed by
/// this store's HTTP server.
#[derive(Clone)]
pub struct ResourceStore {
    inner: Arc<StoreInner>,
}

impl ResourceStore {
    /// Open (or create) the public app-data root and its first JSON documents.
    /// Existing documents are left untouched, including documents this version
    /// does not understand.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ResourceError> {
        std::fs::create_dir_all(root.as_ref())
            .map_err(|error| ResourceError::new(format!("Cannot create resource root: {error}")))?;
        let root = std::fs::canonicalize(root.as_ref()).map_err(|error| {
            ResourceError::new(format!("Cannot resolve resource root: {error}"))
        })?;
        ensure_resource_writes_recovered(&root)?;
        let store = Self {
            inner: Arc::new(StoreInner {
                root,
                update_lock: Arc::new(Mutex::new(())),
            }),
        };

        let mut settings =
            serde_json::to_value(SettingsDocument::default()).expect("serializable default");
        settings["lastBackupAtMs"] = Value::Null;

        for (resource, value) in [
            (
                store.shelf_ref(),
                serde_json::to_value(ShelfDocument::default()).expect("serializable default"),
            ),
            (store.settings_ref(), settings),
            (
                store.http_tts_configs_ref(),
                crate::http_tts_config::empty_document(),
            ),
        ] {
            let path = store.path_for(&resource)?;
            if !path.exists() {
                let bytes = serde_json::to_vec_pretty(&value).map_err(|error| {
                    ResourceError::new(format!("Cannot encode initial JSON resource: {error}"))
                })?;
                atomic_replace(&path, &bytes).map_err(|error| {
                    ResourceError::new(format!("Cannot initialize {}: {error}", resource.path()))
                })?;
            }
        }
        // Cache garbage collection is maintenance, not app-data recovery.
        // A locked, damaged or read-only orphan file must not prevent the
        // user from opening an otherwise valid library.
        if let Err(error) = prune_unreferenced_chapter_versions(&store.inner.root) {
            eprintln!("Chapter cache maintenance skipped: {error}");
        }
        if let Err(error) = prune_unused_chapter_media_mappings(&store.inner.root) {
            eprintln!("Chapter media mapping maintenance skipped: {error}");
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// Acquire the same store-wide mutex used by all public resource writers.
    /// The owned guard can move into a blocking transaction worker without
    /// releasing the mutex between the caller's reads and the commit.
    pub(crate) async fn transaction_writer_guard(
        &self,
    ) -> Result<ResourceStoreWriterGuard, ResourceError> {
        let guard = Arc::clone(&self.inner.update_lock).lock_owned().await;
        ensure_resource_writes_recovered(&self.inner.root)?;
        Ok(ResourceStoreWriterGuard {
            store: self.clone(),
            _guard: guard,
        })
    }

    pub fn shelf_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("shelf.json")
    }

    pub fn settings_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("settings.json")
    }

    pub fn http_tts_configs_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("http-tts.json")
    }

    pub fn source_definitions_ref(
        &self,
        content_hash: &str,
    ) -> Result<ResourceRef, ResourceError> {
        self.reading_ref(&format!("source-definitions-{content_hash}"))
    }

    pub fn bookmarks_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("bookmarks.json")
    }

    pub fn book_ref(&self, book_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        ResourceRef::new(format!("resource://books/{book_id}/book.json"))
    }

    pub fn progress_ref(&self, book_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        ResourceRef::new(format!("resource://progress/{book_id}.json"))
    }

    /// Versioned chapter bodies are addressed by identity and content.
    /// Changing a source chapter does not silently retarget older descriptors.
    fn chapter_version_ref(
        book_id: &str,
        chapter_id: &str,
        document: &Value,
    ) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        validate_id(chapter_id)?;
        let serialized = serde_json::to_vec(document)
            .map_err(|error| ResourceError::new(format!("Cannot encode chapter payload: {error}")))?;
        let chapter_hash = Sha256::digest(chapter_id.as_bytes());
        let content_hash = Sha256::digest(&serialized);
        ResourceRef::new(format!(
            "resource://books/{book_id}/chapters/{chapter_hash:x}-{content_hash:x}.json"
        ))
    }

    pub fn search_ref(&self, search_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(search_id)?;
        ResourceRef::new(format!("resource://search/{search_id}.json"))
    }

    pub fn reading_ref(&self, document_name: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(document_name)?;
        ResourceRef::new(format!("resource://reading/{document_name}.json"))
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
        ResourceRef::new(format!("resource://books/{book_id}/assets/{asset_id}"))
    }

    pub async fn write_asset(
        &self,
        book_id: &str,
        asset_id: &str,
        bytes: &[u8],
    ) -> Result<ResourceRef, ResourceError> {
        let reference = self.asset_ref(book_id, asset_id)?;
        let path = self.path_for(&reference)?;
        let root = self.inner.root.clone();
        let write_path = path;
        let bytes = bytes.to_vec();
        let _guard = self.inner.update_lock.lock().await;
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
    /// The reference namespace is deliberately limited to `books/tts-audio/media`.
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
        let _guard = self.inner.update_lock.lock().await;
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

    /// Search result snapshots are WebView-session resources. They have no
    /// stable business identity and must not accumulate across app restarts.
    pub(crate) async fn remove_search_snapshots(&self) -> Result<usize, ResourceError> {
        self.remove_session_json_snapshots("search", "search-snapshot-")
            .await
    }

    /// TXT rule versions are immutable browser resources for one WebView
    /// session; the persisted current document remains at its stable ref.
    pub(crate) async fn remove_txt_toc_rule_snapshots(&self) -> Result<usize, ResourceError> {
        self.remove_session_json_snapshots("reading", "txt-toc-rules-snapshot-")
            .await
    }

    /// Source-definition resources are immutable snapshots scoped to one
    /// WebView session; Rust's private source records remain authoritative.
    pub(crate) async fn remove_source_definition_snapshots(
        &self,
    ) -> Result<usize, ResourceError> {
        self.remove_session_json_snapshots("reading", "source-definitions-")
            .await
    }

    async fn remove_session_json_snapshots(
        &self,
        directory_name: &'static str,
        filename_prefix: &'static str,
    ) -> Result<usize, ResourceError> {
        let _guard = self.inner.update_lock.lock().await;
        let root = self.inner.root.clone();
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            let directory = root.join(directory_name);
            check_path_no_symlink(&root, &directory, true)?;
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(0);
                }
                Err(error) => {
                    return Err(ResourceError::new(format!(
                        "Cannot inspect resource snapshots: {error}"
                    )));
                }
            };
            let mut removed = 0usize;
            for entry in entries {
                let entry = entry.map_err(|error| {
                    ResourceError::new(format!("Cannot inspect resource snapshot entry: {error}"))
                })?;
                let file_type = entry.file_type().map_err(|error| {
                    ResourceError::new(format!("Cannot inspect resource snapshot type: {error}"))
                })?;
                if !file_type.is_file() || file_type.is_symlink() {
                    continue;
                }
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !name.starts_with(filename_prefix) || !name.ends_with(".json") {
                    continue;
                }
                let path = entry.path();
                check_path_no_symlink(&root, &path, false)?;
                std::fs::remove_file(&path).map_err(|error| {
                    ResourceError::new(format!("Cannot remove stale resource snapshot: {error}"))
                })?;
                removed = removed.saturating_add(1);
            }
            Ok(removed)
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("Resource snapshot cleanup worker failed: {error}"))
        })?
    }

    pub(crate) async fn regular_file_exists(
        &self,
        reference: &ResourceRef,
    ) -> Result<bool, ResourceError> {
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
        let path = self.path_for(reference)?;
        check_path_no_symlink(&self.inner.root, &path, false)?;
        let bytes = tokio::fs::read(&path).await.map_err(|error| {
            ResourceError::new(format!("Cannot read {}: {error}", reference.path()))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            ResourceError::new(format!("Cannot decode {}: {error}", reference.path()))
        })
    }

    /// Read mutable app-owned JSON state and atomically initialize a missing
    /// or malformed file. Callers must use this only for state with a known
    /// default; resource payloads keep strict reads.
    pub async fn read_json_ref_or_default(
        &self,
        reference: &ResourceRef,
        default: Value,
    ) -> Result<Value, ResourceError> {
        let path = self.path_for(reference)?;
        check_path_no_symlink(&self.inner.root, &path, true)?;
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return self
                    .update_json_ref_or_default(reference, default, |value| Ok(value))
                    .await;
            }
            Err(error) => {
                return Err(ResourceError::new(format!(
                    "Cannot read {}: {error}",
                    reference.path()
                )));
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(value) => Ok(value),
            Err(_) => {
                self.update_json_ref_or_default(reference, default, |value| Ok(value))
                    .await
            }
        }
    }

    /// Atomically replace a public JSON document. Stable references embedded in
    /// it remain stable on disk and are expanded only by an HTTP GET response.
    pub async fn write_json_ref(
        &self,
        reference: &ResourceRef,
        value: &Value,
    ) -> Result<(), ResourceError> {
        let path = self.path_for(reference)?;
        if !reference.path().ends_with(".json") {
            return Err(ResourceError::new("JSON data requires a .json resource"));
        }
        // Source definitions are opaque Legado input: a nested `src` can be a
        // parsing rule rather than an application resource reference.
        if reference.path() != "sources.json" {
            validate_embedded_refs(value)?;
        }
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|error| ResourceError::new(format!("Cannot encode JSON: {error}")))?;
        let _guard = self.inner.update_lock.lock().await;
        let root = self.inner.root.clone();
        let write_path = path.clone();
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            check_path_no_symlink(&root, &write_path, true)?;
            atomic_replace(&write_path, &bytes).map_err(|error| {
                ResourceError::new(format!("Cannot atomically update JSON resource: {error}"))
            })
        })
        .await
        .map_err(|error| ResourceError::new(format!("JSON storage worker failed: {error}")))??;
        Ok(())
    }

    /// Serialize a concurrent read/modify/write operation under the same
    /// store lock used for all persistent JSON documents.
    pub async fn update_json_ref<F>(
        &self,
        reference: &ResourceRef,
        update: F,
    ) -> Result<Value, ResourceError>
    where
        F: FnOnce(Value) -> Result<Value, String> + Send,
    {
        self.update_json_ref_inner(reference, None, update).await
    }

    /// Update mutable app-owned state, treating a missing or malformed JSON
    /// file as the supplied default. The next successful update atomically
    /// replaces the damaged file. Use only when the caller owns a default
    /// document; immutable resources and transaction metadata remain strict.
    pub async fn update_json_ref_or_default<F>(
        &self,
        reference: &ResourceRef,
        default: Value,
        update: F,
    ) -> Result<Value, ResourceError>
    where
        F: FnOnce(Value) -> Result<Value, String> + Send,
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
        F: FnOnce(Value) -> Result<Value, String> + Send,
    {
        let path = self.path_for(reference)?;
        if !reference.is_local() || !reference.path().ends_with(".json") {
            return Err(ResourceError::new(
                "JSON update requires a local .json resource",
            ));
        }
        let _guard = self.inner.update_lock.lock().await;
        ensure_resource_writes_recovered(&self.inner.root)?;
        check_path_no_symlink(&self.inner.root, &path, true)?;
        let current = match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(error) => match &default {
                    Some(default) => default.clone(),
                    None => {
                        return Err(ResourceError::new(format!(
                            "Cannot decode {}: {error}",
                            reference.path()
                        )));
                    }
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                default.unwrap_or(Value::Null)
            }
            Err(error) => {
                return Err(ResourceError::new(format!(
                    "Cannot read {}: {error}",
                    reference.path()
                )));
            }
        };
        let updated = update(current).map_err(ResourceError::new)?;
        validate_embedded_refs(&updated)?;
        let bytes = serde_json::to_vec_pretty(&updated)
            .map_err(|error| ResourceError::new(format!("Cannot encode JSON: {error}")))?;
        let root = self.inner.root.clone();
        let write_path = path;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
            check_path_no_symlink(&root, &write_path, true)?;
            atomic_replace(&write_path, &bytes).map_err(|error| {
                ResourceError::new(format!("Cannot atomically update JSON resource: {error}"))
            })
        })
        .await
        .map_err(|error| ResourceError::new(format!("JSON storage worker failed: {error}")))??;
        Ok(updated)
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
        self.write_serializable(&self.shelf_ref(), shelf).await
    }

    pub async fn write_settings(&self, settings: &SettingsDocument) -> Result<(), ResourceError> {
        self.write_serializable(&self.settings_ref(), settings)
            .await
    }

    pub async fn write_progress(
        &self,
        book_id: &str,
        progress: &ProgressDocument,
    ) -> Result<(), ResourceError> {
        validate_id(book_id)?;
        if progress.book_id != book_id {
            return Err(ResourceError::new(
                "Progress bookId does not match the resource path",
            ));
        }
        let reference = self.progress_ref(book_id)?;
        self.write_serializable(&reference, progress).await
    }

    pub async fn write_book(&self, book: &BookDocument) -> Result<(), ResourceError> {
        let reference = self.book_ref(&book.id)?;
        self.write_serializable(&reference, book).await
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
        let reference = Self::chapter_version_ref(book_id, chapter_id, &document)?;
        self.write_json_ref(&reference, &document).await?;
        Ok(reference)
    }

    pub(crate) fn chapter_text_document(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_text: &str,
    ) -> Result<(ResourceRef, Value), ResourceError> {
        let document = serde_json::json!({
            "schemaVersion": crate::models::CURRENT_SCHEMA_VERSION,
            "kind": "text",
            "text": processed_text,
        });
        let reference = Self::chapter_version_ref(book_id, chapter_id, &document)?;
        Ok((reference, document))
    }

    /// Store resolved media metadata as data. The actual media remains in the
    /// private media registry and Vue owns the final <audio>/<video> rendering.
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
        self.write_json_ref(&reference, &document).await?;
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
        let reference = Self::chapter_version_ref(book_id, chapter_id, &document)?;
        Ok((reference, document))
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
        let reference = Self::chapter_version_ref(book_id, chapter_id, &document)?;
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
        let reference = Self::chapter_version_ref(book_id, chapter_id, &document)?;
        Ok((reference, document))
    }

    /// Store one local-PDF page as typed data. The PDF stays a binary asset
    /// and the JSON document only describes which page Vue should render.
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

        let document = json!({
            "schemaVersion": CURRENT_SCHEMA_VERSION,
            "kind": "pdfPage",
            "src": pdf_ref.as_str(),
            "pageIndex": page_index,
            "defaultZoom": default_zoom,
        });
        let reference = Self::chapter_version_ref(book_id, chapter_id, &document)?;
        self.write_json_ref(&reference, &document).await?;
        Ok(reference)
    }

fn local_epub_asset_refs(book_id: &str, markup: &str) -> Result<String, ResourceError> {
    validate_id(book_id)?;
    let base = format!("resource://books/{book_id}/assets/");
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
        let _guard = self.inner.update_lock.lock().await;
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
    /// The original is intentionally outside the public ResourceRef namespace:
    /// WebView consumers can only access parsed/processed resources.
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
            .map_err(|error| ResourceError::new(format!("Cannot inspect managed import: {error}")))?;
        if !source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
            return Err(ResourceError::new("Managed local original must be a regular file"));
        }

        let book_dir = self.inner.root.join("books").join(book_id);
        let target = book_dir.join(format!("original.{extension}"));
        let root = self.inner.root.clone();
        let source_path = source_path.to_path_buf();
        let _guard = self.inner.update_lock.lock().await;
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

    /// Delete a book's public cache and its progress document. The caller
    /// updates the shelf JSON as part of the same application operation.
    pub async fn remove_book_resources(&self, book_id: &str) -> Result<(), ResourceError> {
        validate_id(book_id)?;
        let book_dir = self.inner.root.join("books").join(book_id);
        let progress_path = self
            .inner
            .root
            .join("progress")
            .join(format!("{book_id}.json"));
        let root = self.inner.root.clone();
        let _guard = self.inner.update_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
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
            check_path_no_symlink(&root, &progress_path, true)?;
            match std::fs::remove_file(progress_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ResourceError::new(format!(
                    "Cannot remove book progress: {error}"
                ))),
            }
        })
        .await
        .map_err(|error| ResourceError::new(format!("Book deletion worker failed: {error}")))??;
        Ok(())
    }

    /// Remove only cached RSS typed article bodies and media for a source's private
    /// pseudo-book namespace. Shelf, progress, and ordinary book resources are
    /// outside this operation and remain untouched.
    pub async fn remove_rss_chapter_resources(&self, source_id: &str) -> Result<(), ResourceError> {
        validate_id(source_id)?;
        let rss_book_id = format!("rss-{source_id}");
        validate_id(&rss_book_id)?;
        let book_dir = self.inner.root.join("books").join(rss_book_id);
        let directories = [book_dir.join("chapters"), book_dir.join("assets")];
        let root = self.inner.root.clone();
        let _guard = self.inner.update_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            ensure_resource_writes_recovered(&root)?;
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

    pub(super) fn path_for(&self, reference: &ResourceRef) -> Result<PathBuf, ResourceError> {
        // Revalidate even though ResourceRef itself is validated: deserialized
        // data and future constructors should never weaken the filesystem guard.
        if !reference.is_local() {
            return Err(ResourceError::new(
                "External HTTP(S) resource URLs do not map to local files",
            ));
        }
        validate_public_path(reference.path())?;
        Ok(self.inner.root.join(reference.path()))
    }
}

/// An owned permit for the store-wide writer mutex used by ordinary resource
/// writes. This is the adapter held for the full file-transaction lifetime.
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

    /// Read a persisted public document while the caller holds the shared
    /// writer mutex, without trying to acquire it recursively.
    pub(crate) fn read_json_ref(&self, reference: &ResourceRef) -> Result<Value, ResourceError> {
        if !reference.is_local() || !reference.path().ends_with(".json") {
            return Err(ResourceError::new(
                "Transaction reads require a local .json resource",
            ));
        }
        let path = self.store.path_for(reference)?;
        check_path_no_symlink(self.store.root(), &path, false)?;
        let bytes = std::fs::read(&path).map_err(|error| {
            ResourceError::new(format!("Cannot read {}: {error}", reference.path()))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            ResourceError::new(format!("Cannot decode {}: {error}", reference.path()))
        })
    }

    /// Read an optional public JSON resource under the writer mutex. A missing
    /// target remains missing until the caller commits it with replacements.
    pub(crate) fn read_optional_json_ref(
        &self,
        reference: &ResourceRef,
    ) -> Result<Option<Value>, ResourceError> {
        if !reference.is_local() || !reference.path().ends_with(".json") {
            return Err(ResourceError::new(
                "Transaction reads require a local .json resource",
            ));
        }
        let path = self.store.path_for(reference)?;
        check_path_no_symlink(self.store.root(), &path, true)?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ResourceError::new(format!(
                    "Cannot read {}: {error}",
                    reference.path()
                )));
            }
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            ResourceError::new(format!("Cannot decode {}: {error}", reference.path()))
        })
    }

    /// Construct a public transaction replacement using the same JSON
    /// serialization and resource-reference validation as ordinary writes.
    pub(crate) fn public_json_replacement(
        &self,
        reference: &ResourceRef,
        value: &Value,
    ) -> Result<crate::resource_transactions::Replacement, ResourceError> {
        if !reference.is_local() || !reference.path().ends_with(".json") {
            return Err(ResourceError::new(
                "Transaction replacements require a local .json resource",
            ));
        }
        self.store.path_for(reference)?;
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|error| ResourceError::new(format!("Cannot encode JSON: {error}")))?;
        crate::resource_transactions::Replacement::public_json(reference.clone(), bytes)
            .map_err(|error| ResourceError::new(error.to_string()))
    }
}

/// Reclaim stale typed versions after crash recovery, before any WebView can
/// hold a descriptor. During normal operation an older published resource is
/// retained, even when the book switches to a newer version.
fn prune_unreferenced_chapter_versions(root: &Path) -> Result<(), ResourceError> {
    use std::collections::HashSet;
    use std::fs;

    // RSS has its own Rust-owned article catalog instead of book.json.
    // If that catalog cannot be read, retain all RSS versions conservatively.
    let rss_path = root.join("reading/rss-state.json");
    check_path_no_symlink(root, &rss_path, true)?;
    let rss_state: Option<crate::rss::RssStateDocument> =
        fs::read(&rss_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|document| crate::rss::validate_rss_state(document).is_ok())
            .and_then(|document| serde_json::from_value(document).ok());
    let books_dir = root.join("books");
    check_path_no_symlink(root, &books_dir, true)?;
    let books = match fs::read_dir(&books_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ResourceError::new(format!("Cannot inspect book cache: {error}")));
        }
    };
    for book in books {
        let book = book.map_err(|error| ResourceError::new(error.to_string()))?;
        // Never descend into symlinks or special files.
        if !book.file_type().map_err(|error| ResourceError::new(error.to_string()))?.is_dir() {
            continue;
        }
        let Some(book_id) = book.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_id(&book_id).is_err() {
            continue;
        }
        let book_path = book.path();
        let book_json_path = book_path.join("book.json");
        check_path_no_symlink(root, &book_json_path, true)?;
        let referenced: HashSet<String> = match fs::read(&book_json_path) {
            Ok(bytes) => {
                let Ok(document) = serde_json::from_slice::<BookDocument>(&bytes) else {
                    // Corrupt or incomplete catalogs are not permission to
                    // delete potentially recoverable user content.
                    continue;
                };
                // An invalid catalog must never become permission to remove
                // files merely because its refs point outside this book.
                if document.id != book_id {
                    continue;
                }
                let prefix = format!("books/{book_id}/chapters/");
                if document.chapters.iter().any(|chapter| {
                    chapter.resource.as_ref().is_some_and(|resource| {
                        !resource.resource_id.is_local()
                            || !resource.resource_id.path().starts_with(&prefix)
                            || !resource.resource_id.path().ends_with(".json")
                    })
                }) {
                    continue;
                }
                document.chapters.iter()
                    .filter_map(|chapter| chapter.resource.as_ref())
                    .filter_map(|resource| resource.resource_id.path().strip_prefix(&prefix))
                    .map(str::to_owned)
                    .collect()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
                && book_id.starts_with("rss-") =>
            {
                let Some(state) = rss_state.as_ref() else {
                    continue;
                };
                let prefix = format!("books/{book_id}/chapters/");
                state.articles.iter()
                    .filter(|article| format!("rss-{}", article.source_id) == book_id)
                    .filter_map(|article| article.content_ref.as_ref())
                    .filter_map(|reference| reference.path().strip_prefix(&prefix))
                    .map(str::to_owned)
                    .collect()
            }
            // Source debug previews and import staging also lack book.json.
            // Their lifecycle is managed separately; do not treat their files
            // as orphaned book content.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ResourceError::new(format!("Cannot read book cache: {error}")));
            }
        };
        let chapter_dir = book_path.join("chapters");
        check_path_no_symlink(root, &chapter_dir, true)?;
        let chapters = match fs::read_dir(&chapter_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ResourceError::new(format!("Cannot inspect chapter cache: {error}")));
            }
        };
        for chapter in chapters {
            let chapter = chapter.map_err(|error| ResourceError::new(error.to_string()))?;
            let name = chapter.file_name().to_string_lossy().into_owned();
            if chapter.file_type().is_ok_and(|type_| type_.is_file())
                && name.ends_with(".json")
                && !referenced.contains(&name)
            {
                fs::remove_file(chapter.path()).map_err(|error| {
                    ResourceError::new(format!("Cannot reclaim chapter cache: {error}"))
                })?;
            }
        }
    }
    Ok(())
}

/// Only chapter-media identities are eligible: other opaque media mappings
/// may be owned by independent subsystems. Scan current typed chapter bodies,
/// not stale versions, and fail closed (preserve mappings) if any book/catalog
/// or referenced media document cannot be inspected safely.
fn prune_unused_chapter_media_mappings(root: &Path) -> Result<(), ResourceError> {
    use std::collections::HashSet;
    use std::fs;

    let books_dir = root.join("books");
    check_path_no_symlink(root, &books_dir, true)?;
    let mut live_ids = HashSet::new();
    let books = match fs::read_dir(&books_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ResourceError::new(format!(
                "Cannot inspect book catalogs for media cleanup: {error}"
            )));
        }
    };
    for book in books {
        let book = book.map_err(|error| ResourceError::new(error.to_string()))?;
        if !book.file_type().map_err(|error| ResourceError::new(error.to_string()))?.is_dir() {
            continue;
        }
        let Some(book_id) = book.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_id(&book_id).is_err() {
            continue;
        }
        let catalog_path = book.path().join("book.json");
        check_path_no_symlink(root, &catalog_path, true)?;
        let catalog = match fs::read(&catalog_path) {
            Ok(bytes) => serde_json::from_slice::<BookDocument>(&bytes)
                .map_err(|error| ResourceError::new(format!(
                    "Cannot decode book catalog before media cleanup: {error}"
                )))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(ResourceError::new(format!(
                "Cannot read book catalog before media cleanup: {error}"
            ))),
        };
        if catalog.id != book_id {
            return Err(ResourceError::new(
                "Book catalog identity mismatch; refusing media mapping cleanup"
            ));
        }
        for chapter in catalog.chapters {
            let Some(resource) = chapter.resource else {
                continue;
            };
            if !matches!(resource.content_format.as_str(), "audio" | "video") {
                continue;
            }
            let expected_prefix = format!("books/{book_id}/chapters/");
            if !resource.resource_id.is_local()
                || !resource.resource_id.path().starts_with(&expected_prefix)
                || !resource.resource_id.path().ends_with(".json")
            {
                return Err(ResourceError::new(
                    "Cannot validate a media chapter path for mapping cleanup"
                ));
            }
            let path = root.join(resource.resource_id.path());
            check_path_no_symlink(root, &path, false)?;
            let bytes = fs::read(&path).map_err(|error| ResourceError::new(format!(
                "Cannot read media chapter for mapping cleanup: {error}"
            )))?;
            let document: Value = serde_json::from_slice(&bytes).map_err(|error| {
                ResourceError::new(format!("Cannot decode media chapter for mapping cleanup: {error}"))
            })?;
            if document.get("kind").and_then(Value::as_str) != Some("media") {
                return Err(ResourceError::new(
                    "Media chapter is not typed media; refusing mapping cleanup"
                ));
            }
            let mut refs = Vec::new();
            refs.push(document.get("src").and_then(Value::as_str));
            if let Some(sources) = document.get("sources").and_then(Value::as_array) {
                refs.extend(sources.iter().map(|source| source.get("src").and_then(Value::as_str)));
            }
            for reference in refs.into_iter().flatten() {
                if let Some(id) = reference.strip_prefix("resource://media/") {
                    if is_chapter_media_mapping_id(id) {
                        live_ids.insert(id.to_owned());
                    }
                }
            }
        }
    }
    let directory = root.join("private-data/media-maps");
    check_path_no_symlink(root, &directory, true)?;
    let mappings = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ResourceError::new(format!(
            "Cannot inspect private chapter media mappings: {error}"
        ))),
    };
    for mapping in mappings {
        let mapping = mapping.map_err(|error| ResourceError::new(error.to_string()))?;
        let name = mapping.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        if !is_chapter_media_mapping_id(id) || live_ids.contains(id) {
            continue;
        }
        // Symlinks and special files are never removed by this collector.
        if !mapping.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        fs::remove_file(mapping.path()).map_err(|error| {
            ResourceError::new(format!("Cannot remove obsolete chapter media mapping: {error}"))
        })?;
    }
    Ok(())
}

fn is_chapter_media_mapping_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("chapter-media-") else {
        return false;
    };
    let (hash, variant) = match rest.split_once("-variant-") {
        Some((hash, variant)) => (hash, Some(variant)),
        None => (rest, None),
    };
    hash.len() == 64
        && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && variant.is_none_or(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
}

fn ensure_resource_writes_recovered(root: &Path) -> Result<(), ResourceError> {
    crate::resource_transactions::ensure_no_unrecovered_transaction(root).map_err(|error| {
        ResourceError::new(format!(
            "Resource writes are blocked until transaction recovery succeeds: {error}"
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
