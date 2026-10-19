use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Fixture {
    calls: std::sync::Mutex<Vec<(String, String, i32)>>,
    pending: AtomicUsize,
}
impl SourceExecutor for Fixture {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            if request.operation != "search" {
                return match request.operation.as_str() {
                    "bookInfo" => Ok(request.book.unwrap()),
                    "chapters" => {
                        Ok(json!([{"title":"Chapter","url":"https://fixture/chapter"}]))
                    }
                    "content" => Ok(json!("replacement chapter content")),
                    _ => Err("Unexpected fixture operation".into()),
                };
            }
            let name = request.source["bookSourceName"]
                .as_str()
                .unwrap()
                .to_owned();
            self.calls.lock().unwrap().push((
                name.clone(),
                request.keyword.clone().unwrap(),
                request.page.unwrap(),
            ));
            match name.as_str() {
                "pending" => {
                    self.pending.fetch_add(1, Ordering::SeqCst);
                    request.cancellation.unwrap().cancelled().await;
                    self.pending.fetch_sub(1, Ordering::SeqCst);
                    Err("cancelled".into())
                }
                "failure" => Err("fixture engine failure".into()),
                "invalid" => Ok(json!({"unexpected": []})),
                "unsorted" => Ok(json!({"books":[
                    {"name":"无关","author":"作者","bookUrl":"https://fixture/other"},
                    {"name":"凡人续集","author":"作者","bookUrl":"https://fixture/contains"},
                    {"name":"凡人","author":"作者","bookUrl":"https://fixture/exact"}
                ],"hasNextPage":false})),
                "different-author" => Ok(
                    json!({"books":[{"name":"凡人","author":"另一位作者","bookUrl":"https://different-author/book"}],"hasNextPage":false}),
                ),
                "paging" => Ok(
                    json!({"books": [{"name":format!("凡人{}",request.page.unwrap()),"bookUrl":format!("https://fixture/{}",request.page.unwrap()),"author":"作者"}],"hasNextPage":request.page == Some(1)}),
                ),
                _ => Ok(json!({"books":[
                    {"name":"凡人","author":"作者","bookUrl":format!("https://{name}/exact"),"variableMap":{"context":"search-value"}},
                    {"name":"凡人续集","author":"作者","bookUrl":format!("https://{name}/contains")},
                    {"name":"无关","author":"作者","bookUrl":format!("https://{name}/other")}
                ],"hasNextPage":false})),
            }
        })
    }
}

async fn service(names: &[&str]) -> (tempfile::TempDir, ApplicationService, Arc<Fixture>) {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let service = ApplicationService::open_with_executor(directory.path(), fixture.clone())
        .await
        .unwrap();
    let records = names.iter().map(|name| SourceRecord {
        id: (*name).into(), name: (*name).into(), group: None, enabled: true,
        source: json!({"bookSourceName":name,"bookSourceUrl":format!("https://{name}"),"bookSourceType":0}),
    }).collect::<Vec<_>>();
    service.write_sources(&records).await.unwrap();
    (directory, service, fixture)
}

fn updates() -> (
    Arc<dyn Fn(Value) + Send + Sync>,
    tokio::sync::mpsc::UnboundedReceiver<Value>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    (
        Arc::new(move |update| {
            let _ = sender.send(update);
        }),
        receiver,
    )
}
#[tokio::test]
async fn precision_filters_incremental_results_without_tasks_or_snapshots() {
    let (_directory, service, fixture) = service(&["one", "two"]).await;
    let task_ref = service.store.document_ref("tasks").unwrap();
    let task_before = service.store.read_json_ref(&task_ref).await.unwrap();
    for (precision, expected) in [(SearchPrecision::Exact, 2), (SearchPrecision::Precise, 4), (SearchPrecision::None, 6)] {
        let (emit, mut receiver) = updates();
        let id = service.search(&["one".into(), "two".into()], "凡人", 1, precision, emit).await.unwrap();
        let mut count = 0;
        let mut source_count = 0;
        while let Ok(update) = receiver.try_recv() {
            assert_eq!(update["searchId"], id);
            assert!(update.as_object().unwrap().keys().all(|key|
                ["searchId", "sourceId", "results", "hasMore", "error"].contains(&key.as_str())));
            if update.get("sourceId").is_some() { source_count += 1; }
            for result in update["results"].as_array().unwrap() {
                count += 1;
                assert!(result.get("sources").is_none());
                assert!(result["bookUrl"].is_string());
                let cache = service.read_search_result_cache(result["resultId"].as_str().unwrap()).await.unwrap();
                assert_eq!(cache["precision"], json!(precision));
                assert!(cache["book"]["bookUrl"].is_string());
            }
        }
        assert_eq!((source_count, count), (2, expected));
        service.release_search_results(&id).await.unwrap();
        assert!(service.searches.lock().await.search_result_documents.is_empty());
    }
    assert_eq!(fixture.calls.lock().unwrap().len(), 6);
    assert_eq!(service.store.read_json_ref(&task_ref).await.unwrap(), task_before);
}

#[tokio::test]
async fn fast_source_is_emitted_before_slow_source_and_cancellation_keeps_results() {
    let (_directory, service, _fixture) = service(&["unsorted", "pending"]).await;
    let (emit, mut receiver) = updates();
    let running = service.clone();
    let search = tokio::spawn(async move {
        running.search(&["unsorted".into(), "pending".into()], "凡人", 1, SearchPrecision::None, emit).await.unwrap()
    });
    let initial = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await.unwrap().unwrap();
    let update = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await.unwrap().unwrap();
    assert!(!search.is_finished());
    assert_eq!(update["sourceId"], "unsorted");
    assert_eq!(update["results"].as_array().unwrap().iter().map(|book| book["title"].as_str().unwrap()).collect::<Vec<_>>(), ["无关", "凡人续集", "凡人"]);
    let result_id = update["results"][0]["resultId"].as_str().unwrap();
    service.stop_search(initial["searchId"].as_str().unwrap()).await.unwrap();
    let id = tokio::time::timeout(Duration::from_secs(2), search).await.unwrap().unwrap();
    assert_eq!(initial["searchId"], id);
    assert!(receiver.try_recv().is_err()); // 不额外发送结束快照。
    assert!(service.read_search_result_cache(result_id).await.is_ok());
    service.release_search_results(&id).await.unwrap();
    assert!(service.read_search_result_cache(result_id).await.is_err());
}

#[tokio::test]
async fn source_errors_do_not_discard_successful_results() {
    let (_directory, service, _fixture) = service(&["one", "failure", "invalid"]).await;
    let (emit, mut receiver) = updates();
    service.search(&["one".into(), "failure".into(), "invalid".into()], "凡人", 1, SearchPrecision::Exact, emit).await.unwrap();
    let mut errors = 0;
    let mut results = 0;
    while let Ok(update) = receiver.try_recv() {
        errors += usize::from(update.get("error").is_some());
        results += update["results"].as_array().unwrap().len();
    }
    assert_eq!((errors, results), (2, 1));
}

#[tokio::test]
async fn shared_writer_keeps_alternative_context_after_search_is_released() {
    let (_directory, service, _) = service(&["one", "two"]).await;
    let (emit, mut receiver) = updates();
    let search_id = service.search(&["two".into()], "凡人", 1, SearchPrecision::Exact, emit).await.unwrap();
    receiver.recv().await.unwrap();
    let update = receiver.recv().await.unwrap();
    let result = &update["results"][0];
    assert_eq!(result["engine"]["variableMap"]["context"], "search-value");
    service.more_information("one", "https://one/exact").await.unwrap();
    let book_id = service.store.database().information_id("one", "https://one/exact", "text").unwrap().unwrap();
    service.store.database().modify(serde_json::from_value(json!([{
        "action":"upsert","table":"alternative_sources","keys":["book_id","source_id","engine_url"],
        "values":{"book_id":book_id,"source_id":result["sourceId"],"engine_url":result["bookUrl"],"position":0,
            "source_name":result["sourceName"],"title":result["title"],"engine_json":result["engine"]}
    }])).unwrap()).unwrap();
    service.release_search_results(&search_id).await.unwrap();
    assert!(service.searches.lock().await.search_result_documents.is_empty());
    // 搜索会话已释放，详情仍能从统一写入的数据库上下文继续执行规则。
    service.more_information("two", "https://two/exact").await.unwrap();
    let book = service.store.database().information_seed("two", "https://two/exact", "text").unwrap().unwrap();
    assert_eq!(book["variableMap"]["context"], "search-value");
}

#[tokio::test]
async fn pages_are_independent_and_history_is_written_by_search() {
    let (_directory, service, fixture) = service(&["paging"]).await;
    for page in [1, 7] {
        let (emit, mut receiver) = updates();
        service.search(&["paging".into()], " 凡人 ", page, SearchPrecision::Precise, emit).await.unwrap();
        receiver.recv().await.unwrap();
        let update = receiver.recv().await.unwrap();
        assert_eq!(update["results"].as_array().unwrap().len(), 1);
        assert_eq!(update["results"][0]["title"], format!("凡人{page}"));
        assert_eq!(update["hasMore"], page == 1);
    }
    assert_eq!(fixture.calls.lock().unwrap().iter().map(|call| call.2).collect::<Vec<_>>(), [1, 7]);
    let history = crate::search_history::load(&service.store).await.unwrap();
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].query, "凡人");
    assert_eq!(history.entries[0].usage, 2);
}

#[tokio::test]
async fn exact_result_can_replace_a_chapter_before_other_sources_finish() {
    let (_directory, service, _fixture) = service(&["one", "different-author", "pending"]).await;
    let (emit, mut receiver) = updates();
    service.search(&["one".into()], "凡人", 1, SearchPrecision::Exact, emit).await.unwrap();
    receiver.recv().await.unwrap();
    let initial = receiver.recv().await.unwrap();
    let book_id = initial["results"][0]["resultId"].as_str().unwrap();
    service.more_information("one", initial["results"][0]["bookUrl"].as_str().unwrap()).await.unwrap();
    let book_ref = service.store.book_ref(book_id).unwrap();
    let document = service.store.read_json_ref(&book_ref).await.unwrap();
    let chapter_id = document["chapters"][0]["id"].as_str().unwrap();
    let (emit, mut receiver) = updates();
    let running = service.clone();
    let search = tokio::spawn(async move {
        running.search(&["different-author".into(), "pending".into()], "凡人", 1, SearchPrecision::Exact, emit).await.unwrap()
    });
    receiver.recv().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await.unwrap().unwrap();
    assert!(!search.is_finished());
    // 只使用 exact 过滤结果，不再要求整轮完成或额外核对作者。
    service.more_information("different-author",result["results"][0]["bookUrl"].as_str().unwrap()).await.unwrap();
    let source_id = result["results"][0]["resultId"].as_str().unwrap();
    let source_ref = service.store.book_ref(source_id).unwrap();
    let source = service.store.read_json_ref(&source_ref).await.unwrap();
    let source_chapter = source["chapters"][0]["id"].as_str().unwrap();
    service.main_text(source_id, source_chapter, false).await.unwrap();
    let cached = service.store.read_json_ref(&source_ref).await.unwrap();
    let mut chapter = document["chapters"][0].clone();
    // 模拟 Vue 只保存地址，不复制正文，也不改动两本书的来源身份。
    chapter["resource"] = cached["chapters"][0]["resource"].clone();
    service.store.database().save_chapter_view(book_id, chapter_id, &chapter).unwrap();
    let updated = service.store.read_json_ref(&book_ref).await.unwrap();
    assert_eq!(updated["chapters"][0]["resource"]["resourceId"], cached["chapters"][0]["resource"]["resourceId"]);
    assert_eq!(updated["sourceId"], document["sourceId"]);
    assert!(service.indexed_chapter_ready(&updated, 0).await.unwrap());
    assert!(service.store.read_json_ref(&source_ref).await.unwrap() == cached);
    // 原书清缓存只解除引用，被借用的书源缓存仍可读取。
    service.clear_book_chapter_cache(book_id).await.unwrap();
    assert!(service.indexed_chapter_ready(&cached, 0).await.unwrap());
    service.clear_searches().await;
    tokio::time::timeout(Duration::from_secs(2), search).await.unwrap().unwrap();
}
