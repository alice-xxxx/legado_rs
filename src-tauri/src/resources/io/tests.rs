use super::*;
use axum::{http::HeaderMap, routing::get};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

struct Service {
    _root: tempfile::TempDir,
    cache: cache::Cache,
    settings: settings::Settings,
    proxy: proxy::Proxy,
    tts: tts::Tts,
    http: LocalHttp,
    client: reqwest::Client,
}

impl Service {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let cache = cache::Cache::new(root.path().join("cache")).await.unwrap();
        let settings = settings::Settings::new(root.path().to_owned())
            .await
            .unwrap();
        let proxy = proxy::Proxy::new().unwrap();
        let tts = tts::Tts::default();
        let http = start(cache.clone(), settings.clone(), proxy.clone(), tts.clone())
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            _root: root,
            cache,
            settings,
            proxy,
            tts,
            http,
            client,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.http.base_url)
    }
}

#[tokio::test]
async fn dispatch_access_and_file_ranges() {
    let service = Service::new().await;
    service
        .cache
        .write_file("text/book/1.txt", b"0123456789")
        .await
        .unwrap();
    service
        .settings
        .write(br#"{"theme":"dark"}"#)
        .await
        .unwrap();
    service
        .cache
        .write_file(
            "html/book/1.html",
            b"<img src=\"resource://cache/image/book/1.png\">",
        )
        .await
        .unwrap();
    let html = service
        .client
        .get(service.url("cache/html/book/1.html"))
        .send()
        .await
        .unwrap();
    assert!(html.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains("script-src 'none'"));
    assert!(html
        .text()
        .await
        .unwrap()
        .contains(&service.url("cache/image/book/1.png")));
    let response = service
        .client
        .get(service.url("settings.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/json; charset=utf-8"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.text().await.unwrap(), r#"{"theme":"dark"}"#);
    for path in [
        "cache/text/book/1.txt",
        "settings.json",
        "proxy/missing",
        "tts/missing",
    ] {
        let response = service.client.post(service.url(path)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            service
                .client
                .request(Method::OPTIONS, service.url(path))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        let mut url = reqwest::Url::parse(&service.url(path)).unwrap();
        url.set_path(&format!("/wrong/{path}"));
        assert_eq!(
            service.client.get(url).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
    for (range, expected) in [
        ("bytes=2-5", "2345"),
        ("bytes=7-", "789"),
        ("bytes=-3", "789"),
        ("bytes=-99", "0123456789"),
    ] {
        let response = service
            .client
            .get(service.url("cache/text/book/1.txt"))
            .header(header::RANGE, range)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.text().await.unwrap(), expected);
    }
    let response = service
        .client
        .get(service.url("cache/text/book/1.txt"))
        .header(header::RANGE, "bytes=99-")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */10");
    let response = service
        .client
        .head(service.url("cache/text/book/1.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
    assert!(response.bytes().await.unwrap().is_empty());
    assert!(service
        .cache
        .write_file("../escape.txt", b"bad")
        .await
        .is_err());
    assert_eq!(
        service
            .client
            .get(service.url("other.json"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn file_waits_are_scoped_to_the_same_file() {
    let service = Service::new().await;
    service
        .cache
        .write_file("video/1/large.mp4", &vec![0; 32 * 1024 * 1024])
        .await
        .unwrap();
    service
        .cache
        .write_file("text/2/other.txt", b"other")
        .await
        .unwrap();
    // 不消费大响应，让正在发送的文件保持读锁。
    let first = service
        .client
        .get(service.url("cache/video/1/large.mp4"))
        .send()
        .await
        .unwrap();
    let cache = service.cache.clone();
    let mut writer =
        tokio::spawn(async move { cache.write_file("video/1/large.mp4", b"complete").await });
    assert!(timeout(Duration::from_millis(100), &mut writer)
        .await
        .is_err());
    let client = service.client.clone();
    let url = service.url("cache/video/1/large.mp4");
    let mut next =
        tokio::spawn(async move { client.get(url).send().await.unwrap().text().await.unwrap() });
    assert!(timeout(Duration::from_millis(100), &mut next)
        .await
        .is_err());
    assert_eq!(
        service
            .client
            .get(service.url("cache/text/2/other.txt"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "other"
    );
    drop(first);
    timeout(Duration::from_secs(3), writer)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(3), next)
            .await
            .unwrap()
            .unwrap(),
        "complete"
    );
}

#[tokio::test]
async fn tts_stream_replay_finish_and_cancel() {
    let service = Service::new().await;
    let mut writer = service.tts.create("audio/mpeg").await.unwrap();
    let url = service.url(&format!("tts/{}", writer.id));
    let response = service.client.head(&url).send().await.unwrap();
    assert_eq!(response.headers()[header::ACCEPT_RANGES], "none");
    let mut playing = service
        .client
        .get(&url)
        .send()
        .await
        .unwrap()
        .bytes_stream();
    assert!(timeout(Duration::from_millis(50), playing.next())
        .await
        .is_err());
    writer.append(b"abc").await.unwrap();
    assert_eq!(playing.next().await.unwrap().unwrap(), "abc");
    let mut replay = service
        .client
        .get(&url)
        .send()
        .await
        .unwrap()
        .bytes_stream();
    assert_eq!(replay.next().await.unwrap().unwrap(), "abc");
    drop(replay);
    writer.append(b"def").await.unwrap();
    assert_eq!(playing.next().await.unwrap().unwrap(), "def");
    let id = writer.id.clone();
    drop(writer);
    assert!(playing.next().await.is_none());
    let response = service
        .client
        .get(&url)
        .header(header::RANGE, "bytes=2-4")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.bytes().await.unwrap(), "cde");
    assert!(service.tts.release(&id));
    assert_eq!(
        service.client.get(&url).send().await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let mut writer = service.tts.create("audio/mpeg").await.unwrap();
    let response = service
        .client
        .get(service.url(&format!("tts/{}", writer.id)))
        .send()
        .await
        .unwrap();
    service.tts.clear();
    assert!(response.bytes().await.unwrap().is_empty());
    assert!(writer.is_cancelled());
    assert_eq!(
        writer.append(b"late").await.unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[tokio::test]
async fn resume_preserves_urls_and_live_audio_connections() {
    let service = Service::new().await;
    service
        .cache
        .write_file("text/book/1.txt", b"cached")
        .await
        .unwrap();
    let mut writer = service.tts.create("audio/mpeg").await.unwrap();
    let mut audio = service
        .client
        .get(service.url(&format!("tts/{}", writer.id)))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    writer.append(b"before").await.unwrap();
    assert_eq!(audio.next().await.unwrap().unwrap(), "before");
    let base = service.http.base_url.clone();
    service.http.resume().unwrap();
    assert_eq!(service.http.base_url, base);
    assert_eq!(
        service
            .client
            .get(service.url("cache/text/book/1.txt"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "cached"
    );
    writer.append(b"after").await.unwrap();
    assert_eq!(audio.next().await.unwrap().unwrap(), "after");
    drop(writer);
    assert!(audio.next().await.is_none());
}

#[tokio::test]
async fn proxy_ranges_private_headers_and_hls_children() {
    async fn upstream(method: Method, headers: HeaderMap, uri: axum::http::Uri) -> Response {
        if headers.get("x-secret").and_then(|v| v.to_str().ok()) != Some("native") {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        match uri.path() {
            "/root.m3u8" => (
                [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128000\nnested/list.m3u8\n",
            )
                .into_response(),
            "/nested/list.m3u8" => (
                [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
                "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"../key\"\n#EXTINF:1,\n../chunk.ts\n",
            )
                .into_response(),
            "/redirect" => axum::response::Redirect::temporary("/chunk.ts").into_response(),
            "/key" => "key".into_response(),
            "/chunk.ts" => stream::seekable(
                std::io::Cursor::new(b"0123456789".to_vec()),
                10,
                "video/mp2t",
                &method,
                &headers,
                (),
            )
            .await
            .unwrap(),
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let foreign = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = format!("http://{}/foreign", foreign.local_addr().unwrap());
    let foreign_task = tokio::spawn(async move {
        axum::serve(foreign, Router::new().fallback(|| async { "foreign" }))
            .await
            .unwrap()
    });
    let router = Router::new()
        .route(
            "/cross",
            get(move || {
                let destination = destination.clone();
                async move { axum::response::Redirect::temporary(&destination) }
            }),
        )
        .fallback(upstream);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let service = Service::new().await;
    let headers = HeaderMap::from_iter([(
        "x-secret".parse().unwrap(),
        HeaderValue::from_static("native"),
    )]);
    let id = service
        .proxy
        .register(&format!("{upstream_url}/redirect"), headers.clone())
        .unwrap();
    let response = service
        .client
        .get(service.url(&format!("proxy/{id}")))
        .header("x-secret", "browser")
        .header(header::RANGE, "bytes=2-5")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
    assert_eq!(response.bytes().await.unwrap(), "2345");
    let private = service
        .proxy
        .register(&format!("{upstream_url}/cross"), headers.clone())
        .unwrap();
    assert_eq!(
        service
            .client
            .get(service.url(&format!("proxy/{private}")))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_GATEWAY
    );
    let public = service
        .proxy
        .register(&format!("{upstream_url}/cross"), HeaderMap::new())
        .unwrap();
    assert_eq!(
        service
            .client
            .get(service.url(&format!("proxy/{public}")))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "foreign"
    );
    let id = service
        .proxy
        .register(&format!("{upstream_url}/root.m3u8"), headers)
        .unwrap();
    let url = reqwest::Url::parse(&service.url(&format!("proxy/{id}"))).unwrap();
    let text = service
        .client
        .get(url.clone())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!text.contains("nested/") && !text.contains(&upstream_url));
    assert_eq!(
        service
            .client
            .get(url.clone())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        text
    );
    let head = service.client.head(url.clone()).send().await.unwrap();
    assert_eq!(
        head.headers()[header::CONTENT_LENGTH],
        text.len().to_string()
    );
    let nested_url = url.join(text.lines().last().unwrap()).unwrap();
    let nested = service
        .client
        .get(nested_url.clone())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let key = nested
        .split("URI=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(
        service
            .client
            .get(url.join(key).unwrap())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "key"
    );
    let segment_url = url.join(nested.lines().last().unwrap()).unwrap();
    assert_eq!(
        service
            .client
            .get(segment_url.clone())
            .header(header::RANGE, "bytes=-3")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "789"
    );
    assert!(service.proxy.remove(&id));
    for url in [url, nested_url, segment_url] {
        assert_eq!(
            service.client.get(url).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
    assert!(service
        .proxy
        .register("file:///private", HeaderMap::new())
        .is_err());
    service.proxy.clear();
    task.abort();
    foreign_task.abort();
}
