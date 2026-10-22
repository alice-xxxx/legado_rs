use crate::resources::ResourceRef;
use crate::source_catalog::SourceRecord;
use crate::source_engine::EngineFuture;
use crate::source_engine::SourceEngineRequest;
use crate::source_engine::SourceExecutor;
use crate::test_support::TestApp;
use serde_json::json;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
                return Ok(json!({"url":request.media_url,"headers":{"X-TestApp":"private"}}));
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

#[tokio::test]
async fn proxy_registration_survives_restart_and_archive_restore() {
    let origin = origin().await;
    let (directory, service, engine) = fixture().await;
    set_media(&service, &engine, 1, format!("{}/audio", origin.url)).await;
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    let saved = book(&service).await;
    let reference = ResourceRef::new(saved["chapters"][0]["url"].as_str().unwrap()).unwrap();
    let archive = tempfile::tempdir().unwrap();
    let backup = archive.path().join("resources.zip");
    service
        .backup
        .create_backup(&service.database, &service.resources, &backup)
        .await
        .unwrap();
    drop(service);
    let reopened = TestApp::open_with_executor(directory.path(), engine)
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert!(client
        .get(reopened.resources.url_for(&reference))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    reopened.resources.proxy.clear();
    assert_eq!(
        client
            .get(reopened.resources.url_for(&reference))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    reopened
        .backup
        .restore_backup(
            &reopened.search,
            &reopened.information,
            &reopened.text,
            &reopened.tts,
            &reopened.login,
            &reopened.database,
            &reopened.resources,
            &backup,
        )
        .await
        .unwrap();
    assert!(client
        .get(reopened.resources.url_for(&reference))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
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
            if request.headers().get("X-TestApp").and_then(|value|value.to_str().ok()) != Some("private") {
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

async fn set_media(service: &TestApp, engine: &Engine, source_type: u32, url: String) {
    let mut source = service.sources.get("source").await.unwrap();
    source.source["bookSourceType"] = json!(source_type);
    service.sources.replace(&[source]).await.unwrap();
    service
        .database
        .bind_book_type(
            "book".to_owned(),
            match source_type {
                1 => "audio",
                2 => "image",
                _ => "video",
            }
            .to_owned(),
        )
        .unwrap()
        .await
        .unwrap();
    engine.content.set(url).unwrap();
}

#[tokio::test]
async fn cache_flag_downloads_complete_images_and_audio_with_private_headers() {
    for (source_type, path, expected) in [
        (2, "image", "complete-image"),
        (1, "audio", "complete-audio"),
    ] {
        let origin = origin().await;
        let (_directory, service, engine) = fixture().await;
        set_media(
            &service,
            &engine,
            source_type,
            format!("{}/{path}", origin.url),
        )
        .await;
        service
            .text
            .main_text(
                &service.database,
                &service.resources,
                "book",
                "chapter-0",
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            origin.calls.load(Ordering::SeqCst),
            0,
            "阅读只缓存地址，不预先下载完整媒体"
        );
        service
            .text
            .main_text(
                &service.database,
                &service.resources,
                "book",
                "chapter-0",
                true,
            )
            .await
            .unwrap();
        let saved = book(&service).await;
        let address = if source_type == 2 {
            saved["chapters"][0]["pages"][0]["url"].as_str().unwrap()
        } else {
            saved["chapters"][0]["url"].as_str().unwrap()
        };
        let reference = ResourceRef::new(address).unwrap();
        assert!(reference.path().starts_with("cache/"));
        assert_eq!(
            std::fs::read(service.resources.root().join(reference.path())).unwrap(),
            expected.as_bytes()
        );
        assert_eq!(origin.calls.load(Ordering::SeqCst), 1);
        assert!(
            crate::book_files::indexed_chapter_ready(&service.resources, &saved, 0)
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn cache_flag_downloads_hls_playlists_and_all_dependencies() {
    let origin = origin().await;
    let (_directory, service, engine) = fixture().await;
    set_media(&service, &engine, 4, format!("{}/master.m3u8", origin.url)).await;
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            true,
        )
        .await
        .unwrap();
    let saved = book(&service).await;
    let reference = ResourceRef::new(saved["chapters"][0]["url"].as_str().unwrap()).unwrap();
    let path = service.resources.root().join(reference.path());
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
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    assert_eq!(origin.calls.load(Ordering::SeqCst), 0);
    let text = book(&service).await;
    let reference = ResourceRef::new(text["chapters"][0]["url"].as_str().unwrap()).unwrap();
    assert!(service.resources.root().join(reference.path()).is_file());
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            true,
        )
        .await
        .unwrap();
    let cached = book(&service).await;
    let reference = ResourceRef::new(cached["chapters"][0]["url"].as_str().unwrap()).unwrap();
    let document = service
        .resources
        .cache
        .read_content(&reference)
        .await
        .unwrap();
    let markup = document["markup"].as_str().unwrap();
    assert!(!markup.contains("resource://proxy/"));
    assert!(markup.contains("resource://cache/html/book/"));
    assert_eq!(origin.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelling_or_failing_a_media_download_preserves_the_previous_chapter() {
    for path in ["slow", "failure"] {
        let origin = origin().await;
        let (_directory, service, engine) = fixture().await;
        set_media(&service, &engine, 1, format!("{}/{path}", origin.url)).await;
        service
            .text
            .main_text(
                &service.database,
                &service.resources,
                "book",
                "chapter-0",
                false,
            )
            .await
            .unwrap();
        let before = book(&service).await;
        let result = if path == "slow" {
            let cancel = async {
                tokio::time::timeout(Duration::from_secs(3), origin.entered.notified())
                    .await
                    .unwrap();
                service.text.cancel_main_text("book", "chapter-0");
            };
            let (result, ()) = tokio::join!(
                service.text.main_text(
                    &service.database,
                    &service.resources,
                    "book",
                    "chapter-0",
                    true
                ),
                cancel
            );
            assert_eq!(result.as_ref().unwrap_err(), "Chapter content cancelled");
            result
        } else {
            service
                .text
                .main_text(
                    &service.database,
                    &service.resources,
                    "book",
                    "chapter-0",
                    true,
                )
                .await
        };
        assert!(result.is_err());
        assert_eq!(book(&service).await["chapters"], before["chapters"]);
    }
}

async fn fixture() -> (tempfile::TempDir, TestApp, Arc<Engine>) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::default());
    let service = TestApp::open_with_executor(directory.path(), engine.clone())
        .await
        .unwrap();
    service
        .sources.replace(&[SourceRecord {
            id: "source".into(),
            name: "书源".into(),
            group: None,
            enabled: true,
            source: json!({"bookSourceName":"书源","bookSourceUrl":"https://fixture","bookSourceType":0}),
        }])
        .await
        .unwrap();
    service
        .database
        .bind_book_type("book".to_owned(), "text".to_owned())
        .unwrap()
        .await
        .unwrap();
    service
        .database
        .import_book(
            serde_json::from_value(json!({"contentType":"text",
                "id":"book","title":"书籍","sourceId":"source",
                "engine":{"bookUrl":"https://fixture/book"},
                "chapters":[
                    {"id":"chapter-0","title":"第一章","engineData":{"url":"https://fixture/0"}},
                    {"id":"chapter-1","title":"第二章","engineData":{"url":"https://fixture/1"}}
                ]
            }))
            .unwrap(),
        )
        .unwrap()
        .await
        .unwrap();
    (directory, service, engine)
}

async fn book(service: &TestApp) -> Value {
    crate::book_files::read_book(&service.database, "book")
        .await
        .unwrap()
}

#[tokio::test]
async fn same_request_inserts_then_refreshes_and_invalid_body_preserves_cache() {
    let (_directory, service, engine) = fixture().await;
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    let first = book(&service).await;
    engine.revision.store(1, Ordering::SeqCst);
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    let refreshed = book(&service).await;
    assert_ne!(first["chapters"][0]["url"], refreshed["chapters"][0]["url"]);
    engine.revision.store(2, Ordering::SeqCst);
    assert!(service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false
        )
        .await
        .is_err());
    assert_eq!(book(&service).await, refreshed);
}

#[tokio::test]
async fn targeted_cancel_preserves_cache_and_other_chapters_continue() {
    let (_directory, service, engine) = fixture().await;
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    let cached = book(&service).await["chapters"][0].clone();
    engine.slow.store(true, Ordering::SeqCst);
    let pending = async {
        engine.entered.notified().await;
        // 第一章未返回时，第二章仍能独立完成并入库。
        tokio::time::timeout(
            Duration::from_secs(2),
            service.text.main_text(
                &service.database,
                &service.resources,
                "book",
                "chapter-1",
                false,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        service.text.cancel_main_text("book", "chapter-0");
    };
    let (result, ()) = tokio::join!(
        service.text.main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false
        ),
        pending
    );
    assert_eq!(result.unwrap_err(), "Chapter content cancelled");
    assert!(engine.token.get().unwrap().is_cancelled());
    let saved = book(&service).await;
    assert_eq!(saved["chapters"][0], cached);
    assert!(saved["chapters"][1]["url"].is_string());
    // 已结束的取消不污染后续同章请求。
    service.text.cancel_main_text("book", "chapter-0");
    engine.slow.store(false, Ordering::SeqCst);
    engine.revision.store(1, Ordering::SeqCst);
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    assert_ne!(book(&service).await["chapters"][0]["url"], cached["url"]);
}

#[tokio::test]
async fn clearing_cache_cancels_the_previous_inflight_body() {
    let (_directory, service, engine) = fixture().await;
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    engine.slow.store(true, Ordering::SeqCst);
    let clear = async {
        engine.entered.notified().await;
        service.text.cancel_main_text("book", "chapter-0");
        tokio::time::timeout(
            Duration::from_secs(2),
            service.resources.clear_cache("book"),
        )
        .await
        .unwrap()
        .unwrap();
        engine.release.notify_one();
    };
    let (result, ()) = tokio::join!(
        service.text.main_text(
            &service.database,
            &service.resources,
            "book",
            "chapter-0",
            false
        ),
        clear
    );
    assert_eq!(result.unwrap_err(), "Chapter content cancelled");
    assert!(book(&service).await["chapters"][0]["url"].is_null());
}
