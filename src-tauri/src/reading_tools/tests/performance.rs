//! 离线测量现有数据库热路径，不执行网络请求。
use crate::{
    source_engine::{EngineFuture, SourceEngineRequest, SourceExecutor},
    test_support::TestApp,
};
use serde_json::json;
use std::{sync::Arc, time::Instant};
struct Executor;
impl SourceExecutor for Executor {
    fn execute<'a>(&'a self, _: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async { Err("不执行规则".into()) })
    }
}
#[tokio::test]
#[ignore = "显式运行数据库热路径基准"]
async fn database_storage_workload() {
    let root = tempfile::tempdir().unwrap();
    let service = TestApp::open_with_executor(root.path(), Arc::new(Executor))
        .await
        .unwrap();
    let chapters = (0..5000)
        .map(|i| json!({"id":format!("chapter-{i}"),"title":i.to_string()}))
        .collect::<Vec<_>>();
    service.database.import_book(serde_json::from_value(json!({"id":"perf-book","contentType":"text","title":"TestApp","chapters":chapters})).unwrap()).unwrap().wait().unwrap();
    let started = Instant::now();
    for i in 0..30 {
        service.text.save_progress(&service.database, "perf-book",json!({"chapterId":"chapter-4999","chapterIndex":4999,"offset":i,"updatedAtMs":i+1})).await.unwrap();
    }
    eprintln!("5000 章目录下更新 30 次进度：{:?}", started.elapsed());
    let book = service.database.book("perf-book").unwrap().unwrap();
    assert_eq!(book.chapters.len(), 5000);
}
