use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Engine {
    content: std::sync::OnceLock<String>,
    revision: AtomicUsize,
    slow: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    token: std::sync::OnceLock<tokio_util::sync::CancellationToken>,
}

impl SourceExecutor for Engine {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            if request.operation == "resolveMedia" {
                return Ok(json!({"url":request.media_url,"headers":{"X-Fixture":"private"}}));
            }
            assert_eq!(request.operation, "content");
            if self.slow.load(Ordering::SeqCst) && request.chapter.as_ref().unwrap()["index"] == 0 {
                let _ = self.token.set(request.cancellation.unwrap());
                self.entered.notify_one();
                self.release.notified().await;
            }
            if let Some(content) = self.content.get() {
                return Ok(json!(content));
            }
            let revision = self.revision.load(Ordering::SeqCst);
            Ok(if revision == 2 {
                json!("")
            } else {
                json!(format!("正文版本{revision}"))
            })
        })
    }
}

struct Origin {
    url: String,
    calls: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Origin {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn origin() -> Origin {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let requests = calls.clone();
    let blocked = entered.clone();
    let router = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let requests = requests.clone();
        let blocked = blocked.clone();
        async move {
            if request.headers().get("X-Fixture").and_then(|value|value.to_str().ok()) != Some("private") {
                return axum::http::Response::builder().status(401).body(axum::body::Body::empty()).unwrap();
            }
            requests.fetch_add(1, Ordering::SeqCst);
            let (mime, body) = match request.uri().path() {
                "/image" => ("image/png", "complete-image"),
                "/audio" => ("audio/mpeg", "complete-audio"),
                "/master.m3u8" => ("application/vnd.apple.mpegurl", "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nmedia.m3u8\n"),
                "/media.m3u8" => ("application/vnd.apple.mpegurl", "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nsegment.ts\n#EXT-X-ENDLIST\n"),
                "/init.mp4" => ("video/mp4", "complete-init"),
                "/segment.ts" => ("video/mp2t", "complete-segment"),
                "/slow" => { blocked.notify_one(); return std::future::pending().await; }
                _ => return axum::http::Response::builder().status(500).body(axum::body::Body::empty()).unwrap(),
            };
            axum::http::Response::builder().header("Content-Type", mime).body(axum::body::Body::from(body)).unwrap()
        }
    });
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Origin {
        url,
        calls,
        entered,
        server,
    }
}

async fn set_media(service: &ApplicationService, engine: &Engine, source_type: u32, url: String) {
    let mut source = service.find_source("source").await.unwrap();
    source.source["bookSourceType"] = json!(source_type);
    service.write_sources(&[source]).await.unwrap();
    service
        .store
        .bind_book_type(
            "book",
            match source_type {
                1 => "audio",
                2 => "image",
                _ => "video",
            },
        )
        .await
        .unwrap();
    engine.content.set(url).unwrap();
}

#[tokio::test]
async fn cache_flag_downloads_complete_images_and_audio_with_private_headers() {
    for (source_type, path, expected) in [(2, "image", "complete-image"), (1, "audio", "complete-audio")] {
        let origin = origin().await;
        let (_directory, service, engine) = fixture().await;
        set_media(&service, &engine, source_type, format!("{}/{path}", origin.url)).await;
        service.main_text("book", "chapter-0", false).await.unwrap();
        assert_eq!(
            origin.calls.load(Ordering::SeqCst),
            0,
            "阅读只缓存地址，不预先下载完整媒体"
        );
        service.main_text("book", "chapter-0", true).await.unwrap();
        let saved = book(&service).await;
        let address = if source_type == 2 {
            saved["chapters"][0]["pages"][0]["src"].as_str().unwrap()
        } else {
            saved["chapters"][0]["resource"]["resourceId"].as_str().unwrap()
        };
        let reference = ResourceRef::new(address).unwrap();
        assert!(reference.path().starts_with("cache/"));
        assert_eq!(
            std::fs::read(service.root.join(reference.path())).unwrap(),
            expected.as_bytes()
        );
        assert_eq!(origin.calls.load(Ordering::SeqCst), 1);
        assert!(service.indexed_chapter_ready(&saved, 0).await.unwrap());
    }
}

#[tokio::test]
async fn cache_flag_downloads_hls_playlists_and_all_dependencies() {
    let origin = origin().await;
    let (_directory, service, engine) = fixture().await;
    set_media(&service, &engine, 4, format!("{}/master.m3u8", origin.url)).await;
    service.main_text("book", "chapter-0", true).await.unwrap();
    let saved = book(&service).await;
    let reference =
        ResourceRef::new(saved["chapters"][0]["resource"]["resourceId"].as_str().unwrap()).unwrap();
    let path = service.root.join(reference.path());
    let master = std::fs::read_to_string(&path).unwrap();
    let media = master
        .lines()
        .find(|line| !line.starts_with('#') && !line.is_empty())
        .unwrap();
    assert!(!media.contains("://"));
    let playlist = std::fs::read_to_string(path.parent().unwrap().join(media)).unwrap();
    let init = playlist
        .split("URI=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let segment = playlist
        .lines()
        .find(|line| !line.starts_with('#') && !line.is_empty())
        .unwrap();
    assert!(!playlist.contains("://"));
    for (name, expected) in [(init, "complete-init"), (segment, "complete-segment")] {
        assert_eq!(
            std::fs::read(path.parent().unwrap().join(name)).unwrap(),
            expected.as_bytes()
        );
    }
    assert_eq!(origin.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn text_is_always_saved_and_cache_flag_also_saves_embedded_images() {
    let origin = origin().await;
    let (_directory, service, engine) = fixture().await;
    engine
        .content
        .set(format!("<p>正文</p><img src=\"{}/image\">", origin.url))
        .unwrap();
    service.main_text("book", "chapter-0", false).await.unwrap();
    assert_eq!(origin.calls.load(Ordering::SeqCst), 0);
    let text = book(&service).await;
    let reference =
        ResourceRef::new(text["chapters"][0]["resource"]["resourceId"].as_str().unwrap()).unwrap();
    assert!(service.root.join(reference.path()).is_file());
    service.main_text("book", "chapter-0", true).await.unwrap();
    let cached = book(&service).await;
    let reference =
        ResourceRef::new(cached["chapters"][0]["resource"]["resourceId"].as_str().unwrap()).unwrap();
    let document = service.store.read_json_ref(&reference).await.unwrap();
    let markup = document["markup"].as_str().unwrap();
    assert!(!markup.contains("resource://media/"));
    assert!(markup.contains("resource://cache/html/book/"));
    assert_eq!(origin.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelling_or_failing_a_media_download_preserves_the_previous_chapter() {
    for path in ["slow", "failure"] {
        let origin = origin().await;
        let (_directory, service, engine) = fixture().await;
        set_media(&service, &engine, 1, format!("{}/{path}", origin.url)).await;
        service.main_text("book", "chapter-0", false).await.unwrap();
        let before = book(&service).await;
        let result = if path == "slow" {
            let cancel = async {
                tokio::time::timeout(Duration::from_secs(3), origin.entered.notified())
                    .await
                    .unwrap();
                service.cancel_main_text("book", "chapter-0");
            };
            let (result, ()) = tokio::join!(service.main_text("book", "chapter-0", true), cancel);
            assert_eq!(result.as_ref().unwrap_err(), "Chapter content cancelled");
            result
        } else {
            service.main_text("book", "chapter-0", true).await
        };
        assert!(result.is_err());
        assert_eq!(book(&service).await["chapters"], before["chapters"]);
    }
}

async fn fixture() -> (tempfile::TempDir, ApplicationService, Arc<Engine>) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::default());
    let service = ApplicationService::open_with_executor(directory.path(), engine.clone())
        .await
        .unwrap();
    service
        .write_sources(&[SourceRecord {
            id: "source".into(),
            name: "书源".into(),
            group: None,
            enabled: true,
            source: json!({"bookSourceName":"书源","bookSourceUrl":"https://fixture","bookSourceType":0}),
        }])
        .await
        .unwrap();
    service.store.bind_book_type("book", "text").await.unwrap();
    service.store.write_json_ref(&service.store.book_ref("book").unwrap(), &json!({
        "id":"book","title":"书籍","sourceId":"source","chapterCount":2,
        "chapters":[{"id":"chapter-0","index":0,"title":"第一章"},{"id":"chapter-1","index":1,"title":"第二章"}]
    })).await.unwrap();
    service
        .write_internal_json(
            service.book_engine_path("book").unwrap(),
            &json!({
                "sourceId":"source","bookInstanceId":"instance","catalogGeneration":"catalog",
                "book":{"bookUrl":"https://fixture/book"},
                "chapters":[{"index":0,"url":"https://fixture/0"},{"index":1,"url":"https://fixture/1"}]
            }),
        )
        .await
        .unwrap();
    (directory, service, engine)
}

async fn book(service: &ApplicationService) -> Value {
    service
        .store
        .read_json_ref(&service.store.book_ref("book").unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn same_request_inserts_then_refreshes_and_invalid_body_preserves_cache() {
    let (_directory, service, engine) = fixture().await;
    service.main_text("book", "chapter-0", false).await.unwrap();
    let first = book(&service).await;
    engine.revision.store(1, Ordering::SeqCst);
    service.main_text("book", "chapter-0", false).await.unwrap();
    let refreshed = book(&service).await;
    assert_ne!(
        first["chapters"][0]["resource"],
        refreshed["chapters"][0]["resource"]
    );
    engine.revision.store(2, Ordering::SeqCst);
    assert!(service.main_text("book", "chapter-0", false).await.is_err());
    assert_eq!(book(&service).await, refreshed);
}

#[tokio::test]
async fn targeted_cancel_preserves_cache_and_other_chapters_continue() {
    let (_directory, service, engine) = fixture().await;
    service.main_text("book", "chapter-0", false).await.unwrap();
    let cached = book(&service).await["chapters"][0].clone();
    engine.slow.store(true, Ordering::SeqCst);
    let pending = async {
        engine.entered.notified().await;
        // 第一章未返回时，第二章仍能独立完成并入库。
        tokio::time::timeout(
            Duration::from_secs(2),
            service.main_text("book", "chapter-1", false),
        )
        .await
        .unwrap()
        .unwrap();
        service.cancel_main_text("book", "chapter-0");
    };
    let (result, ()) = tokio::join!(service.main_text("book", "chapter-0", false), pending);
    assert_eq!(result.unwrap_err(), "Chapter content cancelled");
    assert!(engine.token.get().unwrap().is_cancelled());
    let saved = book(&service).await;
    assert_eq!(saved["chapters"][0], cached);
    assert!(saved["chapters"][1]["resource"].is_object());
    // 已结束的取消不污染后续同章请求。
    service.cancel_main_text("book", "chapter-0");
    engine.slow.store(false, Ordering::SeqCst);
    engine.revision.store(1, Ordering::SeqCst);
    service.main_text("book", "chapter-0", false).await.unwrap();
    assert_ne!(
        book(&service).await["chapters"][0]["resource"],
        cached["resource"]
    );
}

#[tokio::test]
async fn clearing_cache_cancels_the_previous_inflight_body() {
    let (_directory, service, engine) = fixture().await;
    service.main_text("book", "chapter-0", false).await.unwrap();
    engine.slow.store(true, Ordering::SeqCst);
    let clear = async {
        engine.entered.notified().await;
        tokio::time::timeout(Duration::from_secs(2), service.clear_book_chapter_cache("book"))
            .await
            .unwrap()
            .unwrap();
        engine.release.notify_one();
    };
    let (result, ()) = tokio::join!(service.main_text("book", "chapter-0", false), clear);
    assert_eq!(result.unwrap_err(), "Chapter content cancelled");
    assert!(book(&service).await["chapters"][0]["resource"].is_null());
}
