//! 检查提交回滚、会话竞争和旧源协议，不依赖真实站点或原生窗口。

use super::*;
use crate::database::{Database, Mutation, SourceDefinition, SourceInput};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tauri_plugin_source_engine::*;
use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

fn source(name: &str) -> SourceInput {
    SourceInput {
        id: Some("source".into()),
        definition: Some(SourceDefinition {
            book_source_url: "https://example.org".into(),
            book_source_name: name.into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
async fn save(database: &Database, input: SourceInput) {
    database
        .submit(vec![Mutation::SaveSources {
            sources: vec![input],
        }])
        .unwrap()
        .await
        .unwrap();
}
#[tokio::test]
async fn failed_transaction_keeps_catalog_and_persistent_source() {
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    save(&database, source("初始源")).await;
    let catalog = Catalog::new(database.clone());
    let old = catalog.get("source").await.unwrap();
    let mut invalid = source("无效源");
    invalid.id = Some("invalid".into());
    invalid.definition.as_mut().unwrap().book_source_url.clear();
    let result = database
        .submit(vec![Mutation::SaveSources {
            sources: vec![source("不应提交"), invalid],
        }])
        .unwrap()
        .await;
    assert!(result.is_err());
    assert_eq!(catalog.version(), 0);
    assert_eq!(catalog.get("source").await.unwrap(), old);
    assert_eq!(
        database.sources().unwrap()[0].definition.book_source_name,
        "初始源"
    );
    let guard = catalog.write_guard().await;
    save(&database, source("新源")).await;
    catalog.invalidate().await;
    drop(guard);
    assert_eq!(catalog.get("source").await.unwrap().name, "新源");
    assert!(catalog.current(&old, 0).await.is_err());
}

type Call = (LoginAction, CancellationToken, oneshot::Sender<LoginState>);
struct LoginEngine(mpsc::UnboundedSender<Call>);
macro_rules! unexpected {
    ($name:ident,$request:ty,$response:ty) => {
        fn $name<'a>(&'a self, _: $request, _: CancellationToken) -> EngineFuture<'a, $response> {
            Box::pin(async { Err("登录流程不应调用其他引擎能力".into()) })
        }
    };
}
impl SourceExecutor for LoginEngine {
    unexpected!(search, SearchRequest, SearchPage);
    unexpected!(explore_kinds, SourceRequest, ExploreKinds);
    unexpected!(explore, ExploreRequest, SearchPage);
    unexpected!(detail, BookRequest, Book);
    unexpected!(chapters, BookRequest, ChapterList);
    unexpected!(content, ContentRequest, ChapterContent);
    unexpected!(resolve_media, MediaRequest, MediaLocation);
    unexpected!(http_tts_audio, TtsRequest, AudioData);
    fn login<'a>(
        &'a self,
        request: LoginRequest,
        cancel: CancellationToken,
    ) -> EngineFuture<'a, LoginState> {
        Box::pin(async move {
            let (sender, receiver) = oneshot::channel();
            self.0
                .send((request.action, cancel, sender))
                .map_err(|e| e.to_string())?;
            receiver.await.map_err(|e| e.to_string())
        })
    }
}
#[tokio::test]
async fn newer_open_and_committed_source_change_cancel_older_login() {
    use login::{Action, Login};
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    save(&database, source("初始源")).await;
    let catalog = Arc::new(Catalog::new(database.clone()));
    let (sender, mut calls) = mpsc::unbounded_channel();
    let backup = Arc::new(crate::backup::state::Operations::new());
    let login = Arc::new(Login::new(
        catalog.clone(),
        Arc::new(LoginEngine(sender)),
        backup.clone(),
    ));
    let open = |service: Arc<Login>| {
        tokio::spawn(async move { service.act("source", Action::Open).await })
    };
    let first = open(login.clone());
    let (_, first_cancel, first_reply) = timeout(Duration::from_secs(2), calls.recv())
        .await
        .unwrap()
        .unwrap();
    let second = open(login.clone());
    let (_, _, reply) = timeout(Duration::from_secs(2), calls.recv())
        .await
        .unwrap()
        .unwrap();
    reply
        .send(LoginState {
            message: Some("新会话".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        second.await.unwrap().unwrap().unwrap().message.as_deref(),
        Some("新会话")
    );
    assert!(first.await.unwrap().is_err());
    assert!(first_cancel.is_cancelled());
    assert!(first_reply.send(LoginState::default()).is_err());
    let service = login.clone();
    let submit = tokio::spawn(async move {
        service
            .act(
                "source",
                Action::Submit {
                    fields: HashMap::new(),
                    button: None,
                },
            )
            .await
    });
    let (_, cancel, late) = timeout(Duration::from_secs(2), calls.recv())
        .await
        .unwrap()
        .unwrap();
    let guard = catalog.write_guard().await;
    save(&database, source("修改后的源")).await;
    catalog.invalidate().await;
    drop(guard);
    timeout(Duration::from_secs(2), cancel.cancelled())
        .await
        .unwrap();
    assert!(submit.await.unwrap().is_err());
    assert!(late.send(LoginState::default()).is_err());
    assert!(login
        .act(
            "source",
            Action::Submit {
                fields: HashMap::new(),
                button: None
            }
        )
        .await
        .is_err());
    let restoring = open(login.clone());
    let (_, cancel, _reply) = timeout(Duration::from_secs(2), calls.recv())
        .await
        .unwrap()
        .unwrap();
    backup.restore_barrier.send_replace(true);
    timeout(Duration::from_secs(2), cancel.cancelled())
        .await
        .unwrap();
    assert!(restoring.await.unwrap().is_err());
    assert!(login.act("source", Action::Open).await.is_err());
    backup.restore_barrier.send_replace(false);
    login.cancel_login().await.unwrap();
}
#[test]
fn legacy_rss_import_preserves_rules_scripts_and_integer_precision() {
    let source = json!({"sourceUrl":"https://example.org/rss","sourceName":"订阅","ruleArticles":"article",
        "ruleTitle":"h1@text","ruleLink":"a@href","ruleContent":"div@html","injectJs":"window.ready=true;",
        "style":"body{font-family:\"中文\"}","lastUpdateTime":9223372036854775000i64});
    let preview = import::preview(&json!({"data":[source]}).to_string()).unwrap();
    let raw = preview["sources"][0].as_str().unwrap();
    assert!(raw.contains("9223372036854775000"));
    let converted: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(converted["bookSourceType"], 5);
    assert_eq!(converted["ruleExplore"]["bookList"], "article");
    assert_eq!(converted["ruleContent"]["content"], "div@html");
    assert!(converted["ruleContent"]["webJs"]
        .as_str()
        .unwrap()
        .ends_with("window.ready=true;"));
    let definition: SourceDefinition = serde_json::from_value(converted).unwrap();
    assert_eq!(definition.book_source_url, "https://example.org/rss");
}
#[tokio::test]
async fn linked_import_deduplicates_cycles_and_preserves_source_order() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let base = origin.clone();
    let server = tokio::spawn(async move {
        let mut paths = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            paths.push(path.clone());
            let body = if path == "/list" {
                json!({"sourceUrls":[format!("{base}/list"),format!("{base}/feed"),format!("{base}/feed")]}).to_string()
            } else {
                json!([{"bookSourceUrl":"https://example.org/a","bookSourceName":"甲"},{"bookSourceUrl":"https://example.org/b","bookSourceName":"乙"}]).to_string()
            };
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        paths
    });
    let resolved = timeout(
        Duration::from_secs(3),
        import::resolve(&format!("{origin}/list")),
    )
    .await
    .unwrap()
    .unwrap();
    let document = import::preview(&resolved).unwrap();
    let names: Vec<String> = document["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            serde_json::from_str::<Value>(v.as_str().unwrap()).unwrap()["bookSourceName"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(names, ["甲", "乙"]);
    assert_eq!(server.await.unwrap(), ["/list", "/feed"]);
}

#[tokio::test]
async fn standard_feed_remains_a_feed_after_database_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    let preview =
        import::preview(r#"{"feedUrl":"https://example.org/feed.xml","title":"订阅"}"#).unwrap();
    let raw = preview["sources"][0].as_str().unwrap();
    let definition: SourceDefinition = serde_json::from_str(raw).unwrap();
    save(
        &database,
        SourceInput {
            id: Some("source".into()),
            definition: Some(definition),
            ..Default::default()
        },
    )
    .await;
    let catalog = Catalog::new(database);
    let persisted = catalog.get("source").await.unwrap();
    assert!(tauri_plugin_source_engine::source::is_standard_feed_source(
        &persisted.source
    ));
    assert!(tauri_plugin_source_engine::source::is_rss_source(
        &persisted.source
    ));
    assert_eq!(
        persisted.source["bookSourceUrl"],
        "https://example.org/feed.xml"
    );
    assert!(import::preview(&persisted.source.to_string()).is_ok());
}
