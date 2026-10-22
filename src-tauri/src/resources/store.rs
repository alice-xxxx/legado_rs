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
    ReaderDefaults, SettingsDocument, CURRENT_SCHEMA_VERSION,
};

use super::json::validate_embedded_refs;
use super::reader_html::chapter_document;
use super::validation::{
    check_path_no_symlink, validate_asset_id, validate_id, validate_public_path,
};
use super::{ChapterVideoSource, ResourceError, ResourceRef};

struct StoreInner {
    root: PathBuf,
    database: crate::database::Database,
    asset_lock: Arc<Mutex<()>>,
    content_types: std::sync::RwLock<HashMap<String, String>>,
}

/// 原生资源文件及具名数据库接口的调用入口。
#[derive(Clone)]
pub struct ResourceStore {
    inner: Arc<StoreInner>,
}

impl ResourceStore {
    /// 打开关系数据库，不导入旧 JSON 文件。
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ResourceError> {
        std::fs::create_dir_all(root.as_ref())
            .map_err(|error| ResourceError::new(format!("Cannot create resource root: {error}")))?;
        let root = std::fs::canonicalize(root.as_ref()).map_err(|error| {
            ResourceError::new(format!("Cannot resolve resource root: {error}"))
        })?;

        let store = Self {
            inner: Arc::new(StoreInner {
                database: crate::database::Database::new(&root)?,
                root,
                asset_lock: Arc::new(Mutex::new(())),
                content_types: std::sync::RwLock::new(HashMap::new()),
            }),
        };

        let mut settings =
            serde_json::to_value(SettingsDocument::default()).expect("serializable default");
        settings["lastBackupAtMs"] = Value::Null;

        if crate::configuration::read(store.root(),"settings.json")?.is_none(){
            crate::configuration::write(store.root(),"settings.json",settings)?;
        }
        // Cache garbage collection is maintenance, not app-data recovery.
        // A locked, damaged or read-only orphan file must not prevent the
        // user from opening an otherwise valid library.
        Ok(store)
    }

    pub(crate) fn database(&self) -> &crate::database::Database {
        &self.inner.database
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// Coordinate immutable file publication and cache cleanup. SQLite state
    /// updates use their own transactions and do not take this filesystem guard.
    /// Callers retain the relevant book/chapter guard while preparing metadata.
    pub(crate) async fn transaction_writer_guard(
        &self,
    ) -> Result<ResourceStoreWriterGuard, ResourceError> {
        let guard = Arc::clone(&self.inner.asset_lock).lock_owned().await;

        Ok(ResourceStoreWriterGuard {
            store: self.clone(),
            _guard: guard,
        })
    }

    pub fn settings_ref(&self) -> ResourceRef {
        ResourceRef::from_validated_path("settings.json")
    }

    pub(crate) fn book_type(&self, book_id: &str) -> Result<String, ResourceError> {
        validate_id(book_id)?;
        if let Some(kind) = self.database().book_type(book_id)? {
            return Ok(kind);
        }
        if let Some(kind) = self.inner.content_types.read()
            .map_err(|_|ResourceError::new("Content type lock poisoned"))?.get(book_id) {
            return Ok(kind.clone());
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
        if !super::CONTENT_TYPES.contains(&kind) {
            return Err(ResourceError::new("Unsupported content type"));
        }
        let database=self.database().clone();
        let (id,target)=(book_id.to_owned(),kind.to_owned());
        database.bind_book_type(id,target)?.await?;
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

    pub fn dictionary_ref(&self, dictionary_id: &str) -> Result<ResourceRef, ResourceError> {
        validate_id(dictionary_id)?;
        ResourceRef::new(format!("resource://dictionary/{dictionary_id}.html"))
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

            check_path_no_symlink(&root, &write_path, true)?;
            atomic_replace(&write_path, &bytes).map_err(|error| {
                ResourceError::new(format!("Cannot atomically write resource asset: {error}"))
            })
        })
        .await
        .map_err(|error| ResourceError::new(format!("Asset storage worker failed: {error}")))??;
        Ok(reference)
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
        if crate::configuration::is_file(reference.path()) {
            return crate::configuration::read(self.root(),reference.path())?.ok_or_else(||ResourceError::new("设置文件不存在"));
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

    pub(crate) async fn read_book(&self,id:&str)->Result<Value,ResourceError>{
        let database=self.database().clone();
        let id=id.to_owned();
        tokio::task::spawn_blocking(move||{
            let book=database.book(&id)?.ok_or_else(||ResourceError::new("书籍不存在"))?;
            serde_json::to_value(book).map_err(|error|ResourceError::new(error.to_string()))
        }).await.map_err(|e|ResourceError::new(e.to_string()))?
    }
    /// 读取独立设置文件，首次使用时写入默认设置。
    pub async fn read_json_ref_or_default(
        &self,
        reference: &ResourceRef,
        default: Value,
    ) -> Result<Value, ResourceError> {
        let store = self.clone();
        let reference = reference.clone();
        tokio::task::spawn_blocking(move || {
            let value = match crate::configuration::read(store.root(),reference.path())? {
                Some(value) => value,
                None => {
                    if crate::configuration::is_file(reference.path()) {
                        crate::configuration::write(store.root(),reference.path(),default.clone())?;
                    }
                    default
                }
            };
            Ok(value)
        })
        .await
        .map_err(|error| ResourceError::new(error.to_string()))?
    }

    /// 原子发布正文文件或独立设置；资源地址按存储值保留。
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
        validate_embedded_refs(value)?;
        if crate::configuration::is_file(reference.path()) {
            let root = self.root().to_owned();
            let path = reference.path().to_owned();
            let value = value.clone();
            return tokio::task::spawn_blocking(move || crate::configuration::write(&root, &path, value))
                .await
                .map_err(|e| ResourceError::new(e.to_string()))?;
        }
        Err(ResourceError::new("业务数据请调用具名数据库 API"))
    }

    /// 更新独立设置文件；业务数据通过具名数据库接口修改。
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
                crate::configuration::update(
                    &root,
                    &path,
                    default.unwrap_or(Value::Null),
                    move |value| {
                        let value = update(value).map_err(ResourceError::new)?;
                        validate_embedded_refs(&value)?;
                        Ok(value)
                    },
                )
            })
            .await
            .map_err(|e| ResourceError::new(e.to_string()))?;
        }
        let _ = (reference,default,update);
        Err(ResourceError::new("业务数据不允许外部读改写闭包，请调用具名数据库 API"))
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



    pub async fn write_settings(&self, settings: &SettingsDocument) -> Result<(), ResourceError> {
        self.write_serializable(&self.settings_ref(), settings)
            .await
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
        mut source: std::fs::File,
        progress: crate::local_books::ImportProgress,
    ) -> Result<(), ResourceError> {
        validate_id(book_id)?;
        if !matches!(extension, "txt" | "epub" | "cbz" | "pdf") {
            return Err(ResourceError::new("Unsupported local original format"));
        }
        let extension = String::from(extension);
        let source_metadata = source.metadata().map_err(|error| {
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
        tokio::task::spawn_blocking(move || {

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

            AtomicFile::new(&target, atomicwrites::DisallowOverwrite)
                .write(|output| {
                    use std::io::{Read, Write};
                    let mut buffer = [0; 64 * 1024];
                    let mut copied = 0;
                    progress("copy", 0, source_metadata.len());
                    loop {
                        let count = source.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        output.write_all(&buffer[..count])?;
                        copied += count as u64;
                        if copied % (1024 * 1024) < count as u64 || copied == source_metadata.len()
                        {
                            progress("copy", copied, source_metadata.len());
                        }
                    }
                    output.sync_all()
                })
                .or_else(|error| {
                    // 原件按内容身份只发布一次；并发导入可能已先完成同一目标。
                    if std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_file()) {
                        Ok(())
                    } else {
                        Err(ResourceError::new(format!("Cannot atomically store local original: {error}")))
                    }
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
        let database=self.database().clone();
        let book_id=book_id.to_owned();
        let guard=self.inner.asset_lock.clone().lock_owned().await;
        let protected=crate::settings::protected_resources(self.root())?;
        let _guard=guard;
        database.remove_book_assets(book_id,protected)?.await?;
        Ok(())
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
            .join(reference.path()))
    }
}

/// An owned filesystem permit retained while publishing or removing assets.
/// Metadata writes are committed together by SQLite.
pub(crate) struct ResourceStoreWriterGuard {
    store: ResourceStore,
    _guard: OwnedMutexGuard<()>,
}

impl ResourceStoreWriterGuard {
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
