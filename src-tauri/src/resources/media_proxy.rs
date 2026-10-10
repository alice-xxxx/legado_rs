//! 为私有媒体请求代理上游地址和请求头。
//! Authenticated media streaming behind opaque, Rust-owned resource IDs.
//!
//! The public URL contains only a capability token and an opaque ID. The
//! upstream URL and authentication headers remain in this runtime registry.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
use uuid::Uuid;

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

const MAX_PLAYLIST_BYTES: usize = 2 * 1024 * 1024;
const MAX_PLAYLIST_URIS: usize = 4096;
const MAX_CHILDREN_PER_ROOT: usize = 8192;
const CHILD_MAPPING_TTL: Duration = Duration::from_secs(30 * 60);
const HLS_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl; charset=utf-8";
const HLS_FETCH_ERROR: &str = "HLS playlist could not be fetched.";
const HLS_INVALID_ERROR: &str = "HLS playlist is invalid or exceeds supported limits.";
const HLS_UNSUPPORTED_ERROR: &str = "This HLS playlist uses an unsupported URI or feature.";
const HLS_DRM_ERROR: &str = "DRM-protected HLS is not supported by this player.";

#[derive(Clone)]
pub(super) struct MediaProxyRegistry {
    client: Client,
    private_client: Client,
    entries: Arc<RwLock<RegistryState>>,
}

#[derive(Default)]
struct RegistryState {
    targets: HashMap<String, ProxyTarget>,
    child_ids: HashMap<(String, String), String>,
}

#[derive(Clone)]
struct ProxyTarget {
    url: Url,
    private_headers: reqwest::header::HeaderMap,
    root_id: String,
    dynamic: bool,
    last_referenced: Instant,
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
            entries: Arc::new(RwLock::new(RegistryState::default())),
        })
    }

    pub(super) async fn register(
        &self,
        opaque_id: &str,
        upstream_url: &str,
        private_headers: &[(String, String)],
    ) -> Result<ResourceRef, ResourceError> {
        let (reference, target) = Self::parse_target(opaque_id, upstream_url, private_headers)?;
        let mut entries = self.entries.write().await;
        // Re-registering an identical media source must not invalidate HLS
        // child references that an active WebView player is still consuming.
        if entries.targets.get(opaque_id).is_some_and(|previous| {
            previous.url == target.url && previous.private_headers == target.private_headers
        }) {
            return Ok(reference);
        }
        remove_children_for_root(&mut entries, opaque_id);
        entries.targets.insert(opaque_id.to_owned(), target);
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
        let mut entries = self.entries.write().await;
        entries.targets = replacement;
        entries.child_ids.clear();
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
                root_id: opaque_id.to_owned(),
                dynamic: false,
                last_referenced: Instant::now(),
            },
        ))
    }

    pub(super) async fn unregister(&self, opaque_id: &str) {
        let mut entries = self.entries.write().await;
        entries.targets.remove(opaque_id);
        remove_children_for_root(&mut entries, opaque_id);
    }

    pub(super) async fn serve(
        &self,
        opaque_id: &str,
        method: Method,
        client_headers: &HeaderMap,
        base_url: &str,
    ) -> Response<Body> {
        if !matches!(method, Method::GET | Method::HEAD) {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        let target = {
            let mut entries = self.entries.write().await;
            let Some(target) = entries.targets.get(opaque_id).cloned() else {
                return empty_response(StatusCode::NOT_FOUND);
            };
            let now = Instant::now();
            if !target.dynamic {
                prune_expired_children(&mut entries, &target.root_id, now);
            }
            if let Some(root) = entries.targets.get_mut(&target.root_id) {
                root.last_referenced = now;
            }
            target
        };
        if target.url.as_str().len() > 16 * 1024 {
            return empty_response(StatusCode::NOT_FOUND);
        }
        let is_head = method == Method::HEAD;
        let is_known_playlist = is_playlist_url(&target.url);
        let has_sensitive_headers = has_sensitive_headers(&target.private_headers);

        let mut request_headers = target.private_headers.clone();
        for name in REQUEST_HEADERS {
            // A manifest is generated dynamically, so upstream validators and
            // byte ranges do not describe the browser-visible representation.
            if is_known_playlist
                && [RANGE, IF_RANGE, IF_NONE_MATCH, IF_MODIFIED_SINCE].contains(name)
            {
                continue;
            }
            if let Some(value) = client_headers.get(name) {
                request_headers.insert(name.clone(), value.clone());
            }
        }
        let client = if has_sensitive_headers {
            &self.private_client
        } else {
            &self.client
        };
        let upstream_method = if is_known_playlist && is_head {
            Method::GET
        } else {
            method.clone()
        };
        let result = client
            .request(upstream_method, target.url.clone())
            .headers(request_headers)
            .send()
            .await;
        let upstream = match result {
            Ok(response) => response,
            Err(_) => {
                if is_known_playlist {
                    let mut response = hls_error_response(StatusCode::BAD_GATEWAY, HLS_FETCH_ERROR);
                    if is_head {
                        *response.body_mut() = Body::empty();
                    }
                    return response;
                }
                return text_response(
                    StatusCode::BAD_GATEWAY,
                    "Media upstream request failed; private-header redirects are limited to the registered origin.",
                );
            }
        };

        let is_playlist = is_known_playlist || is_playlist_content_type(upstream.headers());
        if is_playlist {
            if is_head && !is_known_playlist {
                let status = StatusCode::from_u16(upstream.status().as_u16())
                    .unwrap_or(StatusCode::BAD_GATEWAY);
                return hls_head_response(status);
            }
            if !upstream.status().is_success() {
                let mut response = hls_error_response(
                    StatusCode::from_u16(upstream.status().as_u16())
                        .unwrap_or(StatusCode::BAD_GATEWAY),
                    HLS_FETCH_ERROR,
                );
                if is_head {
                    *response.body_mut() = Body::empty();
                }
                return response;
            }
            let final_url = upstream.url().clone();
            let bytes = match read_limited(upstream, MAX_PLAYLIST_BYTES).await {
                Ok(bytes) => bytes,
                Err(PlaylistReadError::TooLarge) => {
                    return hls_error_response(StatusCode::PAYLOAD_TOO_LARGE, HLS_INVALID_ERROR);
                }
                Err(PlaylistReadError::Upstream) => {
                    return hls_error_response(StatusCode::BAD_GATEWAY, HLS_FETCH_ERROR);
                }
            };
            let rewritten = match self
                .rewrite_playlist(&target, &final_url, &bytes, base_url)
                .await
            {
                Ok(body) => body,
                Err(PlaylistError::Drm) => {
                    let mut response =
                        hls_error_response(StatusCode::NOT_IMPLEMENTED, HLS_DRM_ERROR);
                    if is_head {
                        *response.body_mut() = Body::empty();
                    }
                    return response;
                }
                Err(PlaylistError::Unsupported) => {
                    let mut response =
                        hls_error_response(StatusCode::NOT_IMPLEMENTED, HLS_UNSUPPORTED_ERROR);
                    if is_head {
                        *response.body_mut() = Body::empty();
                    }
                    return response;
                }
                Err(PlaylistError::Invalid) => {
                    let mut response =
                        hls_error_response(StatusCode::BAD_GATEWAY, HLS_INVALID_ERROR);
                    if is_head {
                        *response.body_mut() = Body::empty();
                    }
                    return response;
                }
            };
            let mut response = super::byte_response(rewritten, HLS_CONTENT_TYPE, client_headers);
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response.headers_mut().remove(ETAG);
            response.headers_mut().remove(LAST_MODIFIED);
            if is_head {
                *response.body_mut() = Body::empty();
            }
            return add_cors(response);
        }

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
        let response = match builder.body(body) {
            Ok(response) => response,
            Err(_) => return empty_response(StatusCode::BAD_GATEWAY),
        };
        add_cors(response)
    }

    async fn rewrite_playlist(
        &self,
        parent: &ProxyTarget,
        final_url: &Url,
        bytes: &[u8],
        base_url: &str,
    ) -> Result<Vec<u8>, PlaylistError> {
        let text = std::str::from_utf8(bytes).map_err(|_| PlaylistError::Invalid)?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        if lines.first().map(|line| line.trim()) != Some("#EXTM3U") {
            return Err(PlaylistError::Invalid);
        }

        let mut references = 0usize;
        for line in &mut lines {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed == "#EXTM3U" {
                continue;
            }
            if trimmed.starts_with('#') {
                let tag = trimmed.split_once(':').map_or(trimmed, |(tag, _)| tag);
                let is_supported_tag = supported_uri_tag(tag);
                let attributes = trimmed.split_once(':').map(|(_, attributes)| attributes);
                let attrs = if is_supported_tag {
                    parse_uri_attributes(attributes.ok_or(PlaylistError::Invalid)?)?
                } else {
                    if attributes.is_some_and(contains_uri_attribute) {
                        return Err(PlaylistError::Unsupported);
                    }
                    Vec::new()
                };
                if matches!(tag, "#EXT-X-KEY" | "#EXT-X-SESSION-KEY") {
                    let key_attributes = attributes.ok_or(PlaylistError::Invalid)?;
                    let requires_uri = validate_key_attributes(key_attributes)?;
                    if requires_uri && attrs.is_empty() {
                        return Err(PlaylistError::Invalid);
                    }
                }
                if attrs.is_empty() {
                    continue;
                }
                references = references
                    .checked_add(attrs.len())
                    .ok_or(PlaylistError::Invalid)?;
                if references > MAX_PLAYLIST_URIS {
                    return Err(PlaylistError::Invalid);
                }
                let mut replacements = Vec::with_capacity(attrs.len());
                let attributes = trimmed
                    .split_once(':')
                    .map(|(_, attributes)| attributes)
                    .ok_or(PlaylistError::Invalid)?;
                let attr_offset = line.find(attributes).ok_or(PlaylistError::Invalid)?;
                for range in attrs {
                    let range = (range.start + attr_offset)..(range.end + attr_offset);
                    let raw = line.get(range.clone()).ok_or(PlaylistError::Invalid)?;
                    let child = resolve_child_url(final_url, raw)?;
                    let child_id = self.register_child(parent, final_url, child).await?;
                    replacements.push((range, format!("{base_url}media/{child_id}")));
                }
                for (range, replacement) in replacements.into_iter().rev() {
                    line.replace_range(range, &replacement);
                }
            } else {
                let offset = line.find(trimmed).ok_or(PlaylistError::Invalid)?;
                references = references.checked_add(1).ok_or(PlaylistError::Invalid)?;
                if references > MAX_PLAYLIST_URIS {
                    return Err(PlaylistError::Invalid);
                }
                let child = resolve_child_url(final_url, trimmed)?;
                let child_id = self.register_child(parent, final_url, child).await?;
                line.replace_range(
                    offset..offset + trimmed.len(),
                    &format!("{base_url}media/{child_id}"),
                );
            }
        }
        let output = lines.join("\n");
        if output.len() > MAX_PLAYLIST_BYTES + MAX_PLAYLIST_URIS * 128 {
            return Err(PlaylistError::Invalid);
        }
        Ok(output.into_bytes())
    }

    async fn register_child(
        &self,
        parent: &ProxyTarget,
        playlist_url: &Url,
        child_url: Url,
    ) -> Result<String, PlaylistError> {
        let key = (parent.root_id.clone(), child_url.as_str().to_owned());
        let private_headers = child_headers(parent, playlist_url, &child_url);
        let now = Instant::now();
        let mut entries = self.entries.write().await;
        if !entries
            .targets
            .get(&parent.root_id)
            .is_some_and(|root| !root.dynamic)
        {
            return Err(PlaylistError::Invalid);
        }
        prune_expired_children(&mut entries, &parent.root_id, now);
        if let Some(id) = entries.child_ids.get(&key).cloned() {
            if entries.targets.contains_key(&id) {
                return Ok(id);
            }
            entries.child_ids.remove(&key);
        }
        let child_count = entries
            .targets
            .values()
            .filter(|target| target.dynamic && target.root_id == parent.root_id)
            .count();
        if child_count >= MAX_CHILDREN_PER_ROOT {
            return Err(PlaylistError::Invalid);
        }
        let id = Uuid::new_v4().simple().to_string();
        let target = ProxyTarget {
            url: child_url,
            private_headers,
            root_id: parent.root_id.clone(),
            dynamic: true,
            last_referenced: now,
        };
        entries.targets.insert(id.clone(), target);
        entries.child_ids.insert(key, id.clone());
        Ok(id)
    }
}

#[derive(Clone, Copy)]
enum PlaylistError {
    Invalid,
    Unsupported,
    Drm,
}

enum PlaylistReadError {
    TooLarge,
    Upstream,
}

async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, PlaylistReadError> {
    let mut bytes = Vec::new();
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(PlaylistReadError::TooLarge);
    }
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| PlaylistReadError::Upstream)?;
        let Some(chunk) = chunk else {
            return Ok(bytes);
        };
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(PlaylistReadError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
}

fn parse_uri_attributes(attributes: &str) -> Result<Vec<std::ops::Range<usize>>, PlaylistError> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in attributes.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == b'"' {
            quoted = !quoted;
        } else if byte == b',' && !quoted {
            spans.push(start..index);
            start = index + 1;
        }
    }
    if quoted || escaped {
        return Err(PlaylistError::Invalid);
    }
    spans.push(start..attributes.len());

    let mut uris = Vec::new();
    for span in spans {
        let part = attributes.get(span.clone()).ok_or(PlaylistError::Invalid)?;
        let Some(equal) = part.find('=') else {
            if !part.trim().is_empty() {
                return Err(PlaylistError::Invalid);
            }
            continue;
        };
        let key = part[..equal].trim();
        if key.is_empty() {
            return Err(PlaylistError::Invalid);
        }
        if !key.eq_ignore_ascii_case("URI") {
            continue;
        }
        let value_start = span.start + equal + 1;
        let raw_value = attributes
            .get(value_start..span.end)
            .ok_or(PlaylistError::Invalid)?;
        let leading = raw_value.len() - raw_value.trim_start().len();
        let value = raw_value.trim();
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            let inner = &value[1..value.len() - 1];
            if inner.contains('"') || inner.contains('\\') || inner.trim().is_empty() {
                return Err(PlaylistError::Invalid);
            }
            let begin = value_start + leading + 1;
            uris.push(begin..begin + inner.len());
        } else {
            if value.is_empty() || value.contains('"') || value.chars().any(char::is_whitespace) {
                return Err(PlaylistError::Invalid);
            }
            let begin = value_start + leading;
            uris.push(begin..begin + value.len());
        }
    }
    Ok(uris)
}

fn parse_attributes(attributes: &str) -> Result<Vec<(String, String)>, PlaylistError> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in attributes.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == b'"' {
            quoted = !quoted;
        } else if byte == b',' && !quoted {
            spans.push(start..index);
            start = index + 1;
        }
    }
    if quoted || escaped {
        return Err(PlaylistError::Invalid);
    }
    spans.push(start..attributes.len());
    spans
        .into_iter()
        .filter(|span| !attributes[span.clone()].trim().is_empty())
        .map(|span| {
            let part = attributes.get(span).ok_or(PlaylistError::Invalid)?;
            let (key, value) = part.split_once('=').ok_or(PlaylistError::Invalid)?;
            let value = value.trim();
            let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                &value[1..value.len() - 1]
            } else {
                value
            };
            Ok((key.trim().to_ascii_uppercase(), value.to_owned()))
        })
        .collect()
}

fn validate_key_attributes(attributes: &str) -> Result<bool, PlaylistError> {
    let parsed = parse_attributes(attributes)?;
    let method = parsed
        .iter()
        .find(|(key, _)| key == "METHOD")
        .map(|(_, value)| value.as_str())
        .ok_or(PlaylistError::Invalid)?;
    let key_format = parsed
        .iter()
        .find(|(key, _)| key == "KEYFORMAT")
        .map_or("identity", |(_, value)| value.as_str());
    if !matches!(method, "NONE" | "AES-128") || !key_format.eq_ignore_ascii_case("identity") {
        return Err(PlaylistError::Drm);
    }
    Ok(method == "AES-128")
}

fn contains_uri_attribute(attributes: &str) -> bool {
    let mut start = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in attributes.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == b'"' {
            quoted = !quoted;
        } else if byte == b',' && !quoted {
            if is_uri_attribute(&attributes[start..index]) {
                return true;
            }
            start = index + 1;
        }
    }
    is_uri_attribute(&attributes[start..])
}

fn is_uri_attribute(attribute: &str) -> bool {
    attribute.split_once('=').is_some_and(|(name, value)| {
        let name = name.trim().to_ascii_uppercase();
        name == "URI"
            || name.ends_with("-URI")
            || value.to_ascii_lowercase().contains("http://")
            || value.to_ascii_lowercase().contains("https://")
    })
}

fn supported_uri_tag(tag: &str) -> bool {
    matches!(
        tag,
        "#EXT-X-KEY"
            | "#EXT-X-SESSION-KEY"
            | "#EXT-X-MAP"
            | "#EXT-X-MEDIA"
            | "#EXT-X-I-FRAME-STREAM-INF"
            | "#EXT-X-SESSION-DATA"
            | "#EXT-X-PART"
            | "#EXT-X-PRELOAD-HINT"
            | "#EXT-X-RENDITION-REPORT"
            | "#EXT-X-IMAGE-STREAM-INF"
    )
}

fn resolve_child_url(base: &Url, value: &str) -> Result<Url, PlaylistError> {
    let url = base.join(value).map_err(|_| PlaylistError::Unsupported)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.as_str().len() > 16 * 1024
    {
        return Err(PlaylistError::Unsupported);
    }
    Ok(url)
}

fn child_headers(
    parent: &ProxyTarget,
    playlist_url: &Url,
    child_url: &Url,
) -> reqwest::header::HeaderMap {
    if same_origin(playlist_url, child_url) {
        return parent.private_headers.clone();
    }
    let mut headers = reqwest::header::HeaderMap::new();
    for name in [reqwest::header::USER_AGENT, ACCEPT, ACCEPT_LANGUAGE] {
        if let Some(value) = parent.private_headers.get(&name) {
            let mut value = value.clone();
            value.set_sensitive(false);
            headers.insert(name, value);
        }
    }
    headers
}

fn has_sensitive_headers(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .keys()
        .any(|name| ![reqwest::header::USER_AGENT, ACCEPT, ACCEPT_LANGUAGE].contains(name))
}

fn is_playlist_url(url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    path.ends_with(".m3u8") || path.ends_with(".m3u")
}

fn is_playlist_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("mpegurl") || value.contains("x-mpegurl")
        })
}

fn prune_expired_children(entries: &mut RegistryState, root_id: &str, now: Instant) {
    let root_is_idle = entries
        .targets
        .get(root_id)
        .is_some_and(|root| now.duration_since(root.last_referenced) > CHILD_MAPPING_TTL);
    if root_is_idle {
        entries
            .targets
            .retain(|_, target| !target.dynamic || target.root_id != root_id);
        entries.child_ids.retain(|(root, _), _| root != root_id);
    }
}

fn remove_children_for_root(entries: &mut RegistryState, root_id: &str) {
    entries
        .targets
        .retain(|_, target| !target.dynamic || target.root_id != root_id);
    entries.child_ids.retain(|(root, _), _| root != root_id);
}

fn hls_error_response(status: StatusCode, message: &'static str) -> Response<Body> {
    let mut response = text_response(status, message);
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    add_cors(response)
}

fn hls_head_response(status: StatusCode) -> Response<Body> {
    let response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, HLS_CONTENT_TYPE)
        .header(CACHE_CONTROL, "no-store")
        .body(Body::empty())
        .expect("static HLS HEAD response is valid");
    add_cors(response)
}

fn add_cors(mut response: Response<Body>) -> Response<Body> {
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
