use super::*;
use crate::{
    application::ApplicationService,
    source_engine::{EngineFuture, SourceEngineRequest, SourceExecutor},
};
use serde_json::json;

#[tokio::test]
async fn source_catalog_reuses_reads_and_refreshes_after_writes_and_restore() {
    let directory = tempfile::tempdir().unwrap();
    let service =
        ApplicationService::open_with_executor(directory.path(), Arc::new(UnusedExecutor))
            .await
            .unwrap();
    let mut source = SourceRecord {
        id: "source-cache-test".into(),
        name: "Test".into(),
        group: None,
        enabled: true,
        source: json!({"bookSourceName": "Test", "bookSourceUrl": "https://fixture"}),
    };
    service.sources.replace(&[source.clone()]).await.unwrap();
    let first = service.sources.all().await.unwrap();
    assert!(Arc::ptr_eq(&first, &service.sources.all().await.unwrap()));
    source.name = "Edited".into();
    service.sources.replace(&[source.clone()]).await.unwrap();
    assert_eq!(
        first[0].name, "Test",
        "An in-flight snapshot stays immutable"
    );
    assert_eq!(
        service.sources.get(&source.id).await.unwrap().name,
        "Edited"
    );
    let revision = service.sources.revision(&source.id).await;
    service.store.database().submit(serde_json::from_value(json!([{"operation":"saveSources","sources":[{"id":source.id,"definition":json!({"bookSourceName":"From frontend","bookSourceUrl":"https://fixture","enabled":false}).to_string()}]}])).unwrap()).unwrap().await.unwrap();
    service.sources.invalidate().await;
    let latest = service.sources.get(&source.id).await.unwrap();
    assert_eq!(latest.name, "From frontend");
    assert!(!latest.enabled);
    assert!(service.sources.revision(&source.id).await > revision);
    service.sources.replace(&[]).await.unwrap();
    service.sources.invalidate().await;
    assert!(
        service.sources.lookup(&source.id).await.unwrap().is_none(),
        "Restore invalidation reloads the file"
    );
}

struct UnusedExecutor;

impl SourceExecutor for UnusedExecutor {
    fn execute<'a>(&'a self, _request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async { Err("This test must not execute source rules".into()) })
    }
}

#[tokio::test]
async fn source_tables_preserve_rule_fields_and_large_integer_identity() {
    let directory=tempfile::tempdir().unwrap();
    let service=ApplicationService::open_with_executor(directory.path(),Arc::new(UnusedExecutor)).await.unwrap();
    let source=SourceRecord{id:"source-test".into(),name:"Test".into(),group:None,enabled:true,
        source:json!({"bookSourceName":"Test","bookSourceUrl":"https://example.invalid","ruleSearch":{"bookList":"x".repeat(100000)},"lastUpdateTime":9007199254740993_u64})};
    service.sources.replace(&[source.clone()]).await.unwrap();
    let records=service.store.database().sources().unwrap();
    assert_eq!(records[0].definition.last_update_time.unwrap() as u64,9007199254740993_u64);
    assert_eq!(records[0].definition.rule_search.as_ref().unwrap()["bookList"],source.source["ruleSearch"]["bookList"]);
    service.sources.replace(&[]).await.unwrap();
    assert!(service.store.database().sources().unwrap().is_empty());
}

#[tokio::test]
async fn revisions_reject_reverted_edits_and_restores_without_a_persisted_file() {
    let directory = tempfile::tempdir().unwrap();
    let service =
        ApplicationService::open_with_executor(directory.path(), Arc::new(UnusedExecutor))
            .await
            .unwrap();
    service.bootstrap().await.unwrap();
    let source = SourceRecord {
        id: "source-test".into(),
        name: "Original".into(),
        group: None,
        enabled: true,
        source: json!({
            "bookSourceName": "Original",
            "bookSourceUrl": "https://example.invalid",
            "enabled": true,
        }),
    };
    service.sources.replace(&[source.clone()]).await.unwrap();
    let initial = service.sources.revision(&source.id).await;
    service.sources.replace(&[source.clone()]).await.unwrap();
    assert_eq!(service.sources.revision(&source.id).await, initial);

    let backup_directory = tempfile::tempdir().unwrap();
    let archive = backup_directory.path().join("backup.zip");
    crate::backup::create_backup(&service.store, &archive)
        .await
        .unwrap();
    let mut edited = source.clone();
    edited.source["ruleSearch"] = json!({ "name": "changed" });
    service.sources.replace(&[edited]).await.unwrap();
    service.sources.replace(&[source.clone()]).await.unwrap();
    let reverted = service.sources.revision(&source.id).await;
    assert!(reverted > initial);

    service.sources.replace(&[]).await.unwrap();
    service.sources.replace(&[source.clone()]).await.unwrap();
    let reimported = service.sources.revision(&source.id).await;
    assert!(reimported > reverted);
    service.restore_backup(&archive).await.unwrap();
    assert!(service.sources.revision(&source.id).await > reimported);
    assert!(!directory.path().join("source-revisions.json").exists());
    let file = std::fs::File::open(archive).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert!(archive.by_name("source-revisions.json").is_err());

    drop(service);
    let reopened =
        ApplicationService::open_with_executor(directory.path(), Arc::new(UnusedExecutor))
            .await
            .unwrap();
    assert_eq!(reopened.sources.revision(&source.id).await, 0);
    assert_eq!(
        reopened.sources.all().await.unwrap()[0].source,
        source.source
    );
}

impl SourceCatalog {
    pub(crate) async fn replace(&self, sources: &[SourceRecord]) -> Result<bool, String> {
        let previous = self.all().await?;
        if previous.as_slice() == sources {
            return Ok(false);
        }
        let mut inputs = sources.iter().map(|record| {
            let mut definition = record.source.clone();
            definition["bookSourceName"] = json!(record.name);
            definition["bookSourceGroup"] = json!(record.group);
            definition["enabled"] = json!(record.enabled);
            json!({"id":record.id,"definition":definition.to_string(),"removed":false})
        }).collect::<Vec<_>>();
        inputs.extend(previous.iter().filter(|old| !sources.iter().any(|s| s.id == old.id))
            .map(|old| json!({"id":old.id,"removed":true})));
        self.database.submit(serde_json::from_value(json!([
            {"operation":"saveSources","sources":inputs}
        ])).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?
            .await.map_err(|e|e.to_string())?;
        self.invalidate().await;
        Ok(true)
    }
}
