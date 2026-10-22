//! 代理已登记的 HTTP 请求；地址和私有请求头留在 Rust 内。
use super::stream;
use crate::resources::ResourceRef;
use axum::{
    body::to_bytes,
    extract::{Path, State},
    http::{header::*, HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use reqwest::{redirect::Policy, Client, Url};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, OnceLock},
};

struct Target {
    url: Url,
    headers: HeaderMap,
    owner: String,
}
#[derive(Default)]
struct Targets {
    entries: HashMap<String, Arc<Target>>,
    children: HashMap<(String, Url), String>,
}

#[derive(Clone)]
pub(crate) struct Proxy {
    targets: Arc<Mutex<Targets>>,
    client: Client,
    private_client: Client,
}

impl Proxy {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            targets: Arc::new(Mutex::new(Targets::default())),
            client: client(false)?,
            private_client: client(true)?,
        })
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.targets.lock().unwrap().entries.contains_key(id)
    }

    pub(crate) fn register_with_id(
        &self,
        id: &str,
        url: &str,
        mut headers: HeaderMap,
    ) -> io::Result<()> {
        crate::resources::validation::validate_id(id).map_err(io::Error::other)?;
        let url = Url::parse(url).map_err(io::Error::other)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(io::Error::other("代理地址必须是无账号密码的 HTTP(S) 地址"));
        }
        if [HOST, CONNECTION, CONTENT_LENGTH, TRANSFER_ENCODING]
            .iter()
            .any(|name| headers.contains_key(name))
        {
            return Err(io::Error::other("代理请求头不能覆盖连接和传输字段"));
        }
        for value in headers.values_mut() {
            value.set_sensitive(true);
        }
        self.targets.lock().unwrap().entries.insert(
            id.to_owned(),
            Arc::new(Target {
                url,
                headers,
                owner: id.to_owned(),
            }),
        );
        Ok(())
    }

    pub(crate) fn clear(&self) {
        *self.targets.lock().unwrap() = Targets::default();
    }

    pub(super) fn routes(&self) -> Router {
        Router::new()
            .route("/{id}", get(read))
            .with_state(self.clone())
    }

    fn child(&self, parent: &Target, base: &Url, value: &str) -> Result<String, StatusCode> {
        let url = base
            .join(value.trim())
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
        let mut targets = self.targets.lock().unwrap();
        if !targets.entries.contains_key(&parent.owner) {
            return Err(StatusCode::NOT_FOUND);
        }
        let key = (parent.owner.clone(), url.clone());
        if let Some(id) = targets.children.get(&key) {
            return Ok(id.clone());
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        // 不把当前来源的私有请求头交给其他来源。
        let headers = if url.origin() == parent.url.origin() {
            parent.headers.clone()
        } else {
            HeaderMap::new()
        };
        targets.entries.insert(
            id.clone(),
            Arc::new(Target {
                url,
                headers,
                owner: parent.owner.clone(),
            }),
        );
        targets.children.insert(key, id.clone());
        Ok(id)
    }

    fn playlist(&self, target: &Target, base: &Url, text: &str) -> Result<String, StatusCode> {
        if !text.trim_start().starts_with("#EXTM3U") {
            return Err(StatusCode::BAD_GATEWAY);
        }
        static LINKS: OnceLock<regex::Regex> = OnceLock::new();
        let links = LINKS
            .get_or_init(|| regex::Regex::new(r#"(?m)^[^#\r\n][^\r\n]*|URI="([^"]+)""#).unwrap());
        let mut failure = None;
        let result = links.replace_all(text, |capture: &regex::Captures<'_>| {
            let value = capture
                .get(1)
                .unwrap_or_else(|| capture.get(0).unwrap())
                .as_str();
            if value.trim().is_empty() {
                return value.to_owned();
            }
            match self.child(target, base, value) {
                Ok(id) if capture.get(1).is_some() => format!("URI=\"{id}\""),
                Ok(id) => id,
                Err(error) => {
                    failure = Some(error);
                    String::new()
                }
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(result.into_owned())
    }
}

fn client(private: bool) -> io::Result<Client> {
    let mut builder = crate::http_client::raw_builder();
    if private {
        builder = builder.redirect(Policy::custom(|attempt| {
            if attempt.previous().len() < 10
                && attempt
                    .previous()
                    .first()
                    .is_some_and(|url| url.origin() == attempt.url().origin())
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }));
    }
    builder.build().map_err(io::Error::other)
}

async fn read(
    State(proxy): State<Proxy>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let target = proxy
        .targets
        .lock()
        .unwrap()
        .entries
        .get(&id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    let known_playlist = target.url.path().to_ascii_lowercase().ends_with(".m3u8");
    let client = if target.headers.is_empty() {
        &proxy.client
    } else {
        &proxy.private_client
    };
    let mut request_headers = target.headers.clone();
    if !known_playlist {
        for name in [RANGE, IF_RANGE, IF_NONE_MATCH, IF_MODIFIED_SINCE, ACCEPT] {
            if let Some(value) = headers.get(&name) {
                request_headers.entry(name).or_insert(value.clone());
            }
        }
    }
    let mut upstream = client
        .request(
            if known_playlist {
                Method::GET
            } else {
                method.clone()
            },
            target.url.clone(),
        )
        .headers(request_headers)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if upstream.status().is_redirection() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let playlist = known_playlist
        || upstream
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("mpegurl"));
    if playlist && upstream.status().is_success() {
        if !known_playlist && (method == Method::HEAD || headers.contains_key(RANGE)) {
            // 无扩展名的播放列表可能直到响应头到达才被识别。
            upstream = client
                .get(target.url.clone())
                .headers(target.headers.clone())
                .send()
                .await
                .map_err(|_| StatusCode::BAD_GATEWAY)?;
            if !upstream.status().is_success() {
                return Err(StatusCode::BAD_GATEWAY);
            }
        }
        let base = upstream.url().clone();
        let bytes = to_bytes(
            stream::body(&Method::GET, upstream.bytes_stream(), ()),
            2 * 1024 * 1024,
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| StatusCode::BAD_GATEWAY)?;
        let text = proxy.playlist(&target, &base, text)?;
        let length = text.len() as u64;
        return stream::seekable(
            std::io::Cursor::new(text.into_bytes()),
            length,
            "application/vnd.apple.mpegurl; charset=utf-8",
            &method,
            &headers,
            (),
        )
        .await
        .map(stream::no_store);
    }
    let status = upstream.status();
    let mut response_headers = HeaderMap::new();
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        CONTENT_RANGE,
        CONTENT_ENCODING,
        ACCEPT_RANGES,
        CACHE_CONTROL,
        ETAG,
        LAST_MODIFIED,
        EXPIRES,
    ] {
        if let Some(value) = upstream.headers().get(&name) {
            response_headers.insert(name, value.clone());
        }
    }
    let mut response = Response::new(stream::body(&method, upstream.bytes_stream(), target));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok(response)
}

impl crate::resources::Resources {
    pub(crate) async fn register_proxy(
        &self,
        book: &str,
        id: &str,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<ResourceRef, String> {
        let private = headers.iter().cloned().collect();
        let headers = headers
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.parse().map_err(|_| "无效请求头名称")?,
                    value.parse().map_err(|_| "无效请求头内容")?,
                ))
            })
            .collect::<Result<_, String>>()?;
        self.proxy
            .register_with_id(id, url, headers)
            .map_err(|error| error.to_string())?;
        let saved = self
            .database
            .save_media(crate::database::MediaRequest {
                id: id.to_owned(),
                book_id: book.to_owned(),
                url: url.to_owned(),
                headers: private,
            })
            .map_err(|error| error.to_string())?
            .await;
        if let Err(error) = saved {
            self.reload_proxy().await?;
            return Err(error.to_string());
        }
        ResourceRef::new(format!("resource://proxy/{id}")).map_err(|error| error.to_string())
    }
    pub(crate) async fn reload_proxy(&self) -> Result<(), String> {
        let database = self.database.clone();
        let requests = tokio::task::spawn_blocking(move || database.media())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        self.proxy.clear();
        for request in requests {
            let headers = request
                .headers
                .into_iter()
                .map(|(name, value)| {
                    Ok((
                        name.parse().map_err(|_| "无效代理请求头")?,
                        value.parse().map_err(|_| "无效代理请求头")?,
                    ))
                })
                .collect::<Result<_, String>>()?;
            self.proxy
                .register_with_id(&request.id, &request.url, headers)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
