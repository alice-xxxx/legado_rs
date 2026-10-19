//! 管理 loopback HTTP 服务、私有媒体映射和 HTTP 响应。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::task::Poll;

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
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::sync::{oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use http_range_header::parse_range_header;

use super::json::{materialize_html_resource_refs, materialize_json};
use super::media_proxy::MediaProxyRegistry;
use super::tts_stream::{TtsStreamProgress, TtsStreamRegistry, TtsStreamWriter};
use super::validation::{check_path_no_symlink, mime_type, percent_encode_path, valid_id};
use super::{ResourceError, ResourceRef, ResourceStore};

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
    tts_streams: TtsStreamRegistry,
    root: PathBuf,
    store: ResourceStore,
    media_update_lock: Mutex<()>,
    listener: watch::Sender<Option<TcpListener>>,
    runtime: tokio::runtime::Handle,
    shutdown_sender: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl ResourceServer {
    pub async fn create_tts_stream(
        &self,
        progress: TtsStreamProgress,
    ) -> Result<TtsStreamWriter, ResourceError> {
        self.tts_streams.create(&self.base_url, progress).await
    }

    pub async fn tts_stream_progress(&self, id: &str) -> Option<TtsStreamProgress> {
        self.tts_streams.progress(id).await
    }

    pub async fn update_tts_stream(&self, id: &str, progress: TtsStreamProgress) {
        self.tts_streams.update(id, progress).await;
    }

    pub async fn release_tts_stream(&self, id: &str) -> bool {
        self.tts_streams.release(id).await
    }

    pub async fn clear_tts_streams(&self) {
        self.tts_streams.clear().await;
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// A listening socket can outlive its OS resources across suspension (iOS
    /// need not report an accept error). Rebind before the foreground event
    /// returns; keep every published URL, media mapping and TTS stream intact.
    pub fn resume_http(&self) -> Result<(), ResourceError> {
        let _runtime = self.runtime.enter();
        let mut result = Ok(());
        self.listener.send_modify(|listener| {
            // Drop the old socket before binding the same port. The watch write
            // lock also excludes an accept poll during the replacement.
            *listener = None;
            result = (|| {
                let socket = if self.local_addr.is_ipv4() {
                    TcpSocket::new_v4()?
                } else {
                    TcpSocket::new_v6()?
                };
                // Match TcpListener::bind's Unix behavior, including sockets
                // whose accepted connections are still playing background audio.
                #[cfg(unix)]
                socket.set_reuseaddr(true)?;
                socket.bind(self.local_addr)?;
                *listener = Some(socket.listen(1024)?);
                Ok::<_, std::io::Error>(())
            })()
            .map_err(|error| {
                ResourceError::new(format!("Cannot resume resource listener: {error}"))
            });
        });
        result
    }

    pub fn url_for(&self, reference: &ResourceRef) -> String {
        if !reference.is_local() {
            return reference.as_str().to_owned();
        }
        format!("{}{}", self.base_url, percent_encode_path(reference.path()))
    }

    /// Register a private upstream media request. The returned stable resource
    /// ref can be stored in a public descriptor; the URL and headers remain in
    /// the owner book's internal cache and are never exposed to the browser.
    pub async fn register_media(
        &self,
        book_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        let opaque_id = Uuid::new_v4().simple().to_string();
        self.register_media_with_id(book_id, &opaque_id, upstream_url, private_headers)
            .await
    }

    /// Restore a media ref whose private mapping has been loaded by Rust from
    /// protected storage. `opaque_id` must be safe and is validated as a ref.
    pub async fn register_media_with_id(
        &self,
        book_id: &str,
        opaque_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        if !valid_id(book_id) {
            return Err(ResourceError::new("Invalid media owner book ID"));
        }
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
        self.write_private_media_mapping(book_id, mapping).await?;
        self.media_proxy
            .register(opaque_id, upstream_url, private_headers)
            .await?;
        Ok(reference)
    }

    pub(crate) async fn contains_media(&self, id: &str) -> bool {
        self.media_proxy.contains(id).await
    }

    pub async fn unregister_media(
        &self,
        book_id: &str,
        opaque_id: &str,
    ) -> Result<(), ResourceError> {
        if !valid_id(book_id) {
            return Err(ResourceError::new("Invalid media owner book ID"));
        }
        ResourceRef::new(format!("resource://media/{opaque_id}"))?;
        let _guard = self.media_update_lock.lock().await;
        self.delete_private_media_mapping(book_id, opaque_id)
            .await?;
        self.media_proxy.unregister(opaque_id).await;
        Ok(())
    }

    /// Replace the runtime media registry from the protected on-disk snapshot.
    /// Backup restore calls this after atomically replacing the app-data tree,
    /// so mappings added after the backup are removed and restored IDs point
    /// to the restored private URLs and headers.
    pub async fn reload_private_media_mappings(&self) -> Result<(), ResourceError> {
        self.store.reset_content_types();
        let _guard = self.media_update_lock.lock().await;
        let root = self.root.clone();
        let mappings = tokio::task::spawn_blocking(move || read_book_media_mappings(&root, true))
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
        book_id: &str,
        mapping: PrivateMediaMapping,
    ) -> Result<(), ResourceError> {
        let reference = self.store.book_ref(book_id)?;
        let db = self.store.database().clone();
        let path = reference.path().to_owned();
        let book_id = book_id.to_owned();
        let id = mapping.id;
        let entry = serde_json::json!({"url":mapping.upstream_url,"headers":mapping.headers.into_iter()
            .map(|header|(header.name,Value::String(header.value))).collect::<serde_json::Map<String,Value>>()});
        tokio::task::spawn_blocking(move || db.set_media(&path, &book_id, &id, Some(&entry)))
            .await
            .map_err(|e| ResourceError::new(e.to_string()))?
    }

    async fn delete_private_media_mapping(
        &self,
        book_id: &str,
        opaque_id: &str,
    ) -> Result<(), ResourceError> {
        let reference = self.store.book_ref(book_id)?;
        let db = self.store.database().clone();
        let path = reference.path().to_owned();
        let book_id = book_id.to_owned();
        let id = opaque_id.to_owned();
        tokio::task::spawn_blocking(move || db.set_media(&path, &book_id, &id, None))
            .await
            .map_err(|e| ResourceError::new(e.to_string()))?
    }

    /// Rewrite stable resource refs inside a JSON value to this server's
    /// capability URLs. This is a response-only transformation.
    pub fn materialize_json(&self, value: &mut Value) -> Result<(), ResourceError> {
        materialize_json(value, &self.base_url)
    }

    pub async fn shutdown(mut self) -> Result<(), ResourceError> {
        self.tts_streams.clear().await;
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
    tts_streams: TtsStreamRegistry,
}

/// Only the listening socket changes on resume. Axum keeps its router and the
/// existing connection tasks, so a background audio response is not cancelled.
struct ResourceListener {
    socket: watch::Receiver<Option<TcpListener>>,
    local_addr: SocketAddr,
}

impl axum::serve::Listener for ResourceListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (TcpStream, SocketAddr) {
        let socket = self.socket.clone();
        loop {
            let result = tokio::select! {
                biased;
                changed = self.socket.changed() => {
                    if changed.is_err() {
                        std::future::pending::<()>().await;
                    }
                    continue;
                }
                result = std::future::poll_fn(|cx| match socket.borrow().as_ref() {
                    Some(listener) => listener.poll_accept(cx),
                    None => Poll::Pending,
                }) => result,
            };
            match result {
                Ok(connection) => return connection,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    continue
                }
                Err(error) => {
                    eprintln!("[resource-listener] accept failed: {error}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.local_addr)
    }
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
    // Session audio is not a persistable filesystem resource. Resolve it after
    // capability authentication, before the public file-path allowlist.
    if let Some(id) = resource_path
        .strip_prefix("tts-stream/")
        .and_then(|name| name.strip_suffix(".mp3"))
    {
        if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return empty_response(StatusCode::NOT_FOUND);
        }
        return state.tts_streams.serve(id, method, &headers).await;
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

    if reference.path().ends_with(".json") {
        return empty_response(StatusCode::NOT_FOUND);
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

pub(super) fn set_cors_headers(headers: &mut HeaderMap) {
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

pub(super) fn empty_response(status: StatusCode) -> Response<Body> {
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

pub(super) async fn stream_response(
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
        let mappings_root = self.root().to_path_buf();
        let mappings =
            tokio::task::spawn_blocking(move || read_book_media_mappings(&mappings_root, false))
                .await
                .map_err(|error| {
                    ResourceError::new(format!("Private media mapping worker failed: {error}"))
                })??;
        let media_proxy = MediaProxyRegistry::new()?;
        let tts_streams = TtsStreamRegistry::default();
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
        let (listener_sender, socket) = watch::channel(Some(listener));
        let listener = ResourceListener { socket, local_addr };
        let token = Uuid::new_v4().simple().to_string();
        let base_url = format!("http://{local_addr}/r/{token}/");
        let state = HttpState {
            store: self.clone(),
            token,
            base_url: base_url.clone(),
            media_proxy: media_proxy.clone(),
            tts_streams: tts_streams.clone(),
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
            tts_streams,
            root: self.root().to_path_buf(),
            store: self.clone(),
            media_update_lock: Mutex::new(()),
            listener: listener_sender,
            runtime: tokio::runtime::Handle::current(),
            shutdown_sender: Some(shutdown_sender),
            task: Some(task),
        })
    }
}
fn read_book_media_mappings(
    root: &Path,
    strict: bool,
) -> Result<Vec<PrivateMediaMapping>, ResourceError> {
    let mut mappings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (key, value) in crate::database::Database::at(root).media()? {
        let Some(url) = value["url"].as_str() else {
            if strict {
                return Err(ResourceError::new("Private media entry has no URL"));
            }
            continue;
        };
        let mut private_headers = Vec::new();
        if let Some(headers) = value.get("headers") {
            let headers = headers
                .as_object()
                .ok_or_else(|| ResourceError::new("Private media headers must be an object"))?;
            for (name, value) in headers {
                let value = value
                    .as_str()
                    .ok_or_else(|| ResourceError::new("Private media header must be a string"))?;
                private_headers.push(PrivateMediaHeader {
                    name: name.clone(),
                    value: value.to_owned(),
                });
            }
        }
        let mapping = PrivateMediaMapping {
            schema_version: 1,
            id: key.clone(),
            upstream_url: url.to_owned(),
            headers: private_headers,
        };
        let headers = mapping
            .headers
            .iter()
            .map(|header| (header.name.clone(), header.value.clone()))
            .collect::<Vec<_>>();
        if mapping.id != key
            || mapping.schema_version != 1
            || MediaProxyRegistry::validate_mapping(&mapping.id, &mapping.upstream_url, &headers)
                .is_err()
        {
            if strict {
                return Err(ResourceError::new(
                    "Invalid private media mapping in content document",
                ));
            }
            continue;
        }
        if !seen.insert(mapping.id.clone()) {
            return Err(ResourceError::new("Duplicate media mapping identity"));
        }
        mappings.push(mapping);
    }
    Ok(mappings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn resume_http_restores_lost_listener_without_changing_published_resources() {
        let directory = tempfile::tempdir().unwrap();
        let store = ResourceStore::open(directory.path()).unwrap();
        let chapter = store
            .write_chapter_text("resume", "chapter", "chapter text")
            .await
            .unwrap();
        let settings = store.settings_ref();
        store
            .write_json_ref(&settings, &serde_json::json!({"chapter": {"src": chapter}}))
            .await
            .unwrap();
        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let url = server.url_for(&chapter);
        let chapter_url = server.url_for(&chapter);
        let client = client();
        let original = client.get(&url).send().await.unwrap().text().await.unwrap();
        assert_eq!(original,"chapter text");
        assert_eq!(client.get(server.url_for(&settings)).send().await.unwrap().status(),StatusCode::NOT_FOUND);

        for _ in 0..3 {
            // Simulate loss of the listening socket, with the app/service and
            // frontend descriptors still alive. Repeating fetch cannot fix it.
            server.listener.send_modify(|listener| *listener = None);
            assert!(client.get(&url).send().await.is_err());
            server.resume_http().unwrap();
            assert_eq!(server.url_for(&chapter), url);
            let response = client.get(&url).send().await.unwrap();
            assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
            assert_eq!(response.text().await.unwrap(), original);
            let response = client
                .get(&chapter_url)
                .header(RANGE, "bytes=0-6")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            assert_eq!(response.text().await.unwrap(), "chapter");
            let response = client.head(&chapter_url).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[CONTENT_LENGTH], "12");
            assert_eq!(
                client
                    .get(format!(
                        "http://{}/r/wrong/settings.json",
                        server.local_addr()
                    ))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn resume_http_preserves_live_tts_response_and_reconnect_range() {
        let directory = tempfile::tempdir().unwrap();
        let store = ResourceStore::open(directory.path()).unwrap();
        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let mut writer = server
            .create_tts_stream(TtsStreamProgress {
                boundaries: vec![],
                state: "running",
                next_index: 0,
                skipped: 0,
                error: None,
            })
            .await
            .unwrap();
        let url = writer.url.clone();
        let id = writer.id.clone();
        writer.append(b"before").await.unwrap();
        let client = client();
        let mut response = client.get(&url).send().await.unwrap();
        assert_eq!(response.chunk().await.unwrap().unwrap().as_ref(), b"before");
        server.resume_http().unwrap();
        writer.append(b"after").await.unwrap();
        assert_eq!(response.chunk().await.unwrap().unwrap().as_ref(), b"after");
        assert!(!*writer.cancellation.borrow());
        assert!(server.tts_stream_progress(&id).await.is_some());
        drop(writer);
        assert!(response.chunk().await.unwrap().is_none());
        let response = client
            .get(&url)
            .header(RANGE, "bytes=6-")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.text().await.unwrap(), "after");
        server.shutdown().await.unwrap();
    }
}
