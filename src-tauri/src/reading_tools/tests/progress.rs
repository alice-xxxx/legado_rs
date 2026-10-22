use crate::resources::ResourceRef;
use crate::source_catalog::SourceRecord;
use crate::source_engine::EngineFuture;
use crate::source_engine::SourceEngineRequest;
use crate::source_engine::SourceExecutor;
use crate::test_support::TestApp;
use serde_json::json;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct CountingExecutor(Arc<AtomicUsize>, &'static str);
impl SourceExecutor for CountingExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            if request.operation == "resolveMedia" {
                Ok(json!({"url":request.media_url, "headers":{}}))
            } else {
                Ok(json!(self.1))
            }
        })
    }
}

async fn fixture() -> (tempfile::TempDir, TestApp, Arc<AtomicUsize>) {
    fixture_with_content(0, "　　Cached text\n　　Next paragraph").await
}

async fn fixture_with_content(
    source_type: u32,
    content: &'static str,
) -> (tempfile::TempDir, TestApp, Arc<AtomicUsize>) {
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let service = TestApp::open_with_executor(
        directory.path(),
        Arc::new(CountingExecutor(calls.clone(), content)),
    )
    .await
    .unwrap();
    service.sources.replace(&[SourceRecord {
            id: "source-a".into(), name: "Source".into(), group: None, enabled: true,
            source: json!({"bookSourceName":"Source","bookSourceUrl":"https://example.invalid","bookSourceType":source_type}),
        }]).await.unwrap();
    service
        .database
        .bind_book_type(
            "book-a".to_owned(),
            match source_type {
                1 => "audio",
                2 => "image",
                4 => "video",
                _ => "text",
            }
            .to_owned(),
        )
        .unwrap()
        .await
        .unwrap();
    // Media reuse cases exercise one chapter; the supported preload minimum is one.
    let chapter_count = if source_type == 0 { 5 } else { 1 };
    let chapters: Vec<Value> = (0..chapter_count)
        .map(|index| {
            json!({
                "id":format!("chapter-{index}"),"title":format!("Chapter {index}"),
                "engineData":{"url":format!("https://example.invalid/{index}")}
            })
        })
        .collect();
    let kind = crate::source_catalog::content_type(&json!({"bookSourceType":source_type}));
    service
        .database
        .import_book(
            serde_json::from_value(json!({"contentType":kind,
                "id":"book-a","title":"Book","sourceId":"source-a","chapters":chapters,
                "engine":{"bookUrl":"https://example.invalid/book","name":"Book"}
            }))
            .unwrap(),
        )
        .unwrap()
        .await
        .unwrap();
    service
        .resources
        .settings
        .save(&json!({"reader":{"preloadCount":2}}))
        .await
        .unwrap();
    (directory, service, calls)
}

#[tokio::test]
async fn reading_saves_position_and_prepares_current_plus_configured_chapters() {
    let (_directory, service, calls) = fixture().await;
    service
        .text
        .save_progress(
            &service.database,
            "book-a",
            json!({"chapterId":"chapter-0","chapterIndex":0,"offset":3,"updatedAtMs":1}),
        )
        .await
        .unwrap();
    let published = std::sync::Mutex::new(Vec::new());
    let epoch = service.backup.restore_epoch.load(Ordering::Acquire);
    service
        .text
        .prepare_reading_chapters(
            &service.database,
            &service.resources,
            "book-a",
            0,
            epoch,
            |resource| published.lock().unwrap().push(resource),
        )
        .await
        .unwrap();
    let book = crate::book_files::read_book(&service.database, "book-a")
        .await
        .unwrap();
    assert_eq!(book["progress"]["offset"], 3);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(published.lock().unwrap().len(), 3);
    assert!(book["chapters"][2]["url"].is_string());
    assert!(book["chapters"][3]["url"].is_null());
    service
        .text
        .save_progress(
            &service.database,
            "book-a",
            json!({"chapterId":"chapter-0","chapterIndex":0,"offset":4,"updatedAtMs":2}),
        )
        .await
        .unwrap();
    published.lock().unwrap().clear();
    service
        .text
        .prepare_reading_chapters(
            &service.database,
            &service.resources,
            "book-a",
            0,
            epoch,
            |resource| published.lock().unwrap().push(resource),
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(published.lock().unwrap().is_empty());
    service
        .text
        .save_progress(
            &service.database,
            "book-a",
            json!({"chapterId":"chapter-0","chapterIndex":0,"offset":1,"updatedAtMs":1}),
        )
        .await
        .unwrap();
    let book = crate::book_files::read_book(&service.database, "book-a")
        .await
        .unwrap();
    assert_eq!(
        book["progress"]["offset"], 4,
        "A late notification must not roll progress back"
    );
}

#[tokio::test]
async fn stale_reading_window_does_not_prepare_old_chapters() {
    let (_directory, service, calls) = fixture().await;
    service
        .text
        .save_progress(
            &service.database,
            "book-a",
            json!({"chapterId":"chapter-3","chapterIndex":3,"offset":0}),
        )
        .await
        .unwrap();
    let epoch = service.backup.restore_epoch.load(Ordering::Acquire);
    service
        .text
        .prepare_reading_chapters(
            &service.database,
            &service.resources,
            "book-a",
            0,
            epoch,
            |_| panic!("Stale window must not publish"),
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

const COMIC: &str = "<img src=\"https://images.example.invalid/1.webp\"><img src=\"https://images.example.invalid/2.webp\"><script>alert(1)</script>";

#[tokio::test]
async fn comic_img_list_is_indexed_as_pages_without_a_chapter_json() {
    let (_directory, service, calls) = fixture_with_content(2, COMIC).await;
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            "book-a",
            "chapter-0",
            false,
        )
        .await
        .unwrap();
    let book = crate::book_files::read_book(&service.database, "book-a")
        .await
        .unwrap();
    assert_eq!(book["contentType"], "image");
    assert_eq!(book["chapters"][0]["pages"].as_array().unwrap().len(), 2);
    assert!(book["chapters"][0]["url"].is_null());
    for page in book["chapters"][0]["pages"].as_array().unwrap() {
        assert!(page.get("headers").is_none());
        assert!(page.get("engine").is_none());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn old_comic_text_cache_is_replaced_by_the_unified_content_request() {
    let (_directory, service, calls) = fixture_with_content(2, COMIC).await;
    let original =
        crate::book_files::text(&service.resources, "image", "book-a", "chapter-0", COMIC)
            .await
            .unwrap();
    let descriptor = serde_json::to_value(
        crate::book_files::ChapterResourceDescriptor::for_chapter(&original, "text"),
    )
    .unwrap();
    service
        .database
        .save_main_text(
            "book-a".into(),
            "chapter-0".into(),
            crate::database::Chapter {
                title: "Chapter".into(),
                resource: Some(serde_json::from_value(descriptor).unwrap()),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap()
        .wait()
        .unwrap();
    service
        .resources
        .settings
        .save(&json!({"reader":{"preloadCount":1}}))
        .await
        .unwrap();
    service
        .text
        .save_progress(
            &service.database,
            "book-a",
            json!({"chapterIndex":0,"offset":0}),
        )
        .await
        .unwrap();
    let published = std::sync::Mutex::new(Vec::new());
    let epoch = service.backup.restore_epoch.load(Ordering::Acquire);
    service
        .text
        .prepare_reading_chapters(
            &service.database,
            &service.resources,
            "book-a",
            0,
            epoch,
            |resource| published.lock().unwrap().push(resource),
        )
        .await
        .unwrap();
    let book = crate::book_files::read_book(&service.database, "book-a")
        .await
        .unwrap();
    assert_eq!(book["contentType"], "image");
    assert!(book["chapters"][0]["url"].is_null());
    assert_eq!(book["chapters"][0]["pages"].as_array().unwrap().len(), 2);
    assert_eq!(
        service
            .resources
            .cache
            .read_content(&original)
            .await
            .unwrap()["kind"],
        "text"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "One content request and two image resolutions replace the old cache"
    );
    assert_eq!(published.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn comic_url_lists_become_private_image_resources() {
    for content in [
        "https://images.example.invalid/1.webp?a=1&b=2\n\n https://images.example.invalid/2.webp ",
        "[\"https://images.example.invalid/1.webp?a=1&b=2\",\"https://images.example.invalid/2.webp\"]",
        "<img src=\"https://images.example.invalid/1.webp?a=1&amp;b=2\"><img src=\"https://images.example.invalid/2.webp\">",
    ] {
        let (_directory, service, calls) = fixture_with_content(2, content).await;
        service.text.main_text(&service.database, &service.resources, "book-a", "chapter-0", false)
            .await
            .unwrap();
        let book = crate::book_files::read_book(&service.database, "book-a")
            .await
            .unwrap();
        assert_eq!(book["contentType"], "image");
        let pages = book["chapters"][0]["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 2);
        assert!(pages.iter().all(|page| page["url"]
            .as_str()
            .unwrap()
            .starts_with("resource://proxy/")));
        // 原始地址保留 &，只有 HTML 属性需要实体解码。
        let mut urls = service.database.media().unwrap().into_iter()
            .map(|media|media.url)
            .collect::<Vec<_>>();
        urls.sort();
        assert_eq!(
            urls,
            [
                "https://images.example.invalid/1.webp?a=1&b=2",
                "https://images.example.invalid/2.webp",
            ]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}

#[tokio::test]
async fn changed_source_type_does_not_reuse_an_audio_cache_for_a_comic() {
    let (_directory, service, calls) =
        fixture_with_content(2, "https://images.example.invalid/1.webp").await;
    let original = ResourceRef::new("resource://proxy/obsolete-audio").unwrap();
    let descriptor = serde_json::to_value(
        crate::book_files::ChapterResourceDescriptor::for_chapter(&original, "audio"),
    )
    .unwrap();
    service
        .database
        .save_main_text(
            "book-a".into(),
            "chapter-0".into(),
            crate::database::Chapter {
                title: "Chapter".into(),
                resource: Some(serde_json::from_value(descriptor).unwrap()),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap()
        .wait()
        .unwrap();
    service
        .resources
        .settings
        .save(&json!({"reader":{"preloadCount":1}}))
        .await
        .unwrap();
    service
        .text
        .save_progress(
            &service.database,
            "book-a",
            json!({"chapterIndex":0,"offset":0}),
        )
        .await
        .unwrap();
    let epoch = service.backup.restore_epoch.load(Ordering::Acquire);
    service
        .text
        .prepare_reading_chapters(
            &service.database,
            &service.resources,
            "book-a",
            0,
            epoch,
            |_| {},
        )
        .await
        .unwrap();
    let book = crate::book_files::read_book(&service.database, "book-a")
        .await
        .unwrap();
    assert_eq!(book["contentType"], "image");
    assert!(book["mediaType"].is_null());
    assert!(book["chapters"][0]["url"].is_null());
    assert!(book["chapters"][0]["pages"]
        .as_array()
        .is_some_and(|pages| !pages.is_empty()));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn audio_and_video_are_resolved_to_player_resources_and_reused() {
    for (source_type, expected) in [(1, "audio"), (4, "video")] {
        let (_directory, service, calls) =
            fixture_with_content(source_type, "https://media.example.invalid/play.mp4").await;
        service
            .text
            .main_text(
                &service.database,
                &service.resources,
                "book-a",
                "chapter-0",
                false,
            )
            .await
            .unwrap();
        let book = crate::book_files::read_book(&service.database, "book-a")
            .await
            .unwrap();
        assert_eq!(book["contentType"], expected);
        assert!(book["chapters"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("resource://proxy/"));
        assert!(book["chapters"][0]["sources"].is_array());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        service
            .resources
            .settings
            .save(&json!({"reader":{"preloadCount":1}}))
            .await
            .unwrap();
        service
            .text
            .save_progress(
                &service.database,
                "book-a",
                json!({"chapterIndex":0,"offset":0}),
            )
            .await
            .unwrap();
        let epoch = service.backup.restore_epoch.load(Ordering::Acquire);
        service
            .text
            .prepare_reading_chapters(
                &service.database,
                &service.resources,
                "book-a",
                0,
                epoch,
                |_| panic!("Ready media must be reused"),
            )
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
