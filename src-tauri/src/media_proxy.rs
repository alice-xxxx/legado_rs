//! Authenticated media streaming behind opaque, Rust-owned resource IDs.
//!
//! The public URL contains only a capability token and an opaque ID. The
//! upstream URL and authentication headers remain in this runtime registry.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::header::{
    ACCEPT, ACCEPT_LANGUAGE, ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH,
    CONTENT_RANGE, CONTENT_TYPE, ETAG, EXPIRES, IF_MODIFIED_SINCE, IF_NONE_MATCH, IF_RANGE,
    LAST_MODIFIED, RANGE,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode};
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use tokio::sync::RwLock;

use crate::resources::{ResourceError, ResourceRef};

const REQUEST_HEADERS: &[HeaderName] = &[
    RANGE,
    IF_RANGE,
    IF_NONE_MATCH,
    IF_MODIFIED_SINCE,
    ACCEPT,
    ACCEPT_LANGUAGE,
];

const RESPONSE_HEADERS: &[HeaderName] = &[
    CONTENT_TYPE,
    CONTENT_LENGTH,
    CONTENT_RANGE,
    ACCEPT_RANGES,
    CACHE_CONTROL,
    ETAG,
    LAST_MODIFIED,
    EXPIRES,
    CONTENT_DISPOSITION,
];

#[derive(Clone)]
pub(super) struct MediaProxyRegistry {
    client: Client,
    private_client: Client,
    entries: Arc<RwLock<HashMap<String, ProxyTarget>>>,
}

#[derive(Clone)]
struct ProxyTarget {
    url: Url,
    private_headers: reqwest::header::HeaderMap,
}

impl MediaProxyRegistry {
    pub(super) fn new() -> Result<Self, ResourceError> {
        let client = Client::builder()
            .redirect(Policy::limited(5))
            .build()
            .map_err(|error| {
                ResourceError::new(format!("Cannot create media HTTP client: {error}"))
            })?;
        let private_client = Client::builder()
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() > 5 {
                    return attempt.error(std::io::Error::other(
                        "Media upstream exceeded the redirect limit",
                    ));
                }
                let Some(origin) = attempt.previous().first() else {
                    return attempt
                        .error(std::io::Error::other("Media redirect has no initial URL"));
                };
                if same_origin(origin, attempt.url()) {
                    attempt.follow()
                } else {
                    // Custom credential headers (for example x-api-key) are
                    // not among reqwest's standard sensitive headers. Refuse
                    // any redirect that could carry them to another origin.
                    attempt.error(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Media redirect refused because private headers are origin-scoped",
                    ))
                }
            }))
            .build()
            .map_err(|error| {
                ResourceError::new(format!("Cannot create private media HTTP client: {error}"))
            })?;
        Ok(Self {
            client,
            private_client,
            entries: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub(super) async fn register(
        &self,
        opaque_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        let (reference, target) = Self::parse_target(opaque_id, upstream_url, private_headers)?;
        self.entries
            .write()
            .await
            .insert(opaque_id.to_owned(), target);
        Ok(reference)
    }

    pub(super) async fn replace_mappings(
        &self,
        mappings: &[(String, String, Vec<(String, String)>)],
    ) -> Result<(), ResourceError> {
        let mut replacement = HashMap::with_capacity(mappings.len());
        for (opaque_id, upstream_url, private_headers) in mappings {
            let (_, target) = Self::parse_target(opaque_id, upstream_url, private_headers)?;
            if replacement.insert(opaque_id.clone(), target).is_some() {
                return Err(ResourceError::new(
                    "Duplicate private media mapping identifier",
                ));
            }
        }
        *self.entries.write().await = replacement;
        Ok(())
    }

    pub(super) fn validate_mapping(
        opaque_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        Self::parse_target(opaque_id, upstream_url, private_headers).map(|(reference, _)| reference)
    }

    fn parse_target(
        opaque_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<(ResourceRef, ProxyTarget), ResourceError> {
        let reference = ResourceRef::new(format!("resource://media/{opaque_id}"))?;
        if upstream_url.len() > 16 * 1024
            || private_headers.len() > 64
            || private_headers
                .iter()
                .any(|(name, value)| name.len() > 256 || value.len() > 8 * 1024)
        {
            return Err(ResourceError::new(
                "Private media mapping exceeds supported limits",
            ));
        }
        let url = Url::parse(upstream_url)
            .map_err(|_| ResourceError::new("Media source must be a valid HTTP(S) URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ResourceError::new(
                "Media source must be a credential-free HTTP(S) URL",
            ));
        }

        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in private_headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| ResourceError::new("Invalid private media header name"))?;
            if forbidden_header(&name) {
                return Err(ResourceError::new(
                    "Private media headers cannot override HTTP transport headers",
                ));
            }
            let mut value = HeaderValue::from_str(value)
                .map_err(|_| ResourceError::new("Invalid private media header value"))?;
            // Prevent reqwest/debug output from displaying registered secrets.
            // Redirect handling is also restricted to the same origin below.
            value.set_sensitive(true);
            headers.append(name, value);
        }
        Ok((
            reference,
            ProxyTarget {
                url,
                private_headers: headers,
            },
        ))
    }

    pub(super) async fn unregister(&self, opaque_id: &str) {
        self.entries.write().await.remove(opaque_id);
    }

    pub(super) async fn serve(
        &self,
        opaque_id: &str,
        method: Method,
        client_headers: &HeaderMap,
    ) -> Response<Body> {
        if !matches!(method, Method::GET | Method::HEAD) {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        let Some(target) = self.entries.read().await.get(opaque_id).cloned() else {
            return empty_response(StatusCode::NOT_FOUND);
        };
        let is_head = method == Method::HEAD;
        let has_private_headers = !target.private_headers.is_empty();

        let mut request_headers = target.private_headers;
        for name in REQUEST_HEADERS {
            if let Some(value) = client_headers.get(name) {
                request_headers.insert(name.clone(), value.clone());
            }
        }
        let client = if has_private_headers {
            &self.private_client
        } else {
            &self.client
        };
        let result = client
            .request(method, target.url)
            .headers(request_headers)
            .send()
            .await;
        let upstream = match result {
            Ok(response) => response,
            Err(_) => {
                return text_response(
                    StatusCode::BAD_GATEWAY,
                    "Media upstream request failed; private-header redirects are limited to the registered origin.",
                )
            }
        };

        let status =
            StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let mut builder = Response::builder().status(status);
        for name in RESPONSE_HEADERS {
            if let Some(value) = upstream.headers().get(name) {
                builder = builder.header(name, value.clone());
            }
        }
        let body = if is_head {
            Body::empty()
        } else {
            // Keep the upstream body as a stream. Dropping the downstream body
            // drops this response stream, which cancels the upstream request.
            Body::from_stream(upstream.bytes_stream())
        };
        let mut response = match builder.body(body) {
            Ok(response) => response,
            Err(_) => return empty_response(StatusCode::BAD_GATEWAY),
        };
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
        response.headers_mut().insert(
            ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(
                "Accept-Ranges, Content-Length, Content-Range, Content-Type, ETag, Last-Modified",
            ),
        );
        response
    }
}

fn forbidden_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "content-length"
            | "host"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str().map(str::to_ascii_lowercase)
            == right.host_str().map(str::to_ascii_lowercase)
        && left.port_or_known_default() == right.port_or_known_default()
}

fn empty_response(status: StatusCode) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("status-only response is valid");
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

fn text_response(status: StatusCode, text: &'static str) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(text))
        .expect("static media error response is valid");
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::header::{CONTENT_RANGE, LOCATION};
    use axum::http::Uri;
    use axum::routing::any;
    use axum::Router;
    use std::convert::Infallible;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, Notify};
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_stream::StreamExt;

    #[derive(Clone)]
    struct FixtureState {
        expected_secret: Option<&'static str>,
        cross_redirect: Option<String>,
        cross_hits: Arc<AtomicUsize>,
        stream_cancelled: Arc<Notify>,
    }

    async fn fixture_handler(
        State(state): State<FixtureState>,
        method: Method,
        headers: HeaderMap,
        uri: Uri,
    ) -> Response<Body> {
        if let Some(expected) = state.expected_secret {
            if headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
                != Some(expected)
            {
                return Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Body::empty())
                    .unwrap();
            }
        }

        match uri.path() {
            "/redirect-same" => {
                return Response::builder()
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .header(LOCATION, "/media")
                    .body(Body::empty())
                    .unwrap();
            }
            "/redirect-cross" => {
                return Response::builder()
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .header(
                        LOCATION,
                        state.cross_redirect.as_deref().unwrap_or_default(),
                    )
                    .body(Body::empty())
                    .unwrap();
            }
            "/cross-target" => {
                state.cross_hits.fetch_add(1, Ordering::SeqCst);
                return Response::new(Body::from("unexpected cross-origin request"));
            }
            "/stream" => {
                let (sender, receiver) = mpsc::channel::<Bytes>(1);
                let cancelled = state.stream_cancelled.clone();
                tokio::spawn(async move {
                    let _ = sender.send(Bytes::from_static(b"first chunk")).await;
                    sender.closed().await;
                    cancelled.notify_one();
                });
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(Body::from_stream(
                        ReceiverStream::new(receiver).map(Ok::<_, Infallible>),
                    ))
                    .unwrap();
            }
            _ => {}
        }

        let partial = headers.get(RANGE).is_some();
        let body = if partial { "part" } else { "all media bytes" };
        let mut builder = Response::builder()
            .status(if partial {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            })
            .header(CONTENT_TYPE, "video/mp4")
            .header(CONTENT_LENGTH, body.len().to_string())
            .header(ACCEPT_RANGES, "bytes");
        if partial {
            builder = builder.header(CONTENT_RANGE, "bytes 2-5/15");
        }
        builder
            .body(if method == Method::HEAD {
                Body::empty()
            } else {
                Body::from(body)
            })
            .unwrap()
    }

    async fn start_fixture(state: FixtureState) -> SocketAddr {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/media", any(fixture_handler))
            .route("/redirect-same", any(fixture_handler))
            .route("/redirect-cross", any(fixture_handler))
            .route("/cross-target", any(fixture_handler))
            .route("/stream", any(fixture_handler))
            .with_state(state);
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        address
    }

    fn temp_store() -> (tempfile::TempDir, crate::resources::ResourceStore) {
        let directory = tempfile::tempdir().expect("temporary resource root");
        let store = crate::resources::ResourceStore::open(directory.path()).unwrap();
        (directory, store)
    }

    #[tokio::test]
    async fn media_proxy_streams_get_head_and_ranges_with_private_headers() {
        let state = FixtureState {
            expected_secret: Some("never-show-this"),
            cross_redirect: None,
            cross_hits: Arc::new(AtomicUsize::new(0)),
            stream_cancelled: Arc::new(Notify::new()),
        };
        let upstream = start_fixture(state).await;
        let (directory, store) = temp_store();
        let server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
        let headers = vec![("x-api-key".to_owned(), "never-show-this".to_owned())];
        let media = server
            .register_media_with_id(
                "fixture-media",
                &format!("http://{upstream}/media"),
                &headers,
            )
            .await
            .unwrap();
        assert_eq!(media.as_str(), "resource://media/fixture-media");
        let browser_url = server.url_for(&media);
        assert!(browser_url.contains("/media/fixture-media"));
        assert!(!browser_url.contains("never-show-this"));
        assert!(!browser_url.contains(&upstream.to_string()));

        let descriptor = store.search_ref("media-ref-test").unwrap();
        store
            .write_json_ref(
                &descriptor,
                &serde_json::json!({
                    "content": {
                        "resourceId": media.as_str(),
                        "src": media.as_str(),
                        "contentType": "video/mp4"
                    }
                }),
            )
            .await
            .unwrap();
        let public_json = Client::new()
            .get(server.url_for(&descriptor))
            .send()
            .await
            .unwrap();
        let public_json: serde_json::Value =
            serde_json::from_slice(&public_json.bytes().await.unwrap()).unwrap();
        assert_eq!(public_json["content"]["resourceId"], media.as_str());
        assert_eq!(public_json["content"]["src"], browser_url);
        let durable_json: serde_json::Value = serde_json::from_slice(
            &tokio::fs::read(directory.path().join("search/media-ref-test.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            durable_json["content"]["resourceId"],
            "resource://media/fixture-media"
        );
        assert_eq!(
            durable_json["content"]["src"],
            "resource://media/fixture-media"
        );

        let client = Client::new();
        let get = client.get(&browser_url).send().await.unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        assert_eq!(get.headers()[CONTENT_TYPE], "video/mp4");
        assert_eq!(get.bytes().await.unwrap(), "all media bytes");

        let head = client.head(&browser_url).send().await.unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[CONTENT_LENGTH], "15");
        assert!(head.bytes().await.unwrap().is_empty());

        let partial = client
            .get(&browser_url)
            .header(RANGE, "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(partial.headers()[CONTENT_RANGE], "bytes 2-5/15");
        assert_eq!(partial.bytes().await.unwrap(), "part");

        let partial_head = client
            .head(&browser_url)
            .header(RANGE, "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(partial_head.status(), StatusCode::PARTIAL_CONTENT);
        assert!(partial_head.bytes().await.unwrap().is_empty());

        let unknown_id = ResourceRef::new("resource://media/not-registered").unwrap();
        assert_eq!(
            client
                .get(server.url_for(&unknown_id))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
            "media route only serves Rust-registered opaque IDs"
        );

        let redirected = server
            .register_media_with_id(
                "same-origin-redirect",
                &format!("http://{upstream}/redirect-same"),
                &headers,
            )
            .await
            .unwrap();
        let redirected = client
            .get(server.url_for(&redirected))
            .send()
            .await
            .unwrap();
        assert_eq!(redirected.status(), StatusCode::OK);
        assert_eq!(redirected.bytes().await.unwrap(), "all media bytes");
    }

    #[tokio::test]
    async fn private_media_mappings_survive_restart_and_reload_after_restore() {
        let upstream = start_fixture(FixtureState {
            expected_secret: Some("durable-secret"),
            cross_redirect: None,
            cross_hits: Arc::new(AtomicUsize::new(0)),
            stream_cancelled: Arc::new(Notify::new()),
        })
        .await;
        let (directory, store) = temp_store();
        let first_server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
        let media = first_server
            .register_media_with_id(
                "durable-media",
                &format!("http://{upstream}/media"),
                &[("x-api-key".to_owned(), "durable-secret".to_owned())],
            )
            .await
            .unwrap();
        let descriptor = store.search_ref("durable-media-ref").unwrap();
        store
            .write_json_ref(
                &descriptor,
                &serde_json::json!({
                    "resourceId": media.as_str(),
                    "src": media.as_str(),
                    "contentType": "video/mp4"
                }),
            )
            .await
            .unwrap();

        let durable_json_path = directory.path().join("search/durable-media-ref.json");
        let durable_json = tokio::fs::read_to_string(&durable_json_path).await.unwrap();
        assert!(durable_json.contains("resource://media/durable-media"));
        assert!(!durable_json.contains("durable-secret"));
        assert!(!durable_json.contains(&first_server.base_url().to_owned()));
        let private_mapping_path = directory
            .path()
            .join("private-data/media-maps/durable-media.json");
        let private_mapping = tokio::fs::read_to_string(&private_mapping_path)
            .await
            .unwrap();
        assert!(private_mapping.contains("durable-secret"));
        assert!(!private_mapping.contains("/r/"));

        let first_url = first_server.url_for(&media);
        let first_response = Client::new().get(&first_url).send().await.unwrap();
        assert_eq!(first_response.status(), StatusCode::OK);
        assert_eq!(first_response.bytes().await.unwrap(), "all media bytes");
        first_server.shutdown().await.unwrap();

        let second_server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
        let restored_url = second_server.url_for(&media);
        let restored = Client::new().get(&restored_url).send().await.unwrap();
        assert_eq!(restored.status(), StatusCode::OK);
        assert_eq!(restored.bytes().await.unwrap(), "all media bytes");

        // Model a backup restore whose snapshot did not include this mapping.
        tokio::fs::remove_file(&private_mapping_path).await.unwrap();
        second_server
            .reload_private_media_mappings()
            .await
            .expect("atomically reload restored private mapping tree");
        let removed = Client::new().get(&restored_url).send().await.unwrap();
        assert_eq!(removed.status(), StatusCode::NOT_FOUND);
        let _ = tokio::fs::remove_dir_all(directory.path()).await;
    }

    #[tokio::test]
    async fn private_header_redirects_cannot_cross_origins() {
        let cross_hits = Arc::new(AtomicUsize::new(0));
        let cross_addr = start_fixture(FixtureState {
            expected_secret: None,
            cross_redirect: None,
            cross_hits: cross_hits.clone(),
            stream_cancelled: Arc::new(Notify::new()),
        })
        .await;
        let upstream = start_fixture(FixtureState {
            expected_secret: Some("private-value"),
            cross_redirect: Some(format!("http://{cross_addr}/cross-target")),
            cross_hits: Arc::new(AtomicUsize::new(0)),
            stream_cancelled: Arc::new(Notify::new()),
        })
        .await;
        let (_directory, store) = temp_store();
        let server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
        let media = server
            .register_media_with_id(
                "cross-origin-redirect",
                &format!("http://{upstream}/redirect-cross"),
                &[("x-api-key".to_owned(), "private-value".to_owned())],
            )
            .await
            .unwrap();
        let result = Client::new()
            .get(server.url_for(&media))
            .send()
            .await
            .unwrap();
        assert_eq!(result.status(), StatusCode::BAD_GATEWAY);
        assert!(result
            .text()
            .await
            .unwrap()
            .contains("private-header redirects"));
        assert_eq!(cross_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dropping_browser_stream_cancels_upstream_before_completion() {
        let cancelled = Arc::new(Notify::new());
        let upstream = start_fixture(FixtureState {
            expected_secret: None,
            cross_redirect: None,
            cross_hits: Arc::new(AtomicUsize::new(0)),
            stream_cancelled: cancelled.clone(),
        })
        .await;
        let (_directory, store) = temp_store();
        let server = store
            .start_http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
        let media = server
            .register_media_with_id("streaming-media", &format!("http://{upstream}/stream"), &[])
            .await
            .unwrap();
        let response = Client::new()
            .get(server.url_for(&media))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.bytes_stream();
        let first = tokio::time::timeout(Duration::from_secs(2), body.next())
            .await
            .expect("first chunk arrives without buffering the complete body")
            .unwrap()
            .unwrap();
        assert_eq!(first, "first chunk");
        drop(body);
        tokio::time::timeout(Duration::from_secs(2), cancelled.notified())
            .await
            .expect("dropping downstream stream closes upstream response stream");
    }
}
