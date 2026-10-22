use crate::source_catalog::SourceRecord;
use crate::source_engine::EngineFuture;
use crate::source_engine::SourceEngineRequest;
use crate::source_engine::SourceExecutor;
use crate::test_support::TestApp;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
struct Engine {
    details: AtomicUsize,
    catalog: AtomicUsize,
    calls: std::sync::Mutex<Vec<String>>,
    entered: tokio::sync::Notify,
    slow_token: std::sync::OnceLock<tokio_util::sync::CancellationToken>,
}
impl SourceExecutor for Engine {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(request.operation.clone());
            match request.operation.as_str() {
                "bookInfo" => {
                    if request.source["bookSourceName"] == "slow" {
                        let _ = self.slow_token.set(request.cancellation.unwrap());
                        self.entered.notify_one();
                        return std::future::pending().await;
                    }
                    let mut book = request.book.unwrap();
                    book["name"] = json!(format!(
                        "详情{}",
                        self.details.fetch_add(1, Ordering::SeqCst) + 1
                    ));
                    book["author"] = json!("作者");
                    Ok(book)
                }
                "chapters" => match self.catalog.load(Ordering::SeqCst) {
                    1 => Ok(json!([
                        {"title":"序章","url":"https://fixture/preface"},
                        {"title":"第一章","url":"https://fixture/chapter"},
                        {"title":"第二章","url":"https://fixture/chapter2"}
                    ])),
                    2 => Ok(json!([])),
                    3 => Ok(json!([
                        {"title":"第一章","url":"https://fixture/chapter"},
                        {"title":"重复章节","url":"https://fixture/chapter"}
                    ])),
                    4 => {
                        let _ = self.slow_token.set(request.cancellation.unwrap());
                        self.entered.notify_one();
                        std::future::pending().await
                    }
                    _ => Ok(json!([{"title":"第一章","url":"https://fixture/chapter"}])),
                },
                "content" => Ok(json!("已缓存的正文")),
                _ => Err("Unexpected operation".into()),
            }
        })
    }
}
async fn fixture() -> (tempfile::TempDir, TestApp, Arc<Engine>) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::default());
    let service = TestApp::open_with_executor(directory.path(), engine.clone())
        .await
        .unwrap();
    let records = ["fast","slow"].map(|id|SourceRecord { id:id.into(), name:id.into(), group:None, enabled:true,
        source:json!({"bookSourceName":id,"bookSourceUrl":format!("https://{id}"),"bookSourceType":0}) });
    service.sources.replace(&records).await.unwrap();
    (directory, service, engine)
}

#[tokio::test]
async fn same_details_interface_inserts_then_refreshes_without_resetting_catalog_or_progress() {
    let (_directory, service, engine) = fixture().await;
    let result: () = service
        .information
        .more_information(&service.database, "fast", "https://fixture/book")
        .await
        .unwrap();
    assert_eq!(result, ());
    let id = crate::database::online_book_id("fast", "text", "https://fixture/book");
    service
        .text
        .save_progress(
            &service.database,
            &id,
            json!({"chapterIndex":0,"offset":37,"updatedAtMs":100}),
        )
        .await
        .unwrap();
    let cached = crate::book_files::read_book(&service.database, &id)
        .await
        .unwrap();
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            &id,
            cached["chapters"][0]["id"].as_str().unwrap(),
            false,
        )
        .await
        .unwrap();
    let before = crate::book_files::read_book(&service.database, &id)
        .await
        .unwrap();
    // 模拟同一本书另一条首次打开请求晚入库，不能覆盖已产生的缓存和进度。
    service
        .database
        .save_information(
            crate::database::Book {
                chapters: Vec::new(),
                ..serde_json::from_value(before.clone()).unwrap()
            },
            before["chapters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|chapter| serde_json::from_value(chapter["engineData"].clone()).unwrap())
                .collect(),
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(
        before,
        crate::book_files::read_book(&service.database, &id)
            .await
            .unwrap()
    );
    service
        .information
        .more_information(&service.database, "fast", "https://fixture/book")
        .await
        .unwrap();
    let after = crate::book_files::read_book(&service.database, &id)
        .await
        .unwrap();
    assert_eq!(before["title"], "详情1");
    assert_eq!(after["title"], "详情2");
    assert_eq!(before["chapters"], after["chapters"]);
    assert_eq!(before["progress"], after["progress"]);
    assert_eq!(before["engine"]["bookUrl"], after["engine"]["bookUrl"]);
    assert_eq!(
        *engine.calls.lock().unwrap(),
        ["bookInfo", "chapters", "content", "bookInfo", "chapters"]
    );
}

#[tokio::test]
async fn unified_refresh_updates_catalog_without_tasks_and_preserves_cached_reading_position() {
    let (_directory, service, engine) = fixture().await;
    service
        .information
        .more_information(&service.database, "fast", "https://fixture/book")
        .await
        .unwrap();
    let id = crate::database::online_book_id("fast", "text", "https://fixture/book");
    service
        .text
        .save_progress(
            &service.database,
            &id,
            json!({"chapterIndex":0,"offset":37,"updatedAtMs":100}),
        )
        .await
        .unwrap();
    let cached = crate::book_files::read_book(&service.database, &id)
        .await
        .unwrap();
    service
        .text
        .main_text(
            &service.database,
            &service.resources,
            &id,
            cached["chapters"][0]["id"].as_str().unwrap(),
            false,
        )
        .await
        .unwrap();
    let before = crate::book_files::read_book(&service.database, &id)
        .await
        .unwrap();
    engine.catalog.store(1, Ordering::SeqCst);
    service
        .information
        .more_information(&service.database, "fast", "https://fixture/book")
        .await
        .unwrap();
    let after = crate::book_files::read_book(&service.database, &id)
        .await
        .unwrap();
    assert_eq!(after["title"], "详情2");
    assert_eq!(after["chapters"].as_array().unwrap().len(), 3);
    assert_eq!(after["chapters"][1]["id"], before["chapters"][0]["id"]);
    assert_eq!(after["chapters"][1]["url"], before["chapters"][0]["url"]);
    assert_eq!(after["progress"]["chapterIndex"], 1);
    assert_eq!(after["progress"]["offset"], 37);
    // 空目录和重复 URL 都不能部分提交详情或清除原有目录。
    for invalid in [2, 3] {
        engine.catalog.store(invalid, Ordering::SeqCst);
        assert!(service
            .information
            .more_information(&service.database, "fast", "https://fixture/book")
            .await
            .is_err());
        assert_eq!(
            after,
            crate::book_files::read_book(&service.database, &id)
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn cancelling_catalog_request_does_not_commit_partial_details() {
    let (_directory, service, engine) = fixture().await;
    service
        .information
        .more_information(&service.database, "fast", "https://fixture/book")
        .await
        .unwrap();
    let id = crate::database::online_book_id("fast", "text", "https://fixture/book");
    let before = crate::book_files::read_book(&service.database, &id)
        .await
        .unwrap();
    engine.catalog.store(4, Ordering::SeqCst);
    let worker = service.clone();
    let pending = tokio::spawn(async move {
        worker
            .information
            .more_information(&worker.database, "fast", "https://fixture/book")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    service
        .information
        .cancel_information("fast", "https://fixture/book");
    assert!(tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(engine.slow_token.get().unwrap().is_cancelled());
    assert_eq!(
        before,
        crate::book_files::read_book(&service.database, &id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn independent_details_finish_while_another_request_is_pending() {
    let (_directory, service, engine) = fixture().await;
    let slow_service = service.clone();
    let slow = tokio::spawn(async move {
        slow_service
            .information
            .more_information(&slow_service.database, "slow", "https://fixture/slow")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    // 慢请求仍在执行时，另一部书的详情可以完成。
    tokio::time::timeout(
        Duration::from_secs(2),
        service
            .information
            .more_information(&service.database, "fast", "https://fixture/fast"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!slow.is_finished());
    service
        .information
        .cancel_information("slow", "https://fixture/slow");
    assert!(tokio::time::timeout(Duration::from_secs(2), slow)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .contains("cancelled"));
    assert!(engine.slow_token.get().unwrap().is_cancelled());
    assert!(service
        .database
        .book_type(&crate::database::online_book_id(
            "slow",
            "text",
            "https://fixture/slow"
        ))
        .unwrap()
        .is_none());
    assert!(service
        .database
        .book_type(&crate::database::online_book_id(
            "fast",
            "text",
            "https://fixture/fast"
        ))
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn cancelling_one_url_does_not_cancel_another_on_the_same_source() {
    let (_directory, service, engine) = fixture().await;
    let first_service = service.clone();
    let first = tokio::spawn(async move {
        first_service
            .information
            .more_information(&first_service.database, "slow", "https://fixture/first")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    let second_service = service.clone();
    let second = tokio::spawn(async move {
        second_service
            .information
            .more_information(&second_service.database, "slow", "https://fixture/second")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    service
        .information
        .cancel_information("slow", "https://fixture/first");
    assert!(tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .contains("cancelled"));
    assert!(!second.is_finished());
    service
        .information
        .cancel_information("slow", "https://fixture/second");
    assert!(tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .contains("cancelled"));
}

#[tokio::test]
async fn cancelling_when_idle_does_not_cancel_the_next_details_request() {
    let (_directory, service, _engine) = fixture().await;
    service
        .information
        .cancel_information("fast", "https://fixture/book");
    service
        .information
        .more_information(&service.database, "fast", "https://fixture/book")
        .await
        .unwrap();
    assert!(service
        .database
        .book_type(&crate::database::online_book_id(
            "fast",
            "text",
            "https://fixture/book"
        ))
        .unwrap()
        .is_some());
}
