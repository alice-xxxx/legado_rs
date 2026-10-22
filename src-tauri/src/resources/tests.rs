//! 资源模块可独立启动和恢复，不创建应用服务。
use super::Resources;

#[tokio::test]
async fn standalone_bootstrap_cache_and_proxy_recovery() {
    let root = tempfile::tempdir().unwrap();
    let database = crate::database::Database::new(root.path()).unwrap();
    let resources = Resources::open(root.path(), database.clone())
        .await
        .unwrap();
    let bootstrap = resources.bootstrap().await.unwrap();
    assert_eq!(bootstrap["assetBaseUrl"], resources.base_url());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let settings = client
        .get(format!("{}settings.json", resources.base_url()))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&settings).unwrap(),
        bootstrap["settings"]
    );

    database
        .import_book(crate::database::Book {
            id: "book".into(),
            content_type: "text".into(),
            ..Default::default()
        })
        .unwrap()
        .await
        .unwrap();
    resources
        .cache
        .save_asset("text", "book", "1.txt", b"cached")
        .await
        .unwrap();
    assert_eq!(
        resources.cache_usage().await.unwrap()["localChapterResourceBytes"],
        6
    );
    resources
        .register_proxy("book", "request", "http://127.0.0.1:9/audio", &[])
        .await
        .unwrap();
    drop(resources);

    let reopened = Resources::open(root.path(), database).await.unwrap();
    assert!(reopened.proxy.contains("request"));
    reopened.proxy.clear();
    reopened.reload_proxy().await.unwrap();
    assert!(reopened.proxy.contains("request"));
}
