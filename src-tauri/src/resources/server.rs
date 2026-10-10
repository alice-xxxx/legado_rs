//! 管理 loopback HTTP 服务、私有媒体映射和 HTTP 响应。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

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
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use http_range_header::parse_range_header;

use super::json::{materialize_html_resource_refs, materialize_json};
use super::media_proxy::MediaProxyRegistry;
use super::store::atomic_replace;
use super::validation::{check_path_no_symlink, mime_type, percent_encode_path, valid_id};
use super::{ResourceError, ResourceRef, ResourceStore};

const MAX_PRIVATE_MEDIA_MAPPING_BYTES: usize = 1024 * 1024;

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

    /// Validate a media mapping before committing any sibling resolution
    /// choices. This keeps one malformed optional source from preventing the
    /// chapter's other playable choices from being registered.
    pub(crate) fn validate_media_mapping(
        &self,
        opaque_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        MediaProxyRegistry::validate_mapping(opaque_id, upstream_url, private_headers)
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
        return state
            .media_proxy
            .serve(media_id, method, &headers, &state.base_url)
            .await;
    }
    let path = match state.store.path_for(&reference) {
        Ok(path) => path,
        Err(_) => return empty_response(StatusCode::NOT_FOUND),
    };

    // The mutable canonical source file is backend-owned. Source management
    // receives immutable definitionJson snapshots, never this working file.
    if reference.path() == "sources.json" {
        return empty_response(StatusCode::NOT_FOUND);
    }

    if reference.path().ends_with(".json") {
        let mut json = match state.store.read_json_ref(&reference).await {
            Ok(json) => json,
            Err(_) => return empty_response(StatusCode::NOT_FOUND),
        };
        if reference.path() == "shelf.json"
            && materialize_shelf_book_sources(&mut json, &state.base_url).is_err()
        {
            return empty_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
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

    let content_type = mime_type(reference.path());
    if content_type.starts_with("text/html") {
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(_) => return empty_response(StatusCode::NOT_FOUND),
        };
        let html = match String::from_utf8(bytes) {
            Ok(html) => html,
            Err(_) => return empty_response(StatusCode::INTERNAL_SERVER_ERROR),
        };
        let html = match materialize_html_resource_refs(&html, &state.base_url) {
            Ok(html) => html,
            Err(_) => return empty_response(StatusCode::INTERNAL_SERVER_ERROR),
        };
        let mut response = byte_response(html.into_bytes(), content_type, &headers);
        set_resource_content_security_policy(
            response.headers_mut(),
            content_type,
            state.base_url.split("/r/").next().unwrap_or_default(),
        );
        return response;
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
        content_type,
        state.base_url.split("/r/").next().unwrap_or_default(),
    )
    .await
}

fn materialize_shelf_book_sources(shelf: &mut Value, base_url: &str) -> Result<(), ResourceError> {
    let Some(books) = shelf.get_mut("books").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for book in books {
        let book_id = book
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ResourceError::new("Shelf entry has no book ID"))?;
        let reference = ResourceRef::new(format!("resource://books/{book_id}/book.json"))?;
        book["bookSrc"] = Value::String(format!(
            "{}{}",
            base_url,
            percent_encode_path(reference.path())
        ));
    }
    Ok(())
}

fn set_cors_headers(headers: &mut HeaderMap) {
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Accept-Ranges, Content-Length, Content-Range"),
    );
}

fn set_resource_content_security_policy(
    headers: &mut HeaderMap,
    content_type: &str,
    resource_origin: &str,
) {
    let policy = if content_type.starts_with("text/html") {
        Some(format!(
            "default-src 'none'; img-src 'self' data: http: https:; style-src 'self' 'unsafe-inline' {resource_origin}; font-src 'self' data: {resource_origin}; media-src 'self' http: https:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
        ))
    } else if content_type == "image/svg+xml" {
        Some(format!(
            "default-src 'none'; script-src 'none'; sandbox; img-src {resource_origin} data:; style-src 'unsafe-inline' {resource_origin}; font-src {resource_origin} data:; media-src {resource_origin}; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
        ))
    } else {
        None
    };
    if let Some(policy) = policy {
        if let Ok(value) = HeaderValue::from_str(&policy) {
            headers.insert(CONTENT_SECURITY_POLICY, value);
        }
    }
}

fn empty_response(status: StatusCode) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("status-only response is valid");
    set_cors_headers(response.headers_mut());
    response
}

pub(super) fn byte_response(
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
    } else if content_type == "image/svg+xml" {
        let policy = format!(
            "default-src 'none'; script-src 'none'; sandbox; img-src {resource_origin} data:; style-src 'unsafe-inline' {resource_origin}; font-src {resource_origin} data:; media-src {resource_origin}; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
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

impl ResourceStore {
    /// Start a read-only local HTTP service for public JSON and chapter HTML.
    /// Only loopback addresses are accepted; each server gets its own random
    /// path capability so resources are not enumerable by filename alone.
    pub async fn start_http(&self, bind: SocketAddr) -> Result<ResourceServer, ResourceError> {
        if !bind.ip().is_loopback() {
            return Err(ResourceError::new(
                "Resource server must bind to a loopback address",
            ));
        }
        let private_media_dir = self.root().join("private-data").join("media-maps");
        let media_dir_root = self.root().to_path_buf();
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

        let mappings_root = self.root().to_path_buf();
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
