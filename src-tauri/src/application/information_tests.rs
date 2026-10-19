use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

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
async fn fixture() -> (tempfile::TempDir, ApplicationService, Arc<Engine>) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::default());
    let service = ApplicationService::open_with_executor(directory.path(), engine.clone())
        .await
        .unwrap();
    let records = ["fast","slow"].map(|id|SourceRecord { id:id.into(), name:id.into(), group:None, enabled:true,
        source:json!({"bookSourceName":id,"bookSourceUrl":format!("https://{id}"),"bookSourceType":0}) });
    service.write_sources(&records).await.unwrap();
    (directory, service, engine)
}

#[tokio::test]
async fn same_details_interface_inserts_then_refreshes_without_resetting_catalog_or_progress() {
    let (_directory, service, engine) = fixture().await;
    let result: () = service
        .more_information("fast", "https://fixture/book")
        .await
        .unwrap();
    assert_eq!(result, ());
    let id = service
        .store
        .database()
        .information_id("fast", "https://fixture/book", "text")
        .unwrap()
        .unwrap();
    service
        .save_progress(&id, json!({"chapterIndex":0,"offset":37,"updatedAtMs":100}))
        .await
        .unwrap();
    let cached = service.store.read_json_ref(&service.store.book_ref(&id).unwrap()).await.unwrap();
    service.main_text(&id, cached["chapters"][0]["id"].as_str().unwrap(), false).await.unwrap();
    let reference = service.store.book_ref(&id).unwrap();
    let before = service.store.read_json_ref(&reference).await.unwrap();
    let private = service
        .read_internal_json(service.book_engine_path(&id).unwrap())
        .await
        .unwrap();
    // 模拟同一本书另一条首次打开请求晚入库，不能覆盖已产生的缓存和进度。
    service
        .save_information(
            &id,
            &service.find_source("fast").await.unwrap(),
            private["book"].clone(),
            private["chapters"].as_array().unwrap().clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        before,
        service.store.read_json_ref(&reference).await.unwrap()
    );
    service
        .more_information("fast", "https://fixture/book")
        .await
        .unwrap();
    let after = service.store.read_json_ref(&reference).await.unwrap();
    let next_private = service
        .read_internal_json(service.book_engine_path(&id).unwrap())
        .await
        .unwrap();
    assert_eq!(before["title"], "详情1");
    assert_eq!(after["title"], "详情2");
    assert_eq!(before["chapters"], after["chapters"]);
    assert_eq!(before["progress"], after["progress"]);
    assert_eq!(
        private["catalogGeneration"],
        next_private["catalogGeneration"]
    );
    assert_eq!(private["bookInstanceId"], next_private["bookInstanceId"]);
    assert_eq!(
        *engine.calls.lock().unwrap(),
        ["bookInfo", "chapters", "content", "bookInfo", "chapters"]
    );
}

#[tokio::test]
async fn unified_refresh_updates_catalog_without_tasks_and_preserves_cached_reading_position() {
    let (_directory, service, engine) = fixture().await;
    service
        .more_information("fast", "https://fixture/book")
        .await
        .unwrap();
    let id = service
        .store
        .database()
        .information_id("fast", "https://fixture/book", "text")
        .unwrap()
        .unwrap();
    service
        .save_progress(&id, json!({"chapterIndex":0,"offset":37,"updatedAtMs":100}))
        .await
        .unwrap();
    let cached = service.store.read_json_ref(&service.store.book_ref(&id).unwrap()).await.unwrap();
    service.main_text(&id, cached["chapters"][0]["id"].as_str().unwrap(), false).await.unwrap();
    let reference = service.store.book_ref(&id).unwrap();
    let before = service.store.read_json_ref(&reference).await.unwrap();
    engine.catalog.store(1, Ordering::SeqCst);
    service
        .more_information("fast", "https://fixture/book")
        .await
        .unwrap();
    let after = service.store.read_json_ref(&reference).await.unwrap();
    assert_eq!(after["title"], "详情2");
    assert_eq!(after["chapters"].as_array().unwrap().len(), 3);
    assert_eq!(after["chapters"][1]["id"], before["chapters"][0]["id"]);
    assert_eq!(
        after["chapters"][1]["resource"],
        before["chapters"][0]["resource"]
    );
    assert_eq!(after["progress"]["chapterIndex"], 1);
    assert_eq!(after["progress"]["offset"], 37);
    assert!(service
        .store
        .database()
        .read("tasks.json")
        .unwrap()
        .unwrap()["tasks"]
        .as_array()
        .unwrap()
        .is_empty());
    // 空目录和重复 URL 都不能部分提交详情或清除原有目录。
    for invalid in [2, 3] {
        engine.catalog.store(invalid, Ordering::SeqCst);
        assert!(service
            .more_information("fast", "https://fixture/book")
            .await
            .is_err());
        assert_eq!(
            after,
            service.store.read_json_ref(&reference).await.unwrap()
        );
    }
}

#[tokio::test]
async fn cancelling_catalog_request_does_not_commit_partial_details() {
    let (_directory, service, engine) = fixture().await;
    service
        .more_information("fast", "https://fixture/book")
        .await
        .unwrap();
    let id = service
        .store
        .database()
        .information_id("fast", "https://fixture/book", "text")
        .unwrap()
        .unwrap();
    let reference = service.store.book_ref(&id).unwrap();
    let before = service.store.read_json_ref(&reference).await.unwrap();
    engine.catalog.store(4, Ordering::SeqCst);
    let worker = service.clone();
    let pending = tokio::spawn(async move {
        worker
            .more_information("fast", "https://fixture/book")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    service.cancel_information("fast", "https://fixture/book");
    assert!(tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(engine.slow_token.get().unwrap().is_cancelled());
    assert_eq!(
        before,
        service.store.read_json_ref(&reference).await.unwrap()
    );
}

#[tokio::test]
async fn independent_details_finish_while_another_request_is_pending() {
    let (_directory, service, engine) = fixture().await;
    let slow_service = service.clone();
    let slow = tokio::spawn(async move {
        slow_service
            .more_information("slow", "https://fixture/slow")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    // 慢请求仍在执行时，另一部书的详情可以完成。
    tokio::time::timeout(
        Duration::from_secs(2),
        service.more_information("fast", "https://fixture/fast"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!slow.is_finished());
    service.cancel_information("slow", "https://fixture/slow");
    assert!(tokio::time::timeout(Duration::from_secs(2), slow)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .contains("cancelled"));
    assert!(engine.slow_token.get().unwrap().is_cancelled());
    assert!(service
        .store
        .database()
        .information_id("slow", "https://fixture/slow", "text")
        .unwrap()
        .is_none());
    assert!(service
        .store
        .database()
        .information_id("fast", "https://fixture/fast", "text")
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn cancelling_one_url_does_not_cancel_another_on_the_same_source() {
    let (_directory, service, engine) = fixture().await;
    let first_service = service.clone();
    let first = tokio::spawn(async move {
        first_service
            .more_information("slow", "https://fixture/first")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    let second_service = service.clone();
    let second = tokio::spawn(async move {
        second_service
            .more_information("slow", "https://fixture/second")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), engine.entered.notified())
        .await
        .unwrap();
    service.cancel_information("slow", "https://fixture/first");
    assert!(tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .contains("cancelled"));
    assert!(!second.is_finished());
    service.cancel_information("slow", "https://fixture/second");
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
    service.cancel_information("fast", "https://fixture/book");
    service
        .more_information("fast", "https://fixture/book")
        .await
        .unwrap();
    assert!(service
        .store
        .database()
        .information_id("fast", "https://fixture/book", "text")
        .unwrap()
        .is_some());
}
