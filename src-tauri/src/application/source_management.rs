//! 书源定义的导入、登录、订阅和根目录持久化。

use super::*;

/// Validate only the structural identity of a Legado JSON definition.
/// Everything else, including unknown and nested keys, is retained verbatim.
fn validate_source_definition(
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
                    .unwrap_or("此书源使用网页登录。"),
            }));
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            return Ok(json!({
                "mode": "web",
                "message": result
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("此书源使用网页登录。"),
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

    pub async fn import_sources(&self, source_json: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let imported: Value = serde_json::from_str(source_json)
            .map_err(|error| format!("Invalid source JSON: {error}"))?;
        let sources = extract_sources(imported)?;
        let _lock = self.sources_lock.lock().await;
        let mut records = self.read_sources().await?;
        for (position, source) in sources.into_iter().enumerate() {
            let name = text_at(&source, &["bookSourceName", "name", "sourceName"])
                .unwrap_or_else(|| format!("Source {}", position + 1));
            let source_url =
                text_at(&source, &["bookSourceUrl", "url", "sourceUrl"]).unwrap_or_default();
            if source_url.is_empty() {
                return Err(format!("Source '{name}' is missing bookSourceUrl"));
            }
            let group = text_at(&source, &["bookSourceGroup", "group"]);
            let enabled = bool_at(&source, &["enabled", "bookSourceEnabled"]).unwrap_or(true);
            let id = format!("source-{:016x}", stable_hash(&source_url));
            let record = SourceRecord {
                id: id.clone(),
                name,
                group,
                enabled,
                source,
            };
            // An editor may change bookSourceUrl while deliberately keeping
            // the internal ID stable to preserve existing book references.
            // Reimporting the new URL must update that record, not duplicate it.
            if let Some(existing) = records.iter_mut().find(|existing| {
                existing.id == id
                    || text_at(&existing.source, &["bookSourceUrl", "url", "sourceUrl"])
                        .is_some_and(|existing_url| existing_url == source_url)
            }) {
                *existing = SourceRecord {
                    id: existing.id.clone(),
                    ..record
                };
            } else {
                records.push(record);
            }
        }
        self.write_sources(&records).await?;
        Ok(json!({ "sources": metadata(&records) }))
    }

    pub async fn import_sources_from_url(&self, input: &str) -> Result<Value, String> {
        let source_json = download_json_url(
            input,
            MAX_SOURCE_IMPORT_BODY_BYTES,
            Duration::from_secs(30),
            "书源 JSON",
        )
        .await?;
        self.import_sources(&source_json).await
    }

    /// Fetch a TXT TOC rule JSON file before atomic import.
    pub async fn fetch_txt_toc_rule_json(&self, input: &str) -> Result<String, String> {
        download_json_url(
            input,
            crate::local_books::txt_toc_rules::MAX_IMPORT_BODY_BYTES,
            Duration::from_secs(20),
            "TXT 目录规则 JSON",
        )
        .await
    }

    pub async fn import_replacement_rules_from_url(&self, input: &str) -> Result<Value, String> {
        let rules_json = download_json_url(
            input,
            MAX_REPLACEMENT_RULE_IMPORT_BODY_BYTES,
            Duration::from_secs(20),
            "显示替换规则 JSON",
        )
        .await?;
        let _operation = self.operation_read().await;
        let (resource, imported_count) =
            crate::reading_tools::import_replacement_rules_json(self.resource_store(), &rules_json)
                .await?;
        Ok(json!({
            "resource": self.resource_descriptor(&resource),
            "importedCount": imported_count,
        }))
    }

    pub async fn remove_sources(&self, source_ids: &[String]) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let _lock = self.sources_lock.lock().await;
        let remove = source_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let mut records = self.read_sources().await?;
        let rss_source_ids = records
            .iter()
            .filter(|source| {
                remove.contains(source.id.as_str()) && crate::rss::is_rss_source(&source.source)
            })
            .map(|source| source.id.clone())
            .collect::<Vec<_>>();
        records.retain(|source| !remove.contains(source.id.as_str()));
        self.write_sources(&records).await?;
        let mut result = json!({ "sources": metadata(&records) });
        let mut rss_state = None;
        let mut cleanup_pending = false;
        for source_id in rss_source_ids {
            match crate::rss::remove_subscription_data(self, &source_id).await {
                Ok(resource) => rss_state = Some(resource),
                Err(_) => cleanup_pending = true,
            }
        }
        if cleanup_pending {
            result["cleanupPending"] = json!(true);
            result["cleanupWarning"] = json!("书源已移除，但部分 RSS 缓存未能清理。");
            if rss_state.is_none() {
                rss_state = crate::rss::rss_state_resource(&self.store).await.ok();
            }
        }
        if let Some(resource) = rss_state {
            result["rssState"] = self.resource_descriptor(&resource);
        }
        Ok(result)
    }

    /// Publish the complete definitions as a browser-readable resource. The
    /// source JSON stays a string inside the resource so JavaScript does not
    /// round 64-bit numeric fields while parsing the outer document.
    pub(super) async fn source_definitions_resource(&self) -> Result<ResourceRef, String> {
        let _lock = self.sources_lock.lock().await;
        let records = self.read_sources().await?;
        self.publish_source_definitions(&records).await
    }

    async fn publish_source_definitions(
        &self,
        records: &[SourceRecord],
    ) -> Result<ResourceRef, String> {
        let mut entries = Vec::with_capacity(records.len());
        for (record, metadata) in records.iter().zip(metadata(records)) {
            let mut entry = serde_json::to_value(metadata)
                .map_err(|error| format!("Cannot encode source metadata: {error}"))?;
            let definition_json = serde_json::to_string_pretty(&record.source)
                .map_err(|error| format!("Cannot encode source definition: {error}"))?;
            entry["definitionJson"] = Value::String(definition_json);
            entries.push(entry);
        }
        let document = json!({ "schemaVersion": 1, "sources": entries });
        let document_bytes = serde_json::to_vec(&document)
            .map_err(|error| format!("Cannot encode source resource: {error}"))?;
        let content_hash = format!("{:x}", Sha256::digest(&document_bytes));
        let resource = self
            .store
            .source_definitions_ref(&content_hash)
            .map_err(|error| error.to_string())?;
        let exists = self
            .store
            .regular_file_exists(&resource)
            .await
            .map_err(|error| error.to_string())?;
        if !exists {
            self.store
                .write_json_ref(&resource, &document)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(resource)
    }

    /// Atomically replace one source definition. The internal source ID remains
    /// stable even if bookSourceUrl changes, preserving library references.
    pub async fn save_source_definition(
        &self,
        source_id: &str,
        definition_json: String,
    ) -> Result<Value, String> {
        validate_id(source_id, "sourceId")?;
        if definition_json.len() > MAX_SOURCE_IMPORT_BODY_BYTES {
            return Err("Source definition exceeds the JSON import size limit".into());
        }
        // JSON travels through IPC as text to preserve 64-bit source fields.
        let definition: Value = serde_json::from_str(&definition_json)
            .map_err(|error| format!("Invalid source JSON: {error}"))?;
        let (name, group, enabled, url) = validate_source_definition(&definition)?;
        let _operation = self.operation_read().await;
        let _lock = self.sources_lock.lock().await;
        let mut records = self.read_sources().await?;
        if records.iter().any(|record| {
            record.id != source_id
                && text_at(&record.source, &["bookSourceUrl", "url", "sourceUrl"])
                    .is_some_and(|value| value == url)
        }) {
            return Err("Another imported source already uses that bookSourceUrl".into());
        }
        let record = records
            .iter_mut()
            .find(|record| record.id == source_id)
            .ok_or_else(|| format!("Unknown source '{source_id}'"))?;
        record.name = name;
        record.group = group;
        record.enabled = enabled;
        record.source = definition;
        self.write_sources(&records).await?;
        Ok(json!({ "sources": metadata(&records) }))
    }

    pub(super) async fn find_source(&self, id: &str) -> Result<SourceRecord, String> {
        self.read_sources()
            .await?
            .into_iter()
            .find(|source| source.id == id)
            .ok_or_else(|| format!("Book source '{id}' is no longer imported"))
    }

    pub(super) async fn read_sources(&self) -> Result<Vec<SourceRecord>, String> {
        let reference = ResourceRef::new("resource://sources.json")
            .map_err(|error| error.to_string())?;
        if !self
            .store
            .regular_file_exists(&reference)
            .await
            .map_err(|error| error.to_string())?
        {
            return Ok(Vec::new());
        }
        let document = self
            .store
            .read_json_ref(&reference)
            .await
            .map_err(|error| error.to_string())?;
        let records: Vec<SourceRecord> = serde_json::from_value(document)
            .map_err(|error| format!("Cannot decode saved source definitions: {error}"))?;
        validate_source_records(&records)?;
        Ok(records)
    }

    pub(super) async fn source_revision(&self, source_id: &str) -> Result<u64, String> {
        Ok(self
            .read_source_revisions()
            .await?
            .get(source_id)
            .copied()
            .unwrap_or_default())
    }

    pub(super) async fn write_sources(&self, sources: &[SourceRecord]) -> Result<(), String> {
        let previous = self.read_sources().await?;
        let previous_by_id = previous
            .iter()
            .map(|record| {
                serde_json::to_value(record)
                    .map(|value| (record.id.as_str(), value))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let next_by_id = sources
            .iter()
            .map(|record| {
                serde_json::to_value(record)
                    .map(|value| (record.id.as_str(), value))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let mut revisions = self.read_source_revisions().await?;
        let changed = previous_by_id
            .keys()
            .chain(next_by_id.keys())
            .copied()
            .collect::<HashSet<_>>();
        for source_id in changed {
            if previous_by_id.get(source_id) != next_by_id.get(source_id) {
                let revision = revisions.entry(source_id.to_owned()).or_default();
                *revision = revision.saturating_add(1);
            }
        }
        self.write_private_json(
            Path::new("source-revisions.json"),
            &json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "revisions": revisions }),
        )
        .await?;
        let reference = ResourceRef::new("resource://sources.json")
            .map_err(|error| error.to_string())?;
        self.store
            .write_json_ref(&reference, &json!(sources))
            .await
            .map_err(|error| error.to_string())?;
        self.publish_source_definitions(sources).await?;
        Ok(())
    }

    async fn read_source_revisions(&self) -> Result<HashMap<String, u64>, String> {
        let path = self.private_root.join("source-revisions.json");
        let Some(bytes) = read_private_state_file(&path, "source revisions").await? else {
            return Ok(HashMap::new());
        };
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RevisionsDocument {
            schema_version: u32,
            revisions: HashMap<String, u64>,
        }
        let document: RevisionsDocument = match serde_json::from_slice::<RevisionsDocument>(&bytes)
        {
            Ok(document) if document.schema_version == CURRENT_SCHEMA_VERSION => document,
            _ => return Ok(HashMap::new()),
        };
        Ok(document.revisions)
    }
}

async fn read_private_state_file(path: &Path, label: &str) -> Result<Option<Vec<u8>>, String> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Cannot read {label}: symbolic links are not allowed"
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!("Cannot read {label}: not a file")),
        Ok(_) => tokio::fs::read(path)
            .await
            .map(Some)
            .map_err(|error| format!("Cannot read {label}: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot inspect {label}: {error}")),
    }
}
