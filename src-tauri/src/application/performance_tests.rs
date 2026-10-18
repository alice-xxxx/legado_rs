//! Repeatable CPU/I/O workloads without network latency or a platform WebView.
use super::*;

struct FixtureExecutor;
impl SourceExecutor for FixtureExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            Ok(json!({"books": (0..10).map(|index| json!({
                "name": format!("fixture-{}-{index}", request.source["bookSourceName"].as_str().unwrap()),
                "bookUrl": format!("https://fixture/{index}"), "author": "fixture",
            })).collect::<Vec<_>>(), "hasNextPage": false}))
        })
    }
}

#[tokio::test]
#[ignore = "Run explicitly to measure application hot paths"]
async fn application_hot_path_workload() {
    let directory = tempfile::tempdir().unwrap();
    let service = ApplicationService::open_with_executor(directory.path(), Arc::new(FixtureExecutor)).await.unwrap();
    let records = (0..1000).map(|index| SourceRecord {
        id: format!("source-{index}"), name: format!("source-{index}"), group: None, enabled: true,
        source: json!({"bookSourceName": format!("source-{index}"), "bookSourceUrl": format!("https://fixture/{index}"),
            "bookSourceType": 0, "ruleSearch": {"name": "x".repeat(8192)}}),
    }).collect::<Vec<_>>();
    service.write_sources(&records).await.unwrap();
    let source_ref = ResourceRef::new("resource://sources.json").unwrap();
    let started = Instant::now();
    for _ in 0..30 {
        let document = service.store.read_json_ref(&source_ref).await.unwrap();
        let decoded: Vec<SourceRecord> = serde_json::from_value(document).unwrap();
        validate_source_records(&decoded).unwrap();
        assert!(decoded.iter().any(|record| record.id == "source-999"));
    }
    eprintln!("PERF source_lookup_file_baseline_30_ms={}", started.elapsed().as_millis());
    let started = Instant::now();
    for _ in 0..30 { assert_eq!(service.find_source("source-999").await.unwrap().id, "source-999"); }
    eprintln!("PERF source_lookup_current_30_us={}", started.elapsed().as_micros());

    let data_dir = directory.path().join("source-engine");
    service.store.database().transaction(|tx| {
        for index in 0..1000 { tx.execute("INSERT INTO source_kv(namespace,key,value) VALUES('source-cache',?1,?2)",rusqlite::params![format!("key-{index}"),"x".repeat(1024)])?; }
        Ok(())
    }).unwrap();
    let started = Instant::now();
    for _ in 0..100 {
        let request = serde_json::from_value(json!({"namespace": "source-cache", "operation": "get", "key": "key-999"})).unwrap();
        assert!(crate::source_storage::execute_source_storage_request(request, &data_dir).await.unwrap().value.is_some());
    }
    eprintln!("PERF storage_get_100_ms={}", started.elapsed().as_millis());

    let batches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let delivered = Arc::new(std::sync::Mutex::new(HashSet::<String>::new()));
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let output_batches = batches.clone();
    let output_bytes = bytes.clone();
    let output_delivered = delivered.clone();
    let output_completed = completed.clone();
    let selected = records.iter().take(150).map(|source| source.id.clone()).collect::<Vec<_>>();
    let started = Instant::now();
    let response = service.start_search("text", &selected, "fixture", 1, false, Some(Arc::new(move |batch| {
        output_batches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        output_bytes.fetch_add(serde_json::to_vec(&batch).unwrap().len(), std::sync::atomic::Ordering::Relaxed);
        for result in batch["results"].as_array().into_iter().flatten() {
            output_delivered.lock().unwrap().insert(result["resultId"].as_str().unwrap().to_owned());
        }
        if batch["complete"] == true { output_completed.store(true, std::sync::atomic::Ordering::Release); }
    }))).await.unwrap();
    let task_id = response["taskId"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let status = {
                let registry = service.tasks.lock().await;
                registry.records.iter().find(|task| task.id == task_id).unwrap().status.clone()
            };
            // The task record commits before the final IPC flush. Measure and
            // validate delivery too, so batching cannot hide missing results.
            if status == "completed" && completed.load(std::sync::atomic::Ordering::Acquire) { break; }
            assert!(!matches!(status.as_str(), "failed" | "recoveryRequired"));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let elapsed = started.elapsed();
    let result = service.get_search_task_results(task_id).await.unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 1500);
    assert_eq!(result["errors"], json!([]));
    assert_eq!(delivered.lock().unwrap().len(), 1500);
    eprintln!("PERF search_150_sources_1500_books_ms={} batches={} ipc_bytes={}", elapsed.as_millis(),
        batches.load(std::sync::atomic::Ordering::Relaxed), bytes.load(std::sync::atomic::Ordering::Relaxed));
}

#[tokio::test]
#[ignore = "Run explicitly to compare JSON rewrites with database record updates"]
async fn database_storage_workload() {
    let directory=tempfile::tempdir().unwrap();
    let service=ApplicationService::open_with_executor(directory.path(),Arc::new(FixtureExecutor)).await.unwrap();
    let path="cache/text/perf-book/perf-book.json";
    let chapters=(0..5000).map(|index|json!({"id":format!("chapter-{index}"),"title":format!("Chapter {index}"),
        "engineurl":format!("https://fixture/{index}"),"engineData":{"ruleData":"x".repeat(256)}})).collect::<Vec<_>>();
    let document=json!({"id":"perf-book","title":"Performance fixture","chapters":chapters});
    service.store.database().write(path,&document).unwrap();
    let legacy=directory.path().join("baseline.json");
    let bytes=serde_json::to_vec(&document).unwrap();
    std::fs::write(&legacy,&bytes).unwrap();
    let started=Instant::now();
    for index in 0..30 {
        // The former path decoded the complete file, built a runtime view,
        // encoded every chapter and atomically replaced the entire document.
        let raw: Value=serde_json::from_slice(&std::fs::read(&legacy).unwrap()).unwrap();
        let mut view=crate::resources::content::book_view(&raw,"text").unwrap();
        view["progress"]=json!({"chapterId":"chapter-4999","chapterIndex":4999,"offset":index,"updatedAtMs":index+1});
        let encoded=crate::resources::content::encode_book(&view,None,&raw,"text").unwrap();
        let bytes=serde_json::to_vec(&encoded).unwrap();
        use std::io::Write;
        atomicwrites::AtomicFile::new(&legacy,atomicwrites::AllowOverwrite).write(|file| {file.write_all(&bytes)?;file.sync_all()}).unwrap();
    }
    let old_time=started.elapsed();
    let started=Instant::now();
    for index in 0..30 {
        service.save_progress("perf-book",json!({"chapterId":"chapter-4999","chapterIndex":4999,"offset":index,"updatedAtMs":index+1})).await.unwrap();
    }
    eprintln!("PERF progress_5000_chapters_30_saves json_ms={} sqlite_ms={} json_document_bytes={}",old_time.as_millis(),started.elapsed().as_millis(),bytes.len());
    // A database trigger rejects even an unchanged chapter UPDATE. These
    // workloads must operate exclusively on progress, metadata and media rows.
    service.store.database().transaction(|tx|{tx.execute_batch("CREATE TRIGGER no_chapter_updates BEFORE UPDATE ON chapters BEGIN SELECT RAISE(ABORT,'chapter rewrite'); END;")?;Ok(())}).unwrap();
    let started = Instant::now();
    for index in 0..100 {
        service.set_alternatives("perf-book", vec![json!({"sourceId": format!("source-{index}"), "engineurl": format!("https://fixture/{index}"), "engine": {"name": "Fixture"}})]).await.unwrap();
    }
    eprintln!("PERF alternatives_5000_chapters_100_sources_ms={}", started.elapsed().as_millis());
    let started=Instant::now();
    for index in 0..100 {
        service.store.database().set_media(path,"perf-book",&format!("image-{index}"),Some(&json!({"url":format!("https://fixture/{index}.webp")}))).unwrap();
    }
    eprintln!("PERF media_5000_chapters_100_registrations_sqlite_ms={}",started.elapsed().as_millis());
    let started=Instant::now();
    service.save_book(json!({"id":"perf-book","displayOverrides":{"title":"Edited"}})).await.unwrap();
    service.reset_book_progress("perf-book").await.unwrap();
    service.save_shelf(json!({"schemaVersion":1,"books":[{"id":"perf-book","groups":[]}],"groups":[]})).await.unwrap();
    eprintln!("PERF metadata_reset_shelf_5000_chapters_sqlite_ms={}",started.elapsed().as_millis());
    let raw=service.store.database().read(path).unwrap().unwrap();
    assert_eq!(raw["chapters"].as_array().unwrap().len(),5000);
    assert_eq!(raw["media"].as_object().unwrap().len(),100);
    assert!(raw["progress"].is_null());
}
