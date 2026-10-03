//! JSON-backed persistence and loopback delivery of browser-consumable resources.
//!
//! Persistent JSON stores stable `resource://` references. The HTTP server
//! materializes those references into a per-process URL only when JSON is read.
//! This keeps ports and runtime capabilities out of durable application state.

use std::fmt;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ammonia::Builder as HtmlSanitizer;
use atomicwrites::{AllowOverwrite, AtomicFile};
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{
    ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL, CONTENT_LENGTH,
    CONTENT_RANGE, CONTENT_SECURITY_POLICY, CONTENT_TYPE, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex, OwnedMutexGuard};
use tokio::task::JoinHandle;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::models::{
    BookDocument, ProgressDocument, ReaderDefaults, SettingsDocument, ShelfDocument,
};
use http_range_header::parse_range_header;

const MAX_PRIVATE_MEDIA_MAPPING_BYTES: usize = 1024 * 1024;

#[path = "media_proxy.rs"]
mod media_proxy;
use media_proxy::MediaProxyRegistry;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceRef(String);

impl ResourceRef {
    /// Construct a public stable reference (`resource://...`) or a direct
    /// external HTTP(S) resource URL. Other URL schemes are never accepted.
    pub fn new(reference: impl AsRef<str>) -> Result<Self, ResourceError> {
        let reference = reference.as_ref();
        if let Some(path) = reference.strip_prefix("resource://") {
            validate_public_path(path)?;
            return Ok(Self(format!("resource://{path}")));
        }
        let url = reqwest::Url::parse(reference)
            .map_err(|_| ResourceError::new("Resource URL must be a valid HTTP(S) URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ResourceError::new(
                "Only credential-free HTTP(S) resource URLs are allowed",
            ));
        }
        Ok(Self(url.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn path(&self) -> &str {
        // External references have no local filesystem path.
        self.0.strip_prefix("resource://").unwrap_or("")
    }

    pub fn is_local(&self) -> bool {
        self.0.starts_with("resource://")
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ResourceRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ResourceRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug)]
pub struct ResourceError {
    message: String,
}

impl ResourceError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ResourceError {}

impl From<std::io::Error> for ResourceError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

struct StoreInner {
    root: PathBuf,
    update_lock: Arc<Mutex<()>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrivateMediaMapping {
    schema_version: u32,
    id: String,
    upstream_url: String,
    headers: Vec<PrivateMediaHeader>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrivateMediaHeader {
    name: String,
    value: String,
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

        for (resource, value) in [
            (
                store.shelf_ref(),
                serde_json::to_value(ShelfDocument::default()).expect("serializable default"),
            ),
            (
                store.settings_ref(),
                serde_json::to_value(SettingsDocument::default()).expect("serializable default"),
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

    pub fn book_ref(&self, book_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        ResourceRef::new(format!("resource://books/{book_id}/book.json"))
    }

    pub fn progress_ref(&self, book_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        ResourceRef::new(format!("resource://progress/{book_id}.json"))
    }

    pub fn chapter_ref(
        &self,
        book_id: &str,
        chapter_id: &str,
    ) -> Result<ResourceRef, ResourceError> {
        validate_id(book_id)?;
        validate_id(chapter_id)?;
        ResourceRef::new(format!(
            "resource://books/{book_id}/chapters/{chapter_id}.html"
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
        validate_embedded_refs(value)?;
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
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                ResourceError::new(format!("Cannot decode {}: {error}", reference.path()))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
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

    /// Write processed plain text as safe, paragraph-preserving HTML. Source
    /// rules and unprocessed source data are never written into this resource.
    pub async fn write_chapter_text(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_text: &str,
        defaults: &ReaderDefaults,
    ) -> Result<ResourceRef, ResourceError> {
        let reference = self.chapter_ref(book_id, chapter_id)?;
        let content = text_to_paragraph_html(processed_text);
        let document = chapter_document(&content, defaults);
        self.write_html_resource(&reference, &document).await?;
        Ok(reference)
    }

    /// Write already-processed article HTML after removing executable content
    /// and unsafe attributes/URLs. Useful markup such as paragraphs, links and
    /// images is retained by the HTML sanitizer.
    pub async fn write_chapter_html(
        &self,
        book_id: &str,
        chapter_id: &str,
        processed_html: &str,
        defaults: &ReaderDefaults,
    ) -> Result<ResourceRef, ResourceError> {
        let reference = self.chapter_ref(book_id, chapter_id)?;
        let mut sanitizer = HtmlSanitizer::default();
        sanitizer
            .add_tags(&["link"])
            .add_tag_attributes("link", &["href", "rel", "type", "media"])
            .add_tag_attribute_values("link", "rel", &["stylesheet"])
            .url_relative(ammonia::UrlRelative::PassThrough);
        let sanitized = sanitizer.clean(processed_html).to_string();
        let document = chapter_document(&sanitized, defaults);
        self.write_html_resource(&reference, &document).await?;
        Ok(reference)
    }

    /// Store one local-PDF page as a safe, typed HTML resource. The PDF itself
    /// remains an asset file; the viewer fetches it through the browser resource
    /// server and can use byte ranges without copying the document into JSON.
    pub async fn write_pdf_page(
        &self,
        book_id: &str,
        chapter_id: &str,
        pdf_asset_id: &str,
        page_index: u32,
        default_zoom: &str,
        defaults: &ReaderDefaults,
    ) -> Result<ResourceRef, ResourceError> {
        let reference = self.chapter_ref(book_id, chapter_id)?;
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

        let escaped_asset = escape_html(pdf_asset_id);
        let html = format!(
            "<section data-legado-document=\"pdf-page\" data-page-index=\"{page_index}\" data-default-zoom=\"{default_zoom}\"><a data-legado-pdf-src href=\"../assets/{escaped_asset}\">Open PDF page {}</a></section>",
            page_index.saturating_add(1)
        );
        let document = chapter_document(&html, defaults);
        self.write_html_resource(&reference, &document).await?;
        Ok(reference)
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

    /// Remove only cached RSS article HTML and media for a source's private
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

    /// Start a read-only local HTTP service for public JSON and chapter HTML.
    /// Only loopback addresses are accepted; each server gets its own random
    /// path capability so resources are not enumerable by filename alone.
    pub async fn start_http(&self, bind: SocketAddr) -> Result<ResourceServer, ResourceError> {
        if !bind.ip().is_loopback() {
            return Err(ResourceError::new(
                "Resource server must bind to a loopback address",
            ));
        }
        let private_media_dir = self.inner.root.join("private-data").join("media-maps");
        let media_dir_root = self.inner.root.clone();
        let create_media_dir = private_media_dir.clone();
        tokio::task::spawn_blocking(move || {
            check_path_no_symlink(&media_dir_root, &create_media_dir, true)?;
            std::fs::create_dir_all(&create_media_dir).map_err(|error| {
                ResourceError::new(format!(
                    "Cannot create private media mapping directory: {error}"
                ))
            })?;
            check_path_no_symlink(&media_dir_root, &create_media_dir, false)?;
            let metadata = std::fs::metadata(&create_media_dir).map_err(|error| {
                ResourceError::new(format!(
                    "Cannot inspect private media mapping directory: {error}"
                ))
            })?;
            if !metadata.is_dir() {
                return Err(ResourceError::new(
                    "Private media mapping path is not a directory",
                ));
            }
            Ok(())
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("Private media directory worker failed: {error}"))
        })??;

        let mappings_root = self.inner.root.clone();
        let mappings_dir = private_media_dir.clone();
        let mappings = tokio::task::spawn_blocking(move || {
            read_private_media_mappings(&mappings_root, &mappings_dir, false)
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("Private media mapping worker failed: {error}"))
        })??;
        let media_proxy = MediaProxyRegistry::new()?;
        for mapping in mappings {
            let headers = mapping
                .headers
                .iter()
                .map(|header| (header.name.clone(), header.value.clone()))
                .collect::<Vec<_>>();
            if let Err(error) = media_proxy
                .register(&mapping.id, &mapping.upstream_url, &headers)
                .await
            {
                // One damaged/obsolete mapping should not prevent the rest of the
                // reader from opening. Invalid records stay private and unserved.
                eprintln!(
                    "Ignoring invalid persisted media mapping {}: {error}",
                    mapping.id
                );
            }
        }
        let listener = TcpListener::bind(bind)
            .await
            .map_err(|error| ResourceError::new(format!("Cannot bind resource server: {error}")))?;
        let local_addr = listener.local_addr().map_err(|error| {
            ResourceError::new(format!("Cannot read resource server address: {error}"))
        })?;
        let token = Uuid::new_v4().simple().to_string();
        let base_url = format!("http://{local_addr}/r/{token}/");
        let state = HttpState {
            store: self.clone(),
            token,
            base_url: base_url.clone(),
            media_proxy: media_proxy.clone(),
        };
        let router = Router::new()
            .route("/healthz", get(health))
            .route(
                "/r/{token}/{*resource}",
                get(get_resource)
                    .head(get_resource)
                    .options(resource_options),
            )
            .with_state(state);
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_receiver.await;
                })
                .await
        });
        Ok(ResourceServer {
            local_addr,
            base_url,
            media_proxy,
            private_media_dir,
            media_update_lock: Mutex::new(()),
            shutdown_sender: Some(shutdown_sender),
            task: Some(task),
        })
    }

    fn path_for(&self, reference: &ResourceRef) -> Result<PathBuf, ResourceError> {
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

fn ensure_resource_writes_recovered(root: &Path) -> Result<(), ResourceError> {
    crate::resource_transactions::ensure_no_unrecovered_transaction(root).map_err(|error| {
        ResourceError::new(format!(
            "Resource writes are blocked until transaction recovery succeeds: {error}"
        ))
    })
}

impl ResourceRef {
    fn from_validated_path(path: &str) -> Self {
        Self(format!("resource://{path}"))
    }
}

/// Handle for a running resource server. Dropping it stops the task; callers
/// that need ordered shutdown can call `shutdown` and await server completion.
pub struct ResourceServer {
    local_addr: SocketAddr,
    base_url: String,
    media_proxy: MediaProxyRegistry,
    private_media_dir: PathBuf,
    media_update_lock: Mutex<()>,
    shutdown_sender: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl ResourceServer {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn url_for(&self, reference: &ResourceRef) -> String {
        if !reference.is_local() {
            return reference.as_str().to_owned();
        }
        format!("{}{}", self.base_url, percent_encode_path(reference.path()))
    }

    /// Register a private upstream media request. The returned stable resource
    /// ref can be stored in a public descriptor; the URL and headers remain in
    /// this process and are never serialized to JSON.
    pub async fn register_media(
        &self,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        let opaque_id = Uuid::new_v4().simple().to_string();
        self.register_media_with_id(&opaque_id, upstream_url, private_headers)
            .await
    }

    /// Restore a media ref whose private mapping has been loaded by Rust from
    /// protected storage. `opaque_id` must be safe and is validated as a ref.
    pub async fn register_media_with_id(
        &self,
        opaque_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        let _guard = self.media_update_lock.lock().await;
        let reference =
            MediaProxyRegistry::validate_mapping(opaque_id, upstream_url, private_headers)?;
        let mapping = PrivateMediaMapping {
            schema_version: 1,
            id: opaque_id.to_owned(),
            upstream_url: upstream_url.to_owned(),
            headers: private_headers
                .iter()
                .map(|(name, value)| PrivateMediaHeader {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        };
        self.write_private_media_mapping(mapping).await?;
        self.media_proxy
            .register(opaque_id, upstream_url, private_headers)
            .await?;
        Ok(reference)
    }

    pub async fn unregister_media(&self, opaque_id: &str) -> Result<(), ResourceError> {
        ResourceRef::new(format!("resource://media/{opaque_id}"))?;
        let _guard = self.media_update_lock.lock().await;
        self.delete_private_media_mapping(opaque_id).await?;
        self.media_proxy.unregister(opaque_id).await;
        Ok(())
    }

    /// Replace the runtime media registry from the protected on-disk snapshot.
    /// Backup restore calls this after atomically replacing the app-data tree,
    /// so mappings added after the backup are removed and restored IDs point
    /// to the restored private URLs and headers.
    pub async fn reload_private_media_mappings(&self) -> Result<(), ResourceError> {
        let _guard = self.media_update_lock.lock().await;
        let root = self
            .private_media_dir
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| ResourceError::new("Private media root is invalid"))?
            .to_path_buf();
        let directory = self.private_media_dir.clone();
        let create_root = root.clone();
        let create_dir = directory.clone();
        tokio::task::spawn_blocking(move || {
            check_path_no_symlink(&create_root, &create_dir, true)?;
            std::fs::create_dir_all(&create_dir).map_err(|error| {
                ResourceError::new(format!(
                    "Cannot create private media mapping directory: {error}"
                ))
            })?;
            check_path_no_symlink(&create_root, &create_dir, false)
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("Private media directory worker failed: {error}"))
        })??;

        let mappings_dir = directory;
        let mappings_root = root;
        let mappings = tokio::task::spawn_blocking(move || {
            read_private_media_mappings(&mappings_root, &mappings_dir, true)
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("Private media mapping worker failed: {error}"))
        })??;
        let mappings = mappings
            .into_iter()
            .map(|mapping| {
                (
                    mapping.id,
                    mapping.upstream_url,
                    mapping
                        .headers
                        .into_iter()
                        .map(|header| (header.name, header.value))
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        self.media_proxy.replace_mappings(&mappings).await
    }

    async fn write_private_media_mapping(
        &self,
        mapping: PrivateMediaMapping,
    ) -> Result<(), ResourceError> {
        let root = self
            .private_media_dir
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| ResourceError::new("Private media root is invalid"))?
            .to_path_buf();
        let path = self.private_media_dir.join(format!("{}.json", mapping.id));
        let bytes = serde_json::to_vec_pretty(&mapping).map_err(|error| {
            ResourceError::new(format!("Cannot encode private media mapping: {error}"))
        })?;
        if bytes.len() > MAX_PRIVATE_MEDIA_MAPPING_BYTES {
            return Err(ResourceError::new("Private media mapping is too large"));
        }
        tokio::task::spawn_blocking(move || {
            check_path_no_symlink(&root, &path, true)?;
            atomic_replace(&path, &bytes).map_err(|error| {
                ResourceError::new(format!("Cannot persist private media mapping: {error}"))
            })
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("Private media write worker failed: {error}"))
        })??;
        Ok(())
    }

    async fn delete_private_media_mapping(&self, opaque_id: &str) -> Result<(), ResourceError> {
        let root = self
            .private_media_dir
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| ResourceError::new("Private media root is invalid"))?
            .to_path_buf();
        let path = self.private_media_dir.join(format!("{opaque_id}.json"));
        tokio::task::spawn_blocking(move || {
            check_path_no_symlink(&root, &path, true)?;
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ResourceError::new(format!(
                    "Cannot remove private media mapping: {error}"
                ))),
            }
        })
        .await
        .map_err(|error| {
            ResourceError::new(format!("Private media delete worker failed: {error}"))
        })??;
        Ok(())
    }

    /// Rewrite stable resource refs inside a JSON value to this server's
    /// capability URLs. This is a response-only transformation.
    pub fn materialize_json(&self, value: &mut Value) -> Result<(), ResourceError> {
        materialize_json(value, &self.base_url)
    }

    pub async fn shutdown(mut self) -> Result<(), ResourceError> {
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|error| {
                    ResourceError::new(format!("Resource server task failed: {error}"))
                })?
                .map_err(|error| ResourceError::new(format!("Resource server failed: {error}")))?;
        }
        Ok(())
    }
}

impl Drop for ResourceServer {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct HttpState {
    store: ResourceStore,
    token: String,
    base_url: String,
    media_proxy: MediaProxyRegistry,
}

async fn health() -> Json<Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn resource_options() -> Response<Body> {
    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .expect("static OPTIONS response is valid");
    set_cors_headers(response.headers_mut());
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Range, Content-Type"),
    );
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD, OPTIONS"),
    );
    response
}

async fn get_resource(
    State(state): State<HttpState>,
    AxumPath((token, resource_path)): AxumPath<(String, String)>,
    method: axum::http::Method,
    headers: HeaderMap,
) -> Response<Body> {
    if token != state.token {
        return empty_response(StatusCode::NOT_FOUND);
    }
    let reference = match ResourceRef::new(format!("resource://{resource_path}")) {
        Ok(reference) => reference,
        Err(_) => return empty_response(StatusCode::NOT_FOUND),
    };
    if let Some(media_id) = reference.path().strip_prefix("media/") {
        return state.media_proxy.serve(media_id, method, &headers).await;
    }
    let path = match state.store.path_for(&reference) {
        Ok(path) => path,
        Err(_) => return empty_response(StatusCode::NOT_FOUND),
    };

    if reference.path().ends_with(".json") {
        let mut json = match state.store.read_json_ref(&reference).await {
            Ok(json) => json,
            Err(_) => return empty_response(StatusCode::NOT_FOUND),
        };
        if materialize_json(&mut json, &state.base_url).is_err() {
            return empty_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let bytes = match serde_json::to_vec(&json) {
            Ok(bytes) => bytes,
            Err(_) => return empty_response(StatusCode::INTERNAL_SERVER_ERROR),
        };
        return byte_response(bytes, "application/json; charset=utf-8", &headers);
    }

    if check_path_no_symlink(state.store.root(), &path, false).is_err() {
        return empty_response(StatusCode::NOT_FOUND);
    }

    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return empty_response(StatusCode::NOT_FOUND),
    };
    let metadata = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return empty_response(StatusCode::NOT_FOUND),
    };
    stream_response(
        file,
        metadata.len(),
        &headers,
        mime_type(reference.path()),
        state.base_url.split("/r/").next().unwrap_or_default(),
    )
    .await
}

fn set_cors_headers(headers: &mut HeaderMap) {
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Accept-Ranges, Content-Length, Content-Range"),
    );
}

fn empty_response(status: StatusCode) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("status-only response is valid");
    set_cors_headers(response.headers_mut());
    response
}

fn byte_response(
    bytes: Vec<u8>,
    content_type: &'static str,
    headers: &HeaderMap,
) -> Response<Body> {
    let total = bytes.len() as u64;
    let range = match requested_range(headers, total) {
        Ok(range) => range,
        Err(()) => {
            let mut response = empty_response(StatusCode::RANGE_NOT_SATISFIABLE);
            response.headers_mut().insert(
                CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{total}"))
                    .expect("numeric content-range is valid"),
            );
            return response;
        }
    };
    let (status, content, start, end) = match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            bytes[start as usize..=end as usize].to_vec(),
            Some(start),
            Some(end),
        ),
        None => (StatusCode::OK, bytes, None, None),
    };
    let mut response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, content.len().to_string())
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::from(content))
        .expect("resource byte response is valid");
    set_cors_headers(response.headers_mut());
    if let (Some(start), Some(end)) = (start, end) {
        response.headers_mut().insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
                .expect("numeric content-range is valid"),
        );
    }
    response
}

async fn stream_response(
    mut file: tokio::fs::File,
    total: u64,
    headers: &HeaderMap,
    content_type: &'static str,
    resource_origin: &str,
) -> Response<Body> {
    let range = match requested_range(headers, total) {
        Ok(range) => range,
        Err(()) => {
            let mut response = empty_response(StatusCode::RANGE_NOT_SATISFIABLE);
            response.headers_mut().insert(
                CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{total}"))
                    .expect("numeric content-range is valid"),
            );
            return response;
        }
    };
    let (status, start, length, content_range) = match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            start,
            end - start + 1,
            Some(format!("bytes {start}-{end}/{total}")),
        ),
        None => (StatusCode::OK, 0, total, None),
    };
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return empty_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let bounded_reader = file.take(length);
    let body = Body::from_stream(ReaderStream::new(bounded_reader));
    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, length.to_string())
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "no-store");
    if content_type.starts_with("text/html") {
        let policy = format!(
            "default-src 'none'; img-src 'self' data: http: https:; style-src 'self' 'unsafe-inline' {resource_origin}; font-src 'self' data: {resource_origin}; media-src 'self' http: https:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
        );
        builder = builder.header(CONTENT_SECURITY_POLICY, policy);
    }
    if let Some(content_range) = content_range {
        builder = builder.header(CONTENT_RANGE, content_range);
    }
    let mut response = match builder.body(body) {
        Ok(response) => response,
        Err(_) => return empty_response(StatusCode::INTERNAL_SERVER_ERROR),
    };
    set_cors_headers(response.headers_mut());
    response
}

fn requested_range(headers: &HeaderMap, total: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(header) = headers.get(RANGE) else {
        return Ok(None);
    };
    if total == 0 {
        return Err(());
    }
    let value = header.to_str().map_err(|_| ())?;
    let parsed = parse_range_header(value).map_err(|_| ())?;
    let ranges = parsed.validate(total).map_err(|_| ())?;
    match ranges.as_slice() {
        [range] => Ok(Some((*range.start(), *range.end()))),
        // This server currently emits one byte range per response. Per HTTP
        // semantics, ignore valid multipart requests and send the full entity.
        _ => Ok(None),
    }
}

fn materialize_json(value: &mut Value, base_url: &str) -> Result<(), ResourceError> {
    materialize_json_at(value, base_url, None)
}

fn materialize_json_at(
    value: &mut Value,
    base_url: &str,
    property: Option<&str>,
) -> Result<(), ResourceError> {
    match value {
        Value::String(string)
            if string.starts_with("resource://")
                && property.is_some_and(is_resource_url_property) =>
        {
            let reference = ResourceRef::new(string.as_str())?;
            *string = format!("{base_url}{}", percent_encode_path(reference.path()));
            Ok(())
        }
        Value::Array(values) => {
            for value in values {
                materialize_json_at(value, base_url, property)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values.iter_mut() {
                materialize_json_at(value, base_url, Some(key))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_resource_url_property(property: &str) -> bool {
    property == "src" || property.ends_with("Src")
}

/// Validate an encoded public document with the same persistent-reference
/// rules used by `write_json_ref` and `update_json_ref`.
pub(crate) fn validate_persistable_json_bytes(bytes: &[u8]) -> Result<(), ResourceError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        ResourceError::new(format!("Transaction target is not valid JSON: {error}"))
    })?;
    validate_embedded_refs(&value)
}

fn validate_embedded_refs(value: &Value) -> Result<(), ResourceError> {
    validate_embedded_refs_with_key(value, None)
}

fn validate_embedded_refs_with_key(
    value: &Value,
    property: Option<&str>,
) -> Result<(), ResourceError> {
    match value {
        Value::String(string)
            if property.is_some_and(is_resource_url_property)
                && is_runtime_capability_url(string) =>
        {
            Err(ResourceError::new(
                "A per-run resource-server URL cannot be persisted; save its stable resource:// ref",
            ))
        }
        Value::String(string)
            if string.starts_with("resource://")
                || (property.is_some_and(is_resource_url_property) && !string.is_empty()) =>
        {
            ResourceRef::new(string).map(|_| ())
        }
        Value::Array(values) => {
            for value in values {
                validate_embedded_refs_with_key(value, property)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                validate_embedded_refs_with_key(value, Some(key))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_runtime_capability_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host
        .parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or_else(|_| host.eq_ignore_ascii_case("localhost"));
    if !loopback {
        return false;
    }
    let Some(mut segments) = url.path_segments() else {
        return false;
    };
    if segments.next() != Some("r") {
        return false;
    }
    let Some(token) = segments.next() else {
        return false;
    };
    token.len() == 32
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && segments.next().is_some_and(|resource| !resource.is_empty())
}

fn validate_public_path(path: &str) -> Result<(), ResourceError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('%') {
        return Err(ResourceError::new("Invalid resource path"));
    }
    let path_components: Vec<&str> = path.split('/').collect();
    if path_components
        .iter()
        .any(|component| component.is_empty() || *component == "." || *component == "..")
    {
        return Err(ResourceError::new("Invalid resource path component"));
    }
    let path_obj = Path::new(path);
    if path_obj
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ResourceError::new("Resource path must be relative"));
    }

    let valid = match path_components.as_slice() {
        ["shelf.json"]
        | ["settings.json"]
        | ["bookmarks.json"]
        | ["reading-history.json"]
        | ["replacement-rules.json"]
        | ["discovery-favorites.json"] => true,
        ["progress", name] => filename_id(name, ".json").is_some(),
        ["search", name] => filename_id(name, ".json").is_some(),
        ["reading", name] => filename_id(name, ".json").is_some(),
        ["discovery", name] => filename_id(name, ".json").is_some(),
        ["media", id] => valid_id(id),
        ["books", book_id, "book.json"] => valid_id(book_id),
        ["books", book_id, "chapters", chapter_name] => {
            valid_id(book_id) && filename_id(chapter_name, ".html").is_some()
        }
        ["books", book_id, "assets", asset_id] => valid_id(book_id) && valid_asset_id(asset_id),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ResourceError::new(
            "Resource path is outside the public JSON/HTML resource set",
        ))
    }
}

fn filename_id<'a>(filename: &'a str, extension: &str) -> Option<&'a str> {
    let id = filename.strip_suffix(extension)?;
    valid_id(id).then_some(id)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id != "."
        && id != ".."
        && !id.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

fn validate_id(id: &str) -> Result<(), ResourceError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ResourceError::new("Invalid resource identifier"))
    }
}

fn validate_asset_id(id: &str) -> Result<(), ResourceError> {
    if valid_asset_id(id) {
        Ok(())
    } else {
        Err(ResourceError::new("Invalid or unsupported asset filename"))
    }
}

fn valid_asset_id(id: &str) -> bool {
    let extension = id.rsplit_once('.').map(|(_, extension)| extension);
    valid_id(id)
        && extension.is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "css"
                    | "bmp"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "gif"
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
                    | "pdf"
                    | "webm"
            )
        })
}

fn mime_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "xhtml" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "bmp" => "image/bmp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "mp4" => "video/mp4",
        "pdf" => "application/pdf",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

fn check_path_no_symlink(
    root: &Path,
    path: &Path,
    allow_missing_tail: bool,
) -> Result<(), ResourceError> {
    if !path.starts_with(root) {
        return Err(ResourceError::new("Resource path escaped its storage root"));
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ResourceError::new("Resource path escaped its storage root"))?;
    let mut current = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(ResourceError::new(
                "Resource path contains an invalid component",
            ));
        };
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ResourceError::new(
                    "Symbolic links are not served as resources",
                ));
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && allow_missing_tail
                    && components[index..].iter().all(|_| true) =>
            {
                // Once a parent is missing, subsequent paths cannot already be
                // symlinks. The atomic writer creates these directories below.
                break;
            }
            Err(error) => {
                return Err(ResourceError::new(format!(
                    "Cannot inspect resource path: {error}"
                )));
            }
        }
    }
    Ok(())
}

fn read_private_media_mappings(
    root: &Path,
    directory: &Path,
    strict: bool,
) -> Result<Vec<PrivateMediaMapping>, ResourceError> {
    check_path_no_symlink(root, directory, false)?;
    let entries = std::fs::read_dir(directory).map_err(|error| {
        ResourceError::new(format!("Cannot list private media mappings: {error}"))
    })?;
    let mut mappings = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ResourceError::new(format!("Cannot inspect private media mapping: {error}"))
        })?;
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            if strict {
                return Err(ResourceError::new(
                    "Restored media mapping has an invalid filename",
                ));
            }
            continue;
        };
        let Some(id) = filename.strip_suffix(".json") else {
            if strict {
                return Err(ResourceError::new(format!(
                    "Restored media mapping has an unexpected file: {filename}"
                )));
            }
            continue;
        };
        if !valid_id(id) {
            if strict {
                return Err(ResourceError::new(
                    "Restored media mapping filename contains an invalid ID",
                ));
            }
            continue;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => metadata,
            _ if strict => {
                return Err(ResourceError::new(format!(
                    "Restored media mapping {id} is not a regular file"
                )));
            }
            _ => continue,
        };
        if metadata.len() as usize > MAX_PRIVATE_MEDIA_MAPPING_BYTES {
            if strict {
                return Err(ResourceError::new(format!(
                    "Restored media mapping {id} exceeds its size limit"
                )));
            }
            eprintln!("Ignoring oversized private media mapping {id}");
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if strict => {
                return Err(ResourceError::new(format!(
                    "Cannot read restored media mapping {id}: {error}"
                )));
            }
            Err(_) => continue,
        };
        let mapping: PrivateMediaMapping = match serde_json::from_slice(&bytes) {
            Ok(mapping) => mapping,
            Err(error) if strict => {
                return Err(ResourceError::new(format!(
                    "Restored media mapping {id} is invalid JSON: {error}"
                )));
            }
            Err(_) => continue,
        };
        if mapping.schema_version != 1 || mapping.id != id {
            if strict {
                return Err(ResourceError::new(format!(
                    "Restored media mapping {id} has invalid schema or ID"
                )));
            }
            continue;
        }
        let headers = mapping
            .headers
            .iter()
            .map(|header| (header.name.clone(), header.value.clone()))
            .collect::<Vec<_>>();
        if MediaProxyRegistry::validate_mapping(&mapping.id, &mapping.upstream_url, &headers)
            .is_err()
        {
            if strict {
                return Err(ResourceError::new(format!(
                    "Restored media mapping {id} has invalid source or headers"
                )));
            }
            eprintln!("Ignoring invalid private media mapping {id}");
            continue;
        }
        mappings.push(mapping);
    }
    Ok(mappings)
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
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

fn text_to_paragraph_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs = Vec::new();
    let mut lines = Vec::new();
    for line in normalized.split('\n') {
        if line.trim().is_empty() {
            if !lines.is_empty() {
                paragraphs.push(std::mem::take(&mut lines));
            }
        } else {
            lines.push(line);
        }
    }
    if !lines.is_empty() {
        paragraphs.push(lines);
    }
    paragraphs
        .into_iter()
        .map(|paragraph| {
            let escaped = paragraph
                .into_iter()
                .map(escape_html)
                .collect::<Vec<_>>()
                .join("<br>\n");
            format!("<p>{escaped}</p>")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn chapter_document(content: &str, defaults: &ReaderDefaults) -> String {
    let style = safe_reader_style(defaults);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>:root{{--reader-font-family:{font_family};--reader-font-size:{font_size}px;--reader-line-height:{line_height};--reader-text-color:{text_color};--reader-background-color:{background_color};--reader-text-align:{text_align}}}html,body{{margin:0;min-height:100%;background:var(--reader-background-color);color:var(--reader-text-color);font-family:var(--reader-font-family);font-size:var(--reader-font-size);line-height:var(--reader-line-height)}}.chapter-content{{text-align:var(--reader-text-align);overflow-wrap:anywhere}}.chapter-content img{{max-width:100%;height:auto}}.chapter-content p{{margin:0 0 1em}}</style></head><body><article class=\"chapter-content\">{content}</article></body></html>",
        font_family = style.font_family,
        font_size = style.font_size_px,
        line_height = style.line_height,
        text_color = style.text_color,
        background_color = style.background_color,
        text_align = style.text_align,
    )
}

struct SafeReaderStyle {
    font_family: String,
    font_size_px: f32,
    line_height: f32,
    text_color: String,
    background_color: String,
    text_align: String,
}

fn safe_reader_style(defaults: &ReaderDefaults) -> SafeReaderStyle {
    let font_family = if !defaults.font_family.is_empty()
        && defaults.font_family.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, ' ' | ',' | '-' | '_' | '\'' | '"')
        }) {
        defaults.font_family.clone()
    } else {
        ReaderDefaults::default().font_family
    };
    SafeReaderStyle {
        font_family,
        font_size_px: if defaults.font_size_px.is_finite() {
            defaults.font_size_px.clamp(8.0, 72.0)
        } else {
            ReaderDefaults::default().font_size_px
        },
        line_height: if defaults.line_height.is_finite() {
            defaults.line_height.clamp(1.0, 3.0)
        } else {
            ReaderDefaults::default().line_height
        },
        text_color: safe_color(&defaults.text_color, "#3f3b34"),
        background_color: safe_color(&defaults.background_color, "#f7f3e9"),
        text_align: match defaults.text_align.as_str() {
            "left" | "right" | "center" | "justify" | "start" | "end" => {
                defaults.text_align.clone()
            }
            _ => "justify".to_owned(),
        },
    }
}

fn safe_color(color: &str, fallback: &str) -> String {
    let hex = color.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return format!("#{hex}");
    }
    let lower = color.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "black"
            | "white"
            | "red"
            | "green"
            | "blue"
            | "gray"
            | "grey"
            | "transparent"
            | "currentcolor"
            | "navy"
            | "teal"
            | "olive"
            | "maroon"
            | "purple"
            | "silver"
            | "orange"
            | "yellow"
    ) {
        return lower;
    }
    fallback.to_owned()
}

fn percent_encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(hex_digit(byte >> 4)));
            encoded.push(char::from(hex_digit(byte & 0x0f)));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        10..=15 => b'A' + value - 10,
        _ => unreachable!("a hexadecimal nibble is at most 15"),
    }
}

#[cfg(test)]
mod tests {
    use super::{ResourceRef, ResourceStore};
    use crate::models::ReaderDefaults;
    use crate::resource_transactions::{self, FileTransaction, Replacement};
    use axum::http::header::{CONTENT_RANGE, CONTENT_SECURITY_POLICY, CONTENT_TYPE, RANGE};
    use serde_json::json;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "legado-resource-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn json_updates_are_complete_and_refs_stay_stable_on_disk() {
        let root = temp_dir("atomic");
        let store = ResourceStore::open(&root).expect("open store");
        let shelf = store.shelf_ref();
        store
            .write_json_ref(&shelf, &json!({ "schemaVersion": 1, "books": [] }))
            .await
            .expect("first atomic write");
        store
            .write_json_ref(
                &shelf,
                &json!({ "schemaVersion": 1, "books": [{ "coverSrc": "resource://books/novel/book.json" }] }),
            )
            .await
            .expect("replace existing JSON");
        let persisted: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(root.join("shelf.json")).await.unwrap())
                .unwrap();
        assert_eq!(
            persisted["books"][0]["coverSrc"],
            "resource://books/novel/book.json"
        );
        assert_eq!(store.read_json_ref(&shelf).await.unwrap(), persisted);
        let leftovers = std::fs::read_dir(&root).unwrap().any(|entry| {
            entry.ok().is_some_and(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".resource-tmp")
            })
        });
        assert!(!leftovers, "atomic update should leave no temporary file");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[test]
    fn path_traversal_and_private_paths_are_rejected() {
        for reference in [
            "resource://../secrets.json",
            "resource://books/a/../../source-engine/data.json",
            "resource://source-engine/cache.json",
            "resource://books/a/chapters/%2e%2e.html",
            "resource://books/a\\secret/book.json",
        ] {
            assert!(ResourceRef::new(reference).is_err(), "accepted {reference}");
        }
        for reference in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
        ] {
            assert!(ResourceRef::new(reference).is_err(), "accepted {reference}");
        }
        assert_eq!(
            ResourceRef::new("https://covers.example/cover.jpg")
                .unwrap()
                .as_str(),
            "https://covers.example/cover.jpg"
        );
        assert_eq!(
            ResourceRef::new("resource://discovery-favorites.json")
                .unwrap()
                .path(),
            "discovery-favorites.json"
        );
    }

    #[tokio::test]
    async fn discovery_favorites_is_a_public_processed_json_resource() {
        let root = temp_dir("favorites-http");
        let store = ResourceStore::open(&root).expect("open store");
        let reference = store.discovery_favorites_ref();
        let document = json!({"schemaVersion": 1, "items": [{"sourceId": "source-a", "bookUrl": "https://example.test/book/1"}]});
        store
            .write_json_ref(&reference, &document)
            .await
            .expect("write discovery favorites");
        let server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("start resource server");
        let response = reqwest::get(server.url_for(&reference))
            .await
            .expect("fetch discovery favorites");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let response: serde_json::Value =
            serde_json::from_slice(&response.bytes().await.expect("read response bytes"))
                .expect("decode served favorites JSON");
        assert_eq!(response, document);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn rss_cleanup_removes_only_its_article_html_and_assets() {
        let root = temp_dir("rss-cleanup");
        let store = ResourceStore::open(&root).expect("open store");
        let rss_book_id = "rss-source-a";
        let rss_chapter = store
            .write_chapter_html(
                rss_book_id,
                "article-a",
                "<p>Cached article</p>",
                &ReaderDefaults::default(),
            )
            .await
            .expect("write RSS article");
        let rss_asset = store
            .write_asset(rss_book_id, "cover.png", b"processed image")
            .await
            .expect("write RSS image");
        let pseudo_book = store.book_ref(rss_book_id).unwrap();
        store
            .write_json_ref(&pseudo_book, &json!({"id": rss_book_id, "kind": "rss"}))
            .await
            .expect("write pseudo-book metadata");
        let pseudo_progress = store.progress_ref(rss_book_id).unwrap();
        store
            .write_json_ref(&pseudo_progress, &json!({"bookId": rss_book_id}))
            .await
            .expect("write pseudo-book progress");

        let normal_chapter = store
            .write_chapter_text(
                "ordinary-book",
                "chapter-a",
                "Keep this book",
                &ReaderDefaults::default(),
            )
            .await
            .expect("write normal chapter");
        let normal_asset = store
            .write_asset("ordinary-book", "cover.png", b"ordinary image")
            .await
            .expect("write normal image");

        store
            .remove_rss_chapter_resources("source-a")
            .await
            .expect("clean RSS cache");
        assert!(!root.join(rss_chapter.path()).exists());
        assert!(!root.join(rss_asset.path()).exists());
        assert!(root.join(pseudo_book.path()).is_file());
        assert!(root.join(pseudo_progress.path()).is_file());
        assert!(root.join(normal_chapter.path()).is_file());
        assert!(root.join(normal_asset.path()).is_file());
        assert!(store.remove_rss_chapter_resources("../bad").await.is_err());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn pdf_page_marker_and_asset_support_application_pdf_byte_ranges() {
        let root = temp_dir("pdf-page");
        let store = ResourceStore::open(&root).expect("open store");
        let pdf = b"%PDF-1.7\n0123456789abcdef\n%%EOF";
        let pdf_ref = store
            .write_asset("book-pdf", "document-1.pdf", pdf)
            .await
            .expect("write PDF asset");
        let chapter = store
            .write_pdf_page(
                "book-pdf",
                "page-0003",
                "document-1.pdf",
                2,
                "page-fit",
                &crate::models::ReaderDefaults::default(),
            )
            .await
            .expect("write PDF page marker");
        let html = tokio::fs::read_to_string(root.join(chapter.path()))
            .await
            .expect("read marker HTML");
        assert!(html.contains("data-legado-document=\"pdf-page\""));
        assert!(html.contains("data-page-index=\"2\""));
        assert!(html.contains("data-default-zoom=\"page-fit\""));
        assert!(html.contains("data-legado-pdf-src href=\"../assets/document-1.pdf\""));

        let server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("start resource server");
        let response = reqwest::get(server.url_for(&pdf_ref))
            .await
            .expect("fetch PDF asset");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/pdf");
        assert_eq!(response.bytes().await.unwrap(), pdf.as_slice());
        let partial = reqwest::Client::new()
            .get(server.url_for(&pdf_ref))
            .header(RANGE, "bytes=5-11")
            .send()
            .await
            .expect("fetch PDF byte range");
        assert_eq!(partial.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(partial.headers()[CONTENT_RANGE], "bytes 5-11/31");
        assert_eq!(partial.bytes().await.unwrap(), &pdf[5..=11]);
        assert!(store
            .write_pdf_page(
                "book-pdf",
                "bad-zoom",
                "document-1.pdf",
                0,
                "<script>",
                &crate::models::ReaderDefaults::default(),
            )
            .await
            .is_err());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn runtime_capability_urls_cannot_be_persisted_as_resource_sources() {
        let root = temp_dir("runtime-url");
        let store = ResourceStore::open(&root).expect("open store");
        let search = store.search_ref("runtime-url").unwrap();
        let transient_url = format!(
            "http://127.0.0.1:41821/r/{}/books/book-1/chapters/chapter-1.html",
            "a".repeat(32)
        );
        assert!(store
            .write_json_ref(&search, &json!({ "content": { "src": transient_url } }))
            .await
            .is_err());
        store
            .write_json_ref(
                &search,
                &json!({ "bookUrl": "http://127.0.0.1:41821/r/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/source/book" }),
            )
            .await
            .expect("source book URLs are not image/browser resource refs");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn text_html_escapes_markup_preserves_paragraphs_and_styles() {
        let root = temp_dir("html");
        let store = ResourceStore::open(&root).expect("open store");
        let reference = store
            .write_chapter_text(
                "book-1",
                "chapter-1",
                "<script>alert('x')</script> & text\ncontinued\n\nSecond paragraph",
                &crate::models::ReaderDefaults::default(),
            )
            .await
            .expect("write chapter");
        let html = tokio::fs::read_to_string(root.join(reference.path()))
            .await
            .unwrap();
        assert!(html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; text<br>"));
        assert!(html.contains("<p>Second paragraph</p>"));
        assert!(html.contains("--reader-font-size:19px"));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn processed_html_keeps_articles_and_removes_script_and_event_handlers() {
        let root = temp_dir("sanitize");
        let store = ResourceStore::open(&root).expect("open store");
        let reference = store
            .write_chapter_html(
                "book-1",
                "chapter-1",
                "<link rel=\"stylesheet\" href=\"../assets/book.css\"><p onclick=\"alert(1)\">Hello <a href=\"javascript:alert(2)\">there</a></p><img src=\"https://img.example/a.jpg\"><img src=\"../assets/epub-cover.png\"><script>alert(3)</script>",
                &crate::models::ReaderDefaults::default(),
            )
            .await
            .expect("write processed HTML");
        let html = tokio::fs::read_to_string(root.join(reference.path()))
            .await
            .unwrap();
        assert!(html.contains("<p>Hello"));
        assert!(html.contains("<img src=\"https://img.example/a.jpg\""));
        assert!(
            html.contains("<img src=\"../assets/epub-cover.png\""),
            "relative EPUB image URL was stripped: {html}"
        );
        assert!(html.contains("<link"));
        assert!(html.contains("../assets/book.css"));
        assert!(!html.contains("Content-Security-Policy"));
        assert!(!html.contains("onclick"));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("alert(3)"));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn loopback_http_serves_materialized_json_html_and_ranges() {
        let root = temp_dir("http");
        let store = ResourceStore::open(&root).expect("open store");
        let chapter = store
            .write_chapter_text(
                "书 book",
                "第一章",
                "Hello resource",
                &crate::models::ReaderDefaults::default(),
            )
            .await
            .expect("write chapter");
        let book = ResourceRef::new("resource://books/书 book/book.json").unwrap();
        store
            .write_json_ref(
                &book,
                &json!({
                    "id": "书 book",
                    "chapters": [{"id": "第一章", "title": "第一章", "src": chapter.as_str()}]
                }),
            )
            .await
            .expect("write book descriptor");
        let server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("start HTTP resource server");
        let client = reqwest::Client::new();
        let health = client
            .get(format!("http://{}/healthz", server.local_addr()))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), reqwest::StatusCode::OK);
        let health_body = health.bytes().await.unwrap();
        let health_json: serde_json::Value = serde_json::from_slice(&health_body).unwrap();
        assert_eq!(health_json["status"], "ok");

        let response = client
            .get(server.url_for(&book))
            .send()
            .await
            .expect("fetch JSON resource");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
        let body = response.bytes().await.unwrap();
        let received: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let chapter_url = received["chapters"][0]["src"].as_str().unwrap();
        assert!(chapter_url.starts_with(server.base_url()));
        assert!(chapter_url.contains("%E4%B9%A6%20book"));
        assert_eq!(
            tokio::fs::read_to_string(root.join(book.path()))
                .await
                .unwrap()
                .contains("resource://books/书 book/chapters/第一章.html"),
            true,
            "runtime HTTP URLs must not be written back to JSON"
        );

        let html = client.get(chapter_url).send().await.unwrap();
        assert_eq!(html.status(), reqwest::StatusCode::OK);
        assert_eq!(html.headers()[CONTENT_TYPE], "text/html; charset=utf-8");
        let csp = html.headers()[CONTENT_SECURITY_POLICY].to_str().unwrap();
        let resource_origin = format!("http://{}", server.local_addr());
        assert!(csp.contains("default-src 'none'"));
        assert!(csp.contains(&format!(
            "style-src 'self' 'unsafe-inline' {resource_origin}"
        )));
        assert!(csp.contains(&format!("font-src 'self' data: {resource_origin}")));
        assert!(csp.contains("media-src 'self' http: https:"));
        assert!(!csp.contains("script-src"));
        let html_text = html.text().await.unwrap();
        assert!(html_text.contains("<p>Hello resource</p>"));

        let partial = client
            .get(chapter_url)
            .header(RANGE, "bytes=0-9")
            .send()
            .await
            .unwrap();
        assert_eq!(partial.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert!(partial.headers()[CONTENT_RANGE]
            .to_str()
            .unwrap()
            .starts_with("bytes 0-9/"));
        assert_eq!(partial.bytes().await.unwrap().len(), 10);

        let multipart = client
            .get(chapter_url)
            .header(RANGE, "bytes=0-1,4-5")
            .send()
            .await
            .unwrap();
        assert_eq!(
            multipart.status(),
            reqwest::StatusCode::OK,
            "valid multipart requests are served as the full resource"
        );

        let unsatisfiable = client
            .get(chapter_url)
            .header(RANGE, "bytes=999999-")
            .send()
            .await
            .unwrap();
        assert_eq!(
            unsatisfiable.status(),
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE
        );

        let denied = client
            .get(format!("http://{}/r/wrong/shelf.json", server.local_addr()))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), reqwest::StatusCode::NOT_FOUND);

        server.shutdown().await.expect("stop server");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn atomic_json_updates_serialize_concurrent_read_modify_write() {
        let root = temp_dir("concurrent");
        let store = ResourceStore::open(&root).expect("open store");
        let counter = store.reading_ref("counter").unwrap();
        store
            .write_json_ref(&counter, &json!({ "count": 0 }))
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let store = store.clone();
            let counter = counter.clone();
            tasks.push(tokio::spawn(async move {
                store
                    .update_json_ref(&counter, |mut value| {
                        let count = value["count"].as_u64().unwrap_or_default();
                        value["count"] = json!(count + 1);
                        Ok(value)
                    })
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(store.read_json_ref(&counter).await.unwrap()["count"], 12);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn store_transaction_guard_serializes_two_file_commit_with_ordinary_update() {
        let root = temp_dir("transaction-writer-guard");
        let store = ResourceStore::open(&root).expect("open store");
        let counter = store.reading_ref("transaction-counter").unwrap();
        let companion = store.reading_ref("transaction-companion").unwrap();
        store
            .write_json_ref(&counter, &json!({"count": 0}))
            .await
            .unwrap();
        store
            .write_json_ref(&companion, &json!({"generation": "old"}))
            .await
            .unwrap();

        let writer_guard = store.transaction_writer_guard().await.unwrap();
        let old_counter = writer_guard.read_json_ref(&counter).unwrap();
        let tx_replacements = vec![
            writer_guard
                .public_json_replacement(
                    &counter,
                    &json!({"count": old_counter["count"].as_u64().unwrap() + 1}),
                )
                .unwrap(),
            writer_guard
                .public_json_replacement(&companion, &json!({"generation": "new"}))
                .unwrap(),
        ];

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let ordinary_store = store.clone();
        let ordinary_counter = counter.clone();
        let mut ordinary_update = tokio::spawn(async move {
            let _ = started_tx.send(());
            ordinary_store
                .update_json_ref(&ordinary_counter, |mut value| {
                    let count = value["count"].as_u64().unwrap_or_default();
                    value["count"] = json!(count + 1);
                    Ok(value)
                })
                .await
        });
        started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut ordinary_update)
                .await
                .is_err()
        );

        let tx_root = writer_guard.data_root().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let tx = FileTransaction::prepare(
                &tx_root,
                "store-two-file-commit",
                tx_replacements,
                vec![],
                &writer_guard,
            )
            .map_err(|error| error.to_string())?;
            tx.commit().map_err(|error| error.to_string())
        })
        .await
        .unwrap()
        .unwrap();

        ordinary_update.await.unwrap().unwrap();
        assert_eq!(store.read_json_ref(&counter).await.unwrap()["count"], 2);
        assert_eq!(
            store.read_json_ref(&companion).await.unwrap()["generation"],
            "new"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn pending_transaction_blocks_store_open_and_ordinary_writes_until_recovery() {
        let root = temp_dir("transaction-write-block");
        let store = ResourceStore::open(&root).expect("open store");
        let settings = store.settings_ref();
        let writer_guard = store.transaction_writer_guard().await.unwrap();
        let replacement = writer_guard
            .public_json_replacement(&settings, &json!({"generation": "pending"}))
            .unwrap();
        let tx = FileTransaction::prepare(
            writer_guard.data_root(),
            "leave-prepared-journal",
            vec![replacement],
            vec![],
            &writer_guard,
        )
        .unwrap();
        drop(tx);
        drop(writer_guard);

        let json_error = store
            .write_json_ref(&settings, &json!({"generation": "unsafe"}))
            .await
            .unwrap_err();
        assert!(json_error.to_string().contains("recovery succeeds"));
        let asset_error = store
            .write_asset("book-a", "cover.png", b"unsafe")
            .await
            .unwrap_err();
        assert!(asset_error.to_string().contains("recovery succeeds"));
        let Err(open_error) = ResourceStore::open(&root) else {
            panic!("opening a store with a pending transaction must fail closed");
        };
        assert!(open_error.to_string().contains("recovery succeeds"));

        resource_transactions::recover_all(&root).unwrap();
        let reopened = ResourceStore::open(&root).expect("open after explicit recovery");
        reopened
            .write_json_ref(&settings, &json!({"generation": "safe"}))
            .await
            .unwrap();
        assert_eq!(
            reopened.read_json_ref(&settings).await.unwrap()["generation"],
            "safe"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn transaction_replacements_use_public_reference_validation_only_for_public_json() {
        let root = temp_dir("transaction-json-validation");
        let store = ResourceStore::open(&root).expect("open store");
        let writer_guard = store.transaction_writer_guard().await.unwrap();
        let reference = store.reading_ref("runtime-url").unwrap();
        let runtime_url = format!(
            "http://127.0.0.1:41821/r/{}/books/book-1/cover.png",
            "b".repeat(32)
        );
        let error = writer_guard
            .public_json_replacement(&reference, &json!({"coverSrc": runtime_url}))
            .unwrap_err();
        assert!(error.to_string().contains("cannot be persisted"));
        assert!(writer_guard
            .public_json_replacement(&reference, &json!({"coverSrc": "javascript:alert(1)"}),)
            .is_err());

        assert!(Replacement::private_book_json(
            "book-a",
            br#"{"src":"engine-rule://source/internal"}"#.to_vec(),
        )
        .is_ok());
        drop(writer_guard);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn discovery_json_is_a_public_processed_resource() {
        let root = temp_dir("discovery");
        let store = ResourceStore::open(&root).expect("open store");
        let reference = store.discovery_ref("category-1").expect("discovery ref");
        store
            .write_json_ref(&reference, &json!({ "categories": [{ "id": "fiction" }] }))
            .await
            .expect("write processed discovery categories");
        let server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("start resource server");
        let response = reqwest::get(server.url_for(&reference)).await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
        let body: serde_json::Value =
            serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
        assert_eq!(body["categories"][0]["id"], "fiction");
        assert!(store.discovery_ref("../source-engine").is_err());
        server.shutdown().await.expect("stop server");
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
