//! 书源定义的基础校验、登录与缓存。

use super::*;

pub(super) struct SourceCatalog {
    records: Arc<Vec<SourceRecord>>,
    by_id: HashMap<String, usize>,
}

impl SourceCatalog {
    fn new(records: Vec<SourceRecord>) -> Self {
        let by_id = records.iter().enumerate().map(|(index, source)| (source.id.clone(), index)).collect();
        Self { records: Arc::new(records), by_id }
    }
}

/// Validate only the structural identity of a Legado JSON definition.
/// Everything else, including unknown and nested keys, is retained verbatim.
pub(crate) fn validate_source_definition(
    source: &Value,
) -> Result<(String, Option<String>, bool, String), String> {
    let object = source
        .as_object()
        .ok_or_else(|| "Source definition must be a JSON object".to_owned())?;
    let bytes = serde_json::to_vec(source).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SOURCE_IMPORT_BODY_BYTES {
        return Err("Source definition exceeds the JSON import size limit".into());
    }
    let url = text_at(source, &["bookSourceUrl", "url", "sourceUrl"])
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Source definition must retain a nonempty bookSourceUrl".to_owned())?;
    let name = text_at(source, &["bookSourceName", "name", "sourceName"])
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Source definition must retain a nonempty bookSourceName".to_owned())?;
    let group = text_at(source, &["bookSourceGroup", "group"]);
    let enabled = bool_at(source, &["enabled", "bookSourceEnabled"]).unwrap_or(true);
    // The editor must never inject the internal envelope into raw Legado JSON.
    if object.is_empty() {
        return Err("Source definition cannot be empty".into());
    }
    Ok((name, group, enabled, url))
}

pub(crate) fn validate_source_records(records: &[SourceRecord]) -> Result<(), String> {
    let mut ids = HashSet::with_capacity(records.len());
    for record in records {
        validate_id(&record.id, "sourceId")
            .map_err(|error| format!("Invalid saved source definition: {error}"))?;
        if !ids.insert(record.id.as_str()) {
            return Err("Invalid saved source definitions: duplicate sourceId".to_owned());
        }
        validate_source_definition(&record.source)
            .map_err(|error| format!("Invalid saved source '{}': {error}", record.id))?;
    }
    Ok(())
}


impl ApplicationService {
    pub async fn get_source_login_state(&self, source_id: &str) -> Result<Value, String> {
        validate_id(source_id, "sourceId")?;
        let _operation = self.operation_read().await;
        let _sources = self.sources_lock.lock().await;
        let source = self.find_source(source_id).await?;
        if crate::source_metadata::is_rss_source_metadata(&source.source) {
            return Err("RSS sources do not support book-source login state".to_owned());
        }
        let result = self
            .executor
            .execute(engine_request(
                "sourceLoginState",
                &source.source,
                None,
                None,
                None,
                None,
                None,
            ))
            .await?;
        let has_login_state = result
            .get("hasLoginState")
            .and_then(Value::as_bool)
            .ok_or_else(|| "Source engine returned an invalid login-state summary".to_owned())?;
        Ok(json!({ "hasLoginState": has_login_state }))
    }

    pub async fn get_source_login_form(&self, source_id: &str) -> Result<Value, String> {
        validate_id(source_id, "sourceId")?;
        let _operation = self.operation_read().await;
        let _sources = self.sources_lock.lock().await;
        let source = self.find_source(source_id).await?;
        if crate::source_metadata::is_rss_source_metadata(&source.source) {
            return Err("RSS sources do not support book-source login forms".to_owned());
        }
        let result = self
            .executor
            .execute(engine_request(
                "sourceLoginForm",
                &source.source,
                None,
                None,
                None,
                None,
                None,
            ))
            .await
            .map_err(|_| "The source login form could not be loaded".to_owned())?;
        if result.get("mode").and_then(Value::as_str) == Some("web") {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            return Ok(json!({
                "mode": "web",
                "message": result
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("此源使用网页登录。"),
            }));
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            return Ok(json!({
                "mode": "web",
                "message": result
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("此源使用网页登录。"),
            }));
        }
        if result.get("status").and_then(Value::as_str) == Some("unavailable") {
            return Err(result
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("This source has no supported credential login form")
                .to_owned());
        }
        if result.get("mode").and_then(Value::as_str) != Some("form")
            || !result.get("fields").is_some_and(Value::is_array)
            || !result.get("actions").is_some_and(Value::is_array)
        {
            return Err("Source engine returned an invalid login form".to_owned());
        }
        sanitize_source_login_form(&result)
    }

    /// Resolve a web login URL and its current cookie header inside KMP. The caller keeps this
    /// capability private and opens the native browser only after this short engine call returns.
    pub async fn source_login_web_info(&self, source_id: &str) -> Result<Value, String> {
        validate_id(source_id, "sourceId")?;
        let _operation = self.operation_read().await;
        let restore_epoch = self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst);
        let _sources = self.sources_lock.lock().await;
        let source = self.find_source(source_id).await?;
        if crate::source_metadata::is_rss_source_metadata(&source.source) {
            return Err("RSS sources do not support book-source login".to_owned());
        }
        let result = self
            .executor
            .execute(engine_request(
                "sourceLoginWebInfo",
                &source.source,
                None,
                None,
                None,
                None,
                None,
            ))
            .await?;
        let login_url = result
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| "Source engine returned no web login URL".to_owned())?;
        let url =
            reqwest::Url::parse(login_url).map_err(|_| "Source login URL is invalid".to_owned())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Source login URL must be an HTTP(S) URL without credentials".to_owned());
        }
        let cookie_header = result
            .get("cookieHeader")
            .and_then(Value::as_str)
            .ok_or_else(|| "Source engine returned invalid login cookies".to_owned())?;
        if login_url.len() > 8192
            || cookie_header.len() > 256 * 1024
            || cookie_header
                .chars()
                .any(|character| matches!(character, '\r' | '\n'))
        {
            return Err("Source web login data exceeds its supported limits".to_owned());
        }
        let source_revision = self.source_revision(source_id).await?;
        if self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst) != restore_epoch {
            return Err(
                "A backup restore started while the private login browser was opening".to_owned(),
            );
        }
        Ok(json!({
            "url": login_url,
            "cookieHeader": cookie_header,
            "sourceRevision": source_revision,
            "restoreEpoch": restore_epoch,
        }))
    }

    /// Persist cookies captured by the native browser through the same private KMP CookieStore.
    pub async fn import_source_login_cookies(
        &self,
        source_id: &str,
        source_revision: u64,
        restore_epoch: u64,
        login_url: &str,
        cookie_header: &str,
    ) -> Result<Value, String> {
        validate_id(source_id, "sourceId")?;
        if cookie_header.len() > 256 * 1024
            || cookie_header
                .chars()
                .any(|character| matches!(character, '\r' | '\n'))
        {
            return Err("Private browser cookies exceed the supported size".to_owned());
        }
        let _operation = self.operation_read().await;
        let _sources = self.sources_lock.lock().await;
        if self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst) != restore_epoch {
            return Err(
                "A backup was restored while the private login browser was open".to_owned(),
            );
        }
        let source = self.find_source(source_id).await?;
        if self.source_revision(source_id).await? != source_revision {
            return Err("Book source changed while the private login browser was open".to_owned());
        }
        if crate::source_metadata::is_rss_source_metadata(&source.source) {
            return Err("RSS sources do not support book-source login".to_owned());
        }
        let mut request = engine_request(
            "sourceLoginImportCookies",
            &source.source,
            Some(login_url.to_owned()),
            None,
            None,
            None,
            None,
        );
        request.credentials = Some(HashMap::from([(
            "cookieHeader".to_owned(),
            cookie_header.to_owned(),
        )]));
        let result = self.executor.execute(request).await?;
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| "Source engine returned an invalid web login result".to_owned())?;
        if status != "executed" {
            return Err("Source engine could not save the web login cookies".to_owned());
        }
        let imported_count = result
            .get("importedCount")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Source engine returned an invalid cookie count".to_owned())?;
        let message = result
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("网页登录已结束。");
        Ok(json!({
            "status": "executed",
            "importedCount": imported_count,
            "message": message,
        }))
    }

    pub async fn login_source(
        &self,
        source_id: &str,
        credentials: std::collections::HashMap<String, String>,
    ) -> Result<Value, String> {
        self.execute_source_login_operation(source_id, "loginSource", credentials, None)
            .await
    }

    pub async fn run_source_login_action(
        &self,
        source_id: &str,
        action_id: usize,
        credentials: std::collections::HashMap<String, String>,
    ) -> Result<Value, String> {
        self.execute_source_login_operation(
            source_id,
            "sourceLoginAction",
            credentials,
            Some(action_id),
        )
        .await
    }

    async fn execute_source_login_operation(
        &self,
        source_id: &str,
        operation: &str,
        credentials: std::collections::HashMap<String, String>,
        action_id: Option<usize>,
    ) -> Result<Value, String> {
        validate_id(source_id, "sourceId")?;
        let _operation = self.operation_read().await;
        let _sources = self.sources_lock.lock().await;
        let source = self.find_source(source_id).await?;
        if crate::source_metadata::is_rss_source_metadata(&source.source) {
            return Err("RSS sources do not support book-source login".to_owned());
        }
        let mut request = engine_request(operation, &source.source, None, None, None, None, None);
        validate_source_login_credentials(&credentials)?;
        request.credentials = Some(credentials);
        request.action_id = action_id;
        self.executor
            .execute(request)
            .await
            .map_err(|_| "The source login operation could not be completed".to_owned())
            .and_then(sanitize_source_login_result)
    }

    pub async fn clear_source_login_state(&self, source_id: &str) -> Result<Value, String> {
        validate_id(source_id, "sourceId")?;
        let _operation = self.operation_read().await;
        let _sources = self.sources_lock.lock().await;
        let source = self.find_source(source_id).await?;
        if crate::source_metadata::is_rss_source_metadata(&source.source) {
            return Err("RSS sources do not support book-source login state".to_owned());
        }
        let result = self
            .executor
            .execute(engine_request(
                "clearSourceLoginState",
                &source.source,
                None,
                None,
                None,
                None,
                None,
            ))
            .await?;
        if result.get("cleared").and_then(Value::as_bool) != Some(true) {
            return Err("Source engine did not confirm login-state removal".to_owned());
        }
        Ok(json!({ "cleared": true }))
    }

}

impl ApplicationService {
    pub(super) async fn find_source(&self, id: &str) -> Result<SourceRecord, String> {
        self.lookup_source(id).await?
            .ok_or_else(|| format!("Book source '{id}' is no longer imported"))
    }

    pub(super) async fn lookup_source(&self, id: &str) -> Result<Option<SourceRecord>, String> {
        let catalog = self.read_source_catalog().await?;
        Ok(catalog.by_id.get(id).map(|index| catalog.records[*index].clone()))
    }

    pub(super) async fn read_sources(&self) -> Result<Arc<Vec<SourceRecord>>, String> {
        Ok(self.read_source_catalog().await?.records.clone())
    }

    /// 公共写入提交后使后续请求重新读库，已打开的登录会话也重新核对版本。
    pub(crate) async fn invalidate_sources(&self) {
        *self.source_catalog.lock().await = None;
        for revision in self.source_revisions.lock().await.values_mut() {
            *revision = revision.saturating_add(1);
        }
    }

    async fn read_source_catalog(&self) -> Result<Arc<SourceCatalog>, String> {
        let mut cached = self.source_catalog.lock().await;
        if let Some(catalog) = cached.as_ref() { return Ok(catalog.clone()); }
        let reference = ResourceRef::new("resource://sources.json")
            .map_err(|error| error.to_string())?;
        let records = if self
            .store
            .regular_file_exists(&reference)
            .await
            .map_err(|error| error.to_string())?
        {
            let document = self.store.read_json_ref(&reference).await
                .map_err(|error| error.to_string())?;
            let records: Vec<SourceRecord> = serde_json::from_value(document)
                .map_err(|error| format!("Cannot decode saved source definitions: {error}"))?;
            validate_source_records(&records)?;
            records
        } else { Vec::new() };
        let catalog = Arc::new(SourceCatalog::new(records));
        *cached = Some(catalog.clone());
        Ok(catalog)
    }

    /// Session revisions reject delayed results, including an edit followed by
    /// reverting to the same definition. Track queried IDs for restore invalidation.
    pub(super) async fn source_revision(&self, source_id: &str) -> Result<u64, String> {
        Ok(*self
            .source_revisions
            .lock()
            .await
            .entry(source_id.to_owned())
            .or_default())
    }

    pub(super) async fn write_sources(&self, sources: &[SourceRecord]) -> Result<bool, String> {
        let previous = self.read_sources().await?;
        let previous_by_id = previous
            .iter()
            .map(|record| (record.id.as_str(), record))
            .collect::<HashMap<_, _>>();
        let next_ids = sources.iter().map(|source| source.id.as_str()).collect::<HashSet<_>>();
        let mut changed = previous_by_id
            .keys()
            .copied()
            .filter(|id| !next_ids.contains(id))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let previous_positions=previous.iter().enumerate().map(|(index,source)|(source.id.as_str(),index)).collect::<HashMap<_,_>>();
        let changes=sources.iter().enumerate().filter_map(|(index,source)| {
            let edited=previous_by_id.get(source.id.as_str()).copied()!=Some(source);
            if edited { changed.push(source.id.clone()); }
            if edited || previous_positions.get(source.id.as_str()).copied()!=Some(index) {
                Some((index,source.id.clone(),edited.then(||serde_json::to_value(source).expect("serializable source"))))
            } else { None }
        }).collect::<Vec<_>>();
        if changes.is_empty() && changed.is_empty() { return Ok(false); }
        let list_changed = metadata(&previous) != metadata(sources);
        let ids=sources.iter().map(|source|source.id.clone()).collect::<Vec<_>>();
        let db=self.store.database().clone();
        tokio::task::spawn_blocking(move ||db.sync_sources(&changes,&ids)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        {
            let mut revisions = self.source_revisions.lock().await;
            for source_id in changed {
                let revision = revisions.entry(source_id).or_default();
                *revision = revision.saturating_add(1);
            }
        }
        *self.source_catalog.lock().await = Some(Arc::new(SourceCatalog::new(sources.to_vec())));
        Ok(list_changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn source_catalog_reuses_reads_and_refreshes_after_writes_and_restore() {
        let directory = tempfile::tempdir().unwrap();
        let service = ApplicationService::open_with_executor(directory.path(), Arc::new(UnusedExecutor)).await.unwrap();
        let mut source = SourceRecord {
            id: "source-cache-test".into(), name: "Test".into(), group: None, enabled: true,
            source: json!({"bookSourceName": "Test", "bookSourceUrl": "https://fixture"}),
        };
        service.write_sources(&[source.clone()]).await.unwrap();
        let first = service.read_sources().await.unwrap();
        assert!(Arc::ptr_eq(&first, &service.read_sources().await.unwrap()));
        source.name = "Edited".into();
        service.write_sources(&[source.clone()]).await.unwrap();
        assert_eq!(first[0].name, "Test", "An in-flight snapshot stays immutable");
        assert_eq!(service.find_source(&source.id).await.unwrap().name, "Edited");
        let revision = service.source_revision(&source.id).await.unwrap();
        service.store.database().modify(serde_json::from_value(json!([{"action":"upsert","table":"sources","keys":["id"],
            "values":{"id":source.id,"definition_json":{"bookSourceName":"From frontend","bookSourceUrl":"https://fixture","enabled":false}}}])).unwrap()).unwrap();
        service.invalidate_sources().await;
        let latest = service.find_source(&source.id).await.unwrap();
        assert_eq!(latest.name, "From frontend");
        assert!(!latest.enabled);
        assert!(service.source_revision(&source.id).await.unwrap() > revision);
        let reference = ResourceRef::new("resource://sources.json").unwrap();
        service.store.write_json_ref(&reference, &json!([])).await.unwrap();
        *service.source_catalog.lock().await = None;
        assert!(service.lookup_source(&source.id).await.unwrap().is_none(), "Restore invalidation reloads the file");
    }

    struct UnusedExecutor;

    impl SourceExecutor for UnusedExecutor {
        fn execute<'a>(&'a self, _request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async { Err("This test must not execute source rules".into()) })
        }
    }

    #[tokio::test]
    async fn source_tables_separate_metadata_from_rules_and_http_has_no_business_json() {
        let directory=tempfile::tempdir().unwrap();
        let service=ApplicationService::open_with_executor(directory.path(),Arc::new(UnusedExecutor)).await.unwrap();
        let mut source=SourceRecord{id:"source-test".into(),name:"Test".into(),group:None,enabled:true,
            source:json!({"bookSourceName":"Test","bookSourceUrl":"https://example.invalid","ruleSearch":{"bookList":"x".repeat(100000)},"lastUpdateTime":9007199254740993_u64})};
        assert!(service.write_sources(&[source.clone()]).await.unwrap());
        let connection=service.store.database().connect().unwrap();
        let metadata:String=connection.query_row("SELECT json_object('id',id,'name',name,'enabled',enabled) FROM sources",[],|r|r.get(0)).unwrap();
        assert!(metadata.len()<1024);
        let raw:String=connection.query_row("SELECT definition_json FROM sources WHERE id='source-test'",[],|r|r.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(),source.source);
        source.source["ruleSearch"]["bookList"]=json!("edited");
        assert!(!service.write_sources(&[source.clone()]).await.unwrap());
        let raw:String=connection.query_row("SELECT definition_json FROM sources",[],|r|r.get(0)).unwrap();assert!(raw.contains("edited"));
        for path in ["sources.json","source-list.json","sources/source-test.json"] {
            let reference=ResourceRef::new(format!("resource://{path}")).unwrap();
            assert_eq!(reqwest::get(service.server.url_for(&reference)).await.unwrap().status(),404);
        }
        assert!(service.write_sources(&[]).await.unwrap());
        assert_eq!(connection.query_row("SELECT count(*) FROM sources",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }

    #[tokio::test]
    async fn revisions_reject_reverted_edits_and_restores_without_a_persisted_file() {
        let directory = tempfile::tempdir().unwrap();
        let service = ApplicationService::open_with_executor(
            directory.path(),
            Arc::new(UnusedExecutor),
        )
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
        service.write_sources(&[source.clone()]).await.unwrap();
        let initial = service.source_revision(&source.id).await.unwrap();
        service.write_sources(&[source.clone()]).await.unwrap();
        assert_eq!(service.source_revision(&source.id).await.unwrap(), initial);

        let backup_directory = tempfile::tempdir().unwrap();
        let archive = backup_directory.path().join("backup.zip");
        crate::backup::create_backup(&service.store, &archive).await.unwrap();
        let mut edited = source.clone();
        edited.source["ruleSearch"] = json!({ "name": "changed" });
        service.write_sources(&[edited]).await.unwrap();
        service.write_sources(&[source.clone()]).await.unwrap();
        let reverted = service.source_revision(&source.id).await.unwrap();
        assert!(reverted > initial);

        service.write_sources(&[]).await.unwrap();
        service.write_sources(&[source.clone()]).await.unwrap();
        let reimported = service.source_revision(&source.id).await.unwrap();
        assert!(reimported > reverted);
        service.restore_backup(&archive).await.unwrap();
        assert!(service.source_revision(&source.id).await.unwrap() > reimported);
        assert!(!directory.path().join("source-revisions.json").exists());
        let file = std::fs::File::open(archive).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("source-revisions.json").is_err());

        drop(service);
        let reopened = ApplicationService::open_with_executor(
            directory.path(),
            Arc::new(UnusedExecutor),
        )
        .await
        .unwrap();
        assert_eq!(reopened.source_revision(&source.id).await.unwrap(), 0);
        assert_eq!(reopened.read_sources().await.unwrap()[0].source, source.source);
    }
}
