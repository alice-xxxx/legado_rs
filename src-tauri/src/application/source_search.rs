//! 书源检索结果、引擎调用和搜索结果资源准备。

use super::*;

impl ApplicationService {
    /// One-shot source-debug search. Discovery search uses `start_search`
    /// and immutable snapshots instead of this direct response path.
    pub async fn search_books(
        &self,
        source_ids: &[String],
        keyword: &str,
        page: u32,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return Err("Search keyword cannot be empty".into());
        }
        let page = page.max(1);
        let selected = {
            let _sources = self.sources_lock.lock().await;
            let all_sources = self.read_sources().await?;
            let mut selected = Vec::new();
            for source in all_sources.into_iter().filter(|source| {
                source.enabled
                    && !crate::source_metadata::is_rss_source_metadata(&source.source)
                    && (source_ids.is_empty() || source_ids.contains(&source.id))
            }) {
                let revision = self.source_revision(&source.id).await?;
                selected.push((source, revision));
            }
            selected
        };
        if selected.is_empty() {
            return Err("No enabled book sources are selected".into());
        }
        let search_id = format!("debug-search-{}", uuid::Uuid::new_v4().simple());

        let mut source_results = Vec::new();
        let mut errors = Vec::new();
        for (source, source_revision) in &selected {
            let response = self
                .executor
                .execute(engine_request(
                    "search",
                    &source.source,
                    Some(keyword.to_owned()),
                    Some(page as i32),
                    None,
                    None,
                    None,
                ))
                .await;
            match response {
                Ok(value) => {
                    let books = value
                        .get("books")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    for book in books {
                        source_results.push((source.clone(), book, *source_revision));
                    }
                }
                Err(error) => errors.push(
                    json!({ "sourceId": source.id, "sourceName": source.name, "message": error }),
                ),
            }
        }
        let _sources = self.sources_lock.lock().await;
        let current_sources = self.read_sources().await?;
        let mut publishable_results = Vec::with_capacity(source_results.len());
        for (source, book, source_revision) in source_results {
            let current_source = current_sources
                .iter()
                .find(|current| current.id == source.id);
            let current_revision = self.source_revision(&source.id).await?;
            if current_source.is_some_and(|current| {
                current.enabled
                    && current.source == source.source
                    && current_revision == source_revision
            }) {
                publishable_results.push((source, book, source_revision));
            } else {
                errors.push(json!({
                    "sourceId": source.id,
                    "sourceName": source.name,
                    "message": "Source changed while search was running; search again",
                }));
            }
        }
        self.store_processed_results_with_search_id(
            &search_id,
            keyword,
            page,
            publishable_results,
            errors,
        )
        .await
    }

    /// Search other enabled novel sources for a replacement for an existing
    /// book. The private context binds every result to this book's current
    /// source and catalog snapshot; ordinary search results cannot be used by
    /// `change_book_source`.
    pub async fn search_book_source_candidates(
        &self,
        book_id: &str,
        source_ids: &[String],
        keyword: Option<&str>,
        page: u32,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        self.search_book_source_candidates_unlocked(book_id, source_ids, keyword, page)
            .await
    }

    pub(super) async fn search_book_source_candidates_unlocked(
        &self,
        book_id: &str,
        source_ids: &[String],
        keyword: Option<&str>,
        page: u32,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;

        let (selected, context, effective_keyword, use_cached_candidates) = {
            // Keep the same lock order used by catalog commits. No source or
            // book lock is held while the executor searches the network.
            let _sources = self.sources_lock.lock().await;
            let records = self.read_sources().await?;
            let _book = self.book_lock(book_id).await;
            let private_path = Path::new("books").join(format!("{book_id}.json"));
            let mut private = self
                .read_private_json(&private_path)
                .await
                .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
            let book_ref = self
                .store
                .book_ref(book_id)
                .map_err(|error| error.to_string())?;
            let book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
            if private
                .get("bookInstanceId")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                private["bookInstanceId"] = json!(uuid::Uuid::new_v4().simple().to_string());
                self.write_private_json(&private_path, &private).await?;
            }
            let original_source_id = private["sourceId"]
                .as_str()
                .ok_or_else(|| "Private book is missing its source ID".to_owned())?
                .to_owned();
            if !can_change_source_from_private(&private) {
                return Err("This book is not linked to a replaceable online source".into());
            }
            let canonical_book = private
                .get("book")
                .filter(|value| value.is_object())
                .ok_or_else(|| "Private book is missing canonical engine metadata".to_owned())?;
            let canonical_title = text_at(canonical_book, &["name", "title"])
                .unwrap_or_default()
                .trim()
                .to_owned();
            let canonical_author = text_at(canonical_book, &["author"])
                .unwrap_or_default()
                .trim()
                .to_owned();
            let original_source = records
                .iter()
                .find(|source| source.id == original_source_id);
            let original_source_definition = original_source
                .map(|source| source.source.clone())
                .unwrap_or(Value::Null);
            let override_keyword = keyword
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            let effective_keyword = override_keyword
                .clone()
                .unwrap_or_else(|| canonical_title.clone());
            if normalize_identity(&effective_keyword).is_empty() {
                return Err("A title or search keyword is required to find another source".into());
            }
            let fingerprint = book_source_fingerprint(
                &original_source_id,
                &original_source_definition,
                &private,
                &book,
            )?;
            let selected = records
                .iter()
                .filter(|source| {
                    source.enabled
                        && source.id != original_source_id
                        && !crate::source_metadata::is_rss_source_metadata(&source.source)
                        && (source_ids.is_empty() || source_ids.contains(&source.id))
                })
                .cloned()
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err("No other enabled novel sources are selected".into());
            }
            let now = now_ms();
            let context = json!({
                "schemaVersion": CURRENT_SCHEMA_VERSION,
                "targetBookId": book_id,
                "originalSourceId": original_source_id,
                "originalSourcePresent": original_source.is_some(),
                "originalSourceFingerprint": original_source
                    .map(|source| source_definition_fingerprint(&source.source))
                    .transpose()?,
                "originalSourceRevision": self.source_revision(&original_source_id).await?,
                "catalogFingerprint": fingerprint,
                "catalogGeneration": private.get("catalogGeneration").cloned().unwrap_or(Value::Null),
                "bookInstanceId": private["bookInstanceId"],
                "targetTitle": canonical_title,
                "targetAuthor": canonical_author,
                "createdAtMs": now,
                "expiresAtMs": now.saturating_add(BOOK_SOURCE_CANDIDATE_TTL_MS),
            });
            (selected, context, effective_keyword, override_keyword.is_none())
        };

        let search_id = format!("replace-{}", uuid::Uuid::new_v4().simple());
        let (reused_results, remaining_sources) = if use_cached_candidates {
            self.reuse_search_candidates(book_id, &search_id, &selected, &context)
                .await?
        } else {
            (Vec::new(), selected.clone())
        };
        let mut ordered_source_ids = Vec::new();
        let mut reused_source_ids = HashSet::new();
        for (_, cache) in &reused_results {
            if let Some(source_id) = cache.get("sourceId").and_then(Value::as_str) {
                if reused_source_ids.insert(source_id.to_owned()) {
                    ordered_source_ids.push(source_id.to_owned());
                }
            }
        }
        ordered_source_ids.extend(remaining_sources.iter().map(|source| source.id.clone()));
        let completed = reused_source_ids.len();
        let total = selected.len();
        let task_id = format!("task-{}", uuid::Uuid::new_v4().simple());
        let results = reused_results
            .iter()
            .map(|(result, _)| serde_json::to_value(result).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let initial_document = json!({
            "schemaVersion": CURRENT_SCHEMA_VERSION,
            "searchId": search_id,
            "taskId": task_id,
            "keyword": effective_keyword,
            "page": page.max(1),
            "results": results,
            "errors": [],
            "status": "queued",
            "completedSources": completed,
            "totalSources": total,
            "complete": false,
        });
        let now = now_ms();
        let task = AppTask {
            id: task_id.clone(),
            kind: "bookSourceCandidates".into(),
            status: "queued".into(),
            book_id: Some(book_id.to_owned()),
            chapter_ids: None,
            source_ids: Some(ordered_source_ids),
            keyword: Some(effective_keyword),
            page: page.max(1),
            from_index: 0,
            total,
            completed,
            check_only: false,
            search_id: Some(search_id.clone()),
            result: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let receiver = self
            .create_candidate_search_task(task, initial_document, context, reused_results)
            .await?;
        self.spawn_task_worker(task_id.clone(), receiver);
        Ok(json!({
            "bookId": book_id,
            "taskId": task_id,
            "task": self.task_summary_by_id(&task_id).await?,
            "resource": self.task_resource_descriptor()?,
        }))
    }

    async fn reuse_search_candidates(
        &self,
        book_id: &str,
        replacement_search_id: &str,
        selected: &[SourceRecord],
        context: &Value,
    ) -> Result<(Vec<(SearchResultSource, Value)>, Vec<SourceRecord>), String> {
        let candidates = {
            let registry = self.tasks.lock().await;
            let Some(root_result_id) = registry.search_book_group_roots.get(book_id) else {
                return Ok((Vec::new(), selected.to_vec()));
            };
            let Some(root_cache) = registry.search_result_documents.get(root_result_id) else {
                return Ok((Vec::new(), selected.to_vec()));
            };
            let Some(search_id) = root_cache.get("searchId").and_then(Value::as_str) else {
                return Ok((Vec::new(), selected.to_vec()));
            };
            let Some(document) = registry.search_documents.get(search_id) else {
                return Ok((Vec::new(), selected.to_vec()));
            };
            let Some(group) = document
                .get("results")
                .and_then(Value::as_array)
                .and_then(|results| results.iter().find(|result| {
                    result["resultId"].as_str() == Some(root_result_id.as_str())
                        || result["sources"].as_array().is_some_and(|sources| {
                            sources.iter().any(|source| {
                                source["resultId"].as_str() == Some(root_result_id.as_str())
                            })
                        })
                }))
            else {
                return Ok((Vec::new(), selected.to_vec()));
            };
            let variants = group
                .get("sources")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_else(|| vec![group.clone()]);
            variants
                .iter()
                .filter_map(|variant| {
                    let result_id = variant.get("resultId")?.as_str()?;
                    registry
                        .search_result_documents
                        .get(result_id)
                        .cloned()
                        .map(|cache| (variant.clone(), cache))
                })
                .collect::<Vec<_>>()
        };

        let _sources = self.sources_lock.lock().await;
        let current_sources = self.read_sources().await?;
        let mut reused = Vec::new();
        let mut remaining = Vec::new();
        for selected_source in selected {
            let Some(current_source) = current_sources.iter().find(|source| {
                source.id == selected_source.id
                    && source.enabled
                    && source.source == selected_source.source
            }) else {
                remaining.push(selected_source.clone());
                continue;
            };
            let source_revision = self.source_revision(&current_source.id).await?;
            let current_fingerprint = source_definition_fingerprint(&current_source.source)?;
            let reusable = candidates.iter().filter(|(variant, cache)| {
                variant.get("sourceId").and_then(Value::as_str) == Some(current_source.id.as_str())
                    && cache.get("sourceId").and_then(Value::as_str) == Some(current_source.id.as_str())
                    && cache.get("sourceRevision").and_then(Value::as_u64) == Some(source_revision)
                    && cache.get("sourceDefinitionFingerprint").and_then(Value::as_str)
                        == Some(current_fingerprint.as_str())
                    && cache.get("book").is_some_and(|book| candidate_identity_match(context, book).is_some())
            }).collect::<Vec<_>>();
            if reusable.is_empty() {
                remaining.push(selected_source.clone());
                continue;
            }
            let fingerprint = source_definition_fingerprint(&current_source.source)?;
            let mut reused_for_source = false;
            for (_, cached) in reusable {
                let raw_book = cached["book"].clone();
                let Some(requires_confirmation) = candidate_identity_match(context, &raw_book) else {
                    continue;
                };
                let result_id = format!("result-{}", uuid::Uuid::new_v4().simple());
                let mut leaf = project_search_result(&result_id, current_source, &raw_book);
                leaf.requires_identity_confirmation = Some(requires_confirmation);
                let candidate_context = json!({
                    "targetBookId": context["targetBookId"],
                    "originalSourceId": context["originalSourceId"],
                    "originalSourcePresent": context["originalSourcePresent"],
                    "originalSourceFingerprint": context["originalSourceFingerprint"],
                    "originalSourceRevision": context["originalSourceRevision"],
                    "catalogFingerprint": context["catalogFingerprint"],
                    "catalogGeneration": context["catalogGeneration"],
                    "bookInstanceId": context["bookInstanceId"],
                    "createdAtMs": context["createdAtMs"],
                    "expiresAtMs": context["expiresAtMs"],
                    "searchId": replacement_search_id,
                    "candidateSourceFingerprint": fingerprint,
                    "requiresIdentityConfirmation": requires_confirmation,
                });
                reused.push((leaf, json!({
                    "searchId": replacement_search_id,
                    "originSearchId": cached["searchId"],
                    "sourceId": current_source.id,
                    "sourceRevision": source_revision,
                    "sourceDefinitionFingerprint": fingerprint,
                    "book": raw_book,
                    "replacementContext": candidate_context,
                })));
                reused_for_source = true;
            }
            if !reused_for_source {
                remaining.push(selected_source.clone());
            }
        }
        Ok((reused, remaining))
    }
    pub(crate) async fn store_processed_results(
        &self,
        keyword: &str,
        page: u32,
        source_results: Vec<(SourceRecord, Value)>,
        errors: Vec<Value>,
    ) -> Result<Value, String> {
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let versioned_results = {
            let current_sources = self.read_sources().await?;
            let mut versioned = Vec::with_capacity(source_results.len());
            for (source, book) in source_results {
                let current_source = current_sources
                    .iter()
                    .find(|current| current.id == source.id)
                    .ok_or_else(|| format!("Source '{}' was removed during search", source.id))?;
                if current_source.source != source.source {
                    return Err("Source changed during search; run the search again".into());
                }
                let revision = self.source_revision(&source.id).await?;
                versioned.push((source, book, revision));
            }
            versioned
        };
        self.store_processed_results_with_search_id(
            &search_id,
            keyword,
            page,
            versioned_results,
            errors,
        )
        .await
    }

    pub(crate) async fn store_processed_results_with_search_id(
        &self,
        search_id: &str,
        keyword: &str,
        page: u32,
        source_results: Vec<(SourceRecord, Value, u64)>,
        errors: Vec<Value>,
    ) -> Result<Value, String> {
        let mut public_results = Vec::with_capacity(source_results.len());
        let mut private_results = Vec::with_capacity(source_results.len());
        let mut result_indexes = SearchResultGroupIndexes::default();
        for (source, book, source_revision) in source_results {
            let result_id = format!("result-{}", uuid::Uuid::new_v4().simple());
            let leaf = project_search_result(&result_id, &source, &book);
            let cache = json!({
                "searchId": search_id,
                "sourceId": source.id,
                "sourceRevision": source_revision,
                "sourceDefinitionFingerprint": source_definition_fingerprint(&source.source)?,
                "book": book
            });
            if merge_search_result(&mut public_results, &mut result_indexes, leaf)? {
                private_results.push((result_id, cache));
            }
        }
        self.tasks.lock().await.search_result_documents.extend(private_results);
        let document = json!({
            "schemaVersion": CURRENT_SCHEMA_VERSION,
            "searchId": search_id,
            "keyword": keyword,
            "page": page.max(1),
            "results": public_results,
            "errors": errors,
            "complete": true,
        });
        let (resource, _) = self.write_search_snapshot(&document).await?;
        let descriptor = self.resource_descriptor(&resource);
        let count = document["results"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default();
        let errors = document["errors"].clone();
        Ok(json!({ "resource": descriptor, "bookCount": count, "errors": errors }))
    }

    pub(super) async fn read_search_task_state(&self, task: &AppTask) -> Result<Value, String> {
        let search_id = task
            .search_id
            .as_deref()
            .ok_or_else(|| "Search task has no search resource".to_owned())?;
        self.read_search_document(search_id).await
    }

    pub(in crate::application) async fn read_search_document(
        &self,
        search_id: &str,
    ) -> Result<Value, String> {
        self.tasks
            .lock()
            .await
            .search_documents
            .get(search_id)
            .cloned()
            .ok_or_else(|| "In-memory search state is unavailable; start the search again".into())
    }

    pub(in crate::application) async fn read_search_result_cache(
        &self,
        result_id: &str,
    ) -> Result<Value, String> {
        self.tasks
            .lock()
            .await
            .search_result_documents
            .get(result_id)
            .cloned()
            .ok_or_else(|| "Search result is unavailable; search again".to_owned())
    }

    pub(super) async fn read_search_replacement_context(
        &self,
        search_id: &str,
    ) -> Result<Value, String> {
        self.tasks
            .lock()
            .await
            .search_replacement_contexts
            .get(search_id)
            .cloned()
            .ok_or_else(|| "Replacement search context is unavailable; search again".to_owned())
    }

    pub(super) async fn publish_search_task_state(
        &self,
        task: &AppTask,
        status: &str,
        terminal_error: Option<&str>,
    ) -> Result<(), String> {
        if task.kind != "search" {
            return Ok(());
        }
        let document = {
            let mut registry = self.tasks.lock().await;
            if registry.active_search_task_id.as_deref() != Some(&task.id) {
                return Ok(());
            }
            let document = registry
                .search_documents
                .get_mut(task.search_id.as_deref().unwrap_or_default())
                .ok_or_else(|| "In-memory search state is unavailable; start the search again".to_owned())?;
            document["status"] = json!(status);
            document["completedSources"] = json!(task.completed);
            document["totalSources"] = json!(task.total);
            document["complete"] = json!(matches!(status, "completed" | "failed" | "cancelled"));
            if let Some(object) = document.as_object_mut() {
                if status == "cancelled" { object.insert("cancelled".into(), json!(true)); }
                else { object.remove("cancelled"); }
                if let Some(error) = terminal_error { object.insert("error".into(), json!(error)); }
                else { object.remove("error"); }
            }
            document.clone()
        };
        self.publish_search_snapshot_document(&task.id, &document).await
    }

    pub(super) async fn append_and_publish_search_snapshot(
        &self,
        task: &AppTask,
        results: Vec<(SearchResultSource, Value)>,
        errors: Vec<Value>,
        publish: bool,
    ) -> Result<(), String> {
        let document = {
            let mut registry = self.tasks.lock().await;
            if registry.active_search_task_id.as_deref() != Some(&task.id) {
                return Ok(());
            }
            let search_id = task.search_id.as_deref()
                .ok_or_else(|| "Search task has no search resource".to_owned())?;
            let TaskRegistry {
                search_documents,
                search_result_documents,
                ..
            } = &mut *registry;
            let document = search_documents
                .get_mut(search_id)
                .ok_or_else(|| "In-memory search state is unavailable; start the search again".to_owned())?;
            if document.get("completedSources").and_then(Value::as_u64) != Some(task.completed as u64) {
                return Err("Search document is ahead of its task checkpoint".into());
            }
            let grouped_results = document.get_mut("results").and_then(Value::as_array_mut)
                .ok_or_else(|| "Search state has no results array".to_owned())?;
            let mut result_indexes = search_result_group_indexes(grouped_results);
            for (result, cache) in results {
                let result_id = result.result_id.clone();
                if merge_search_result(grouped_results, &mut result_indexes, result)? {
                    search_result_documents.insert(result_id, cache);
                }
            }
            document.get_mut("errors").and_then(Value::as_array_mut)
                .ok_or_else(|| "Search state has no errors array".to_owned())?
                .extend(errors);
            document["status"] = json!("running");
            document["completedSources"] = json!(task.completed.saturating_add(1));
            document["totalSources"] = json!(task.total);
            document["complete"] = json!(false);
            publish.then(|| document.clone())
        };
        if let Some(document) = document {
            self.publish_search_snapshot_document(&task.id, &document).await
        } else {
            Ok(())
        }
    }

    async fn publish_search_snapshot_document(
        &self,
        task_id: &str,
        document: &Value,
    ) -> Result<(), String> {
        if !self.search_task_is_publisher(task_id).await {
            return Ok(());
        }
        let (resource, _) = self.write_search_snapshot(document).await?;
        let descriptor = self.resource_descriptor(&resource);

        let registry = self.tasks.lock().await;
        if registry.active_search_task_id.as_deref() != Some(task_id) {
            return Ok(());
        }
        if let Ok(notifier) = self.search_snapshot_notifier.read() {
            if let Some(notifier) = notifier.as_ref() {
                notifier(descriptor);
            }
        }
        Ok(())
    }

    pub(super) async fn write_search_snapshot(
        &self,
        document: &Value,
    ) -> Result<(ResourceRef, Value), String> {
        let snapshot_id = format!("search-snapshot-{}", uuid::Uuid::new_v4().simple());
        let resource = self.store.search_ref(&snapshot_id).map_err(|error| error.to_string())?;
        self.store.write_json_ref(&resource, document).await.map_err(|error| error.to_string())?;
        let stable_descriptor = json!({
            "resourceId": resource.as_str(),
            "src": resource.as_str(),
            "contentType": content_type(resource.as_str()),
            "format": resource_format(resource.as_str()),
        });
        Ok((resource, stable_descriptor))
    }

    pub(super) async fn append_candidate_search_results(
        &self,
        task: &AppTask,
        results: Vec<(SearchResultSource, Value)>,
        errors: Vec<Value>,
    ) -> Result<(), String> {
        let search_id = task.search_id.as_deref()
            .ok_or_else(|| "Candidate search has no search resource".to_owned())?;
        let mut registry = self.tasks.lock().await;
        if !registry.search_replacement_contexts.contains_key(search_id) {
            return Err("Replacement search is no longer active; search again".into());
        }
        let TaskRegistry {
            search_documents,
            search_result_documents,
            ..
        } = &mut *registry;
        let document = search_documents.get_mut(search_id)
            .ok_or_else(|| "Replacement search is no longer active; search again".to_owned())?;
        if document.get("completedSources").and_then(Value::as_u64) != Some(task.completed as u64) {
            return Err("Candidate search document is ahead of its task checkpoint".into());
        }
        let target = document.get_mut("results").and_then(Value::as_array_mut)
            .ok_or_else(|| "Candidate search has no results array".to_owned())?;
        for (result, cache) in results {
            let result_id = result.result_id.clone();
            search_result_documents.insert(result_id, cache);
            target.push(serde_json::to_value(result)
                .map_err(|error| format!("Cannot encode candidate result: {error}"))?);
        }
        document.get_mut("errors").and_then(Value::as_array_mut)
            .ok_or_else(|| "Candidate search has no errors array".to_owned())?
            .extend(errors);
        document["status"] = json!("running");
        document["completedSources"] = json!(task.completed.saturating_add(1));
        document["totalSources"] = json!(task.total);
        Ok(())
    }

    pub(crate) async fn source_record(&self, source_id: &str) -> Result<SourceRecord, String> {
        self.find_source(source_id).await
    }

    /// Hold source metadata stable while an RSS request publishes its
    /// category/article/state resources. Removing or changing a subscription
    /// uses the same mutex, so a delayed response cannot recreate its state.
    pub(crate) async fn lock_rss_source_snapshot(
        &self,
        expected: &SourceRecord,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        let guard = self.sources_lock.clone().lock_owned().await;
        let current = self
            .read_sources()
            .await?
            .into_iter()
            .find(|source| source.id == expected.id)
            .ok_or_else(|| "RSS source was removed while the request was running".to_owned())?;
        if !current.enabled
            || current.source != expected.source
            || !crate::source_metadata::is_rss_source_metadata(&current.source)
        {
            return Err("RSS source changed while the request was running".to_owned());
        }
        Ok(guard)
    }

    pub(crate) async fn execute_source_operation(
        &self,
        source_id: &str,
        operation: &str,
        keyword: Option<String>,
        page: Option<u32>,
        book: Option<Value>,
        chapter: Option<Value>,
        next_chapter_url: Option<String>,
    ) -> Result<Value, String> {
        let source = self.find_source(source_id).await?;
        self.executor
            .execute(engine_request(
                operation,
                &source.source,
                keyword,
                page.map(|page| page as i32),
                book,
                chapter,
                next_chapter_url,
            ))
            .await
    }

    /// Inspect one search result through the real source engine without
    /// creating a shelf book or a private book identity. Only processed book
    /// metadata, chapter titles, and a sanitized chapter resource can leave
    /// this method.
    pub async fn debug_search_result(
        &self,
        result_id: Option<&str>,
        chapter_index: Option<usize>,
        cleanup_resource_id: Option<&str>,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        if let Some(resource_id) = cleanup_resource_id {
            self.clear_source_debug_preview(resource_id).await?;
            return Ok(json!({ "ok": true, "cleared": true }));
        }
        let Some(result_id) = result_id else {
            return Ok(
                json!({ "ok": false, "stage": "result", "error": "Select a search result first" }),
            );
        };
        if let Err(error) = validate_id(result_id, "resultId") {
            return Ok(json!({ "ok": false, "stage": "result", "error": error }));
        }
        let cache = match self.read_search_result_cache(result_id).await {
            Ok(cache) => cache,
            Err(error) => {
                return Ok(json!({ "ok": false, "stage": "result", "error": error }));
            }
        };
        let source_id = match cache.get("sourceId").and_then(Value::as_str) {
            Some(source_id) => source_id.to_owned(),
            None => {
                return Ok(
                    json!({ "ok": false, "stage": "result", "error": "Search result has no source ID" }),
                );
            }
        };
        let expected_revision = match cache.get("sourceRevision").and_then(Value::as_u64) {
            Some(revision) => revision,
            None => {
                return Ok(
                    json!({ "ok": false, "stage": "result", "error": "Search result is stale; search again" }),
                );
            }
        };
        let expected_fingerprint = match cache
            .get("sourceDefinitionFingerprint")
            .and_then(Value::as_str)
        {
            Some(fingerprint) if !fingerprint.is_empty() => fingerprint.to_owned(),
            _ => {
                return Ok(
                    json!({ "ok": false, "stage": "result", "error": "Search result is stale; search again" }),
                );
            }
        };
        let raw_book = match cache.get("book").filter(|book| book.is_object()) {
            Some(book) => book.clone(),
            None => {
                return Ok(
                    json!({ "ok": false, "stage": "result", "error": "Search result has invalid book data" }),
                );
            }
        };
        let (source, source_revision) = {
            let _sources = self.sources_lock.lock().await;
            let source = match self.read_sources().await {
                Ok(sources) => sources.into_iter().find(|source| source.id == source_id),
                Err(error) => return Ok(json!({ "ok": false, "stage": "source", "error": error })),
            };
            let Some(source) = source else {
                return Ok(
                    json!({ "ok": false, "stage": "source", "error": "Search result source was removed; search again" }),
                );
            };
            if !source.enabled {
                return Ok(
                    json!({ "ok": false, "stage": "source", "error": "Search result source is disabled; enable it and search again" }),
                );
            }
            let revision = match self.source_revision(&source_id).await {
                Ok(revision) => revision,
                Err(error) => return Ok(json!({ "ok": false, "stage": "source", "error": error })),
            };
            if revision != expected_revision
                || source_definition_fingerprint(&source.source)
                    .ok()
                    .as_deref()
                    != Some(expected_fingerprint.as_str())
            {
                return Ok(
                    json!({ "ok": false, "stage": "source", "error": "Search result source changed; search again" }),
                );
            }
            (source, revision)
        };

        let info = match self
            .executor
            .execute(engine_request(
                if crate::rss::is_legacy_rss_source(&source.source) {
                    "rssBookInfo"
                } else {
                    "bookInfo"
                },
                &source.source,
                None,
                None,
                Some(raw_book.clone()),
                None,
                None,
            ))
            .await
        {
            Ok(info) => info,
            Err(error) => return Ok(json!({ "ok": false, "stage": "detail", "error": error })),
        };
        let engine_book = if info.is_object() && !info.as_object().is_some_and(Map::is_empty) {
            info
        } else {
            raw_book
        };
        let raw_chapters = match self
            .executor
            .execute(engine_request(
                if crate::rss::is_legacy_rss_source(&source.source) {
                    "rssChapters"
                } else {
                    "chapters"
                },
                &source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await
        {
            Ok(Value::Array(chapters)) => chapters,
            Ok(_) => {
                return Ok(
                    json!({ "ok": false, "stage": "catalog", "error": "Source engine returned an invalid chapter catalog" }),
                );
            }
            Err(error) => return Ok(json!({ "ok": false, "stage": "catalog", "error": error })),
        };
        if raw_chapters.len() > 10_000 {
            return Ok(
                json!({ "ok": false, "stage": "catalog", "error": "Source chapter catalog exceeds the 10,000 chapter debug limit" }),
            );
        }
        if let Err(error) = crate::catalog::reconcile_catalog(
            "debug-source",
            &[],
            &[],
            &raw_chapters,
            &ProgressSummary::default(),
            &HashSet::new(),
            now_ms(),
        ) {
            return Ok(json!({ "ok": false, "stage": "catalog", "error": error }));
        }
        {
            let _sources = self.sources_lock.lock().await;
            let current_source = match self.read_sources().await {
                Ok(sources) => sources
                    .into_iter()
                    .find(|candidate| candidate.id == source_id),
                Err(error) => return Ok(json!({ "ok": false, "stage": "source", "error": error })),
            };
            let source_is_current = match current_source.as_ref() {
                Some(current) if current.enabled && current.source == source.source => {
                    match self.source_revision(&source_id).await {
                        Ok(revision) => revision == source_revision,
                        Err(error) => {
                            return Ok(json!({ "ok": false, "stage": "source", "error": error }));
                        }
                    }
                }
                _ => false,
            };
            if !source_is_current {
                return Ok(
                    json!({ "ok": false, "stage": "source", "error": "Search result source changed during book lookup; search again" }),
                );
            }
        }

        let metadata = crate::book_metadata::project_book_metadata(&engine_book, &source);
        let chapters = raw_chapters
            .iter()
            .enumerate()
            .map(|(index, chapter)| {
                let title = text_at(chapter, &["title", "chapterName", "name"])
                    .unwrap_or_else(|| format!("Chapter {}", index + 1));
                let title = title.chars().take(512).collect::<String>();
                json!({ "index": index, "title": title })
            })
            .collect::<Vec<_>>();

        // Each preview receives its own constrained UUID resource. The caller
        // removes exactly that resource on close or when a late request returns.
        let mut chapter_resource = None;
        if let Some(index) = chapter_index {
            let Some(chapter) = raw_chapters.get(index).cloned() else {
                return Ok(
                    json!({ "ok": false, "stage": "chapter", "error": "The selected chapter is no longer in this catalog" }),
                );
            };
            let content_base_url = text_at(&chapter, &["url", "chapterUrl"])
                .or_else(|| text_at(&engine_book, &["bookUrl", "url"]));
            let next_chapter_url = raw_chapters
                .get(index + 1)
                .and_then(|next| text_at(next, &["url", "chapterUrl"]));
            let content = match self
                .executor
                .execute(engine_request(
                    if crate::rss::is_legacy_rss_source(&source.source) {
                        "rssContent"
                    } else {
                        "content"
                    },
                    &source.source,
                    None,
                    None,
                    Some(engine_book.clone()),
                    Some(chapter),
                    next_chapter_url,
                ))
                .await
            {
                Ok(Value::String(content)) if !content.trim().is_empty() => content,
                Ok(Value::String(_)) => {
                    return Ok(
                        json!({ "ok": false, "stage": "chapter", "error": "Source engine returned empty chapter content" }),
                    );
                }
                Ok(_) => {
                    return Ok(
                        json!({ "ok": false, "stage": "chapter", "error": "Source engine returned non-text chapter content" }),
                    );
                }
                Err(error) => {
                    return Ok(json!({ "ok": false, "stage": "chapter", "error": error }));
                }
            };
            if crate::source_metadata::media_type(&source.source).is_some() {
                return Ok(
                    json!({ "ok": false, "stage": "chapter", "error": "Media chapters can be opened after adding the book to the shelf" }),
                );
            }

            let _sources = self.sources_lock.lock().await;
            let current_source = match self.read_sources().await {
                Ok(sources) => sources
                    .into_iter()
                    .find(|candidate| candidate.id == source_id),
                Err(error) => return Ok(json!({ "ok": false, "stage": "source", "error": error })),
            };
            let source_is_current = match current_source.as_ref() {
                Some(current) if current.enabled && current.source == source.source => {
                    match self.source_revision(&source_id).await {
                        Ok(revision) => revision == source_revision,
                        Err(error) => {
                            return Ok(json!({ "ok": false, "stage": "source", "error": error }));
                        }
                    }
                }
                _ => false,
            };
            if !source_is_current {
                return Ok(
                    json!({ "ok": false, "stage": "source", "error": "Search result source changed during chapter preview; search again" }),
                );
            }
            let preview_id = uuid::Uuid::new_v4().simple().to_string();
            let chapter_ref = if looks_like_html(&content) {
                self.store
                    .write_chapter_rich_text(
                        "debug-source",
                        &preview_id,
                        &content,
                        content_base_url.as_deref(),
                    )
                    .await
            } else {
                self.store
                    .write_chapter_text("debug-source", &preview_id, &content)
                    .await
            };
            match chapter_ref {
                Ok(reference) => chapter_resource = Some(self.resource_descriptor(&reference)),
                Err(error) => {
                    return Ok(
                        json!({ "ok": false, "stage": "chapter", "error": error.to_string() }),
                    );
                }
            }
        }

        Ok(json!({
            "ok": true,
            "stage": if chapter_resource.is_some() { "chapter" } else { "catalog" },
            "book": metadata,
            "chapters": chapters,
            "chapterResource": chapter_resource,
        }))
    }

    async fn clear_source_debug_preview(&self, resource_id: &str) -> Result<(), String> {
        let reference = parse_source_debug_preview_ref(resource_id)?;
        let writer = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        writer
            .remove_asset(&reference)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

/// Debug previews have unique chapter IDs and content-versioned JSON names.
/// Only allow a validated local ref in the dedicated debug-source namespace.
fn parse_source_debug_preview_ref(resource_id: &str) -> Result<ResourceRef, String> {
    let reference = ResourceRef::new(resource_id).map_err(|error| error.to_string())?;
    let filename = reference
        .path()
        .strip_prefix("books/debug-source/chapters/")
        .and_then(|path| path.strip_suffix(".json"))
        .ok_or_else(|| "Invalid source debug preview resource ID".to_owned())?;
    let (identity_hash, content_hash) = filename
        .split_once('-')
        .ok_or_else(|| "Invalid source debug preview resource ID".to_owned())?;
    if [identity_hash, content_hash].iter().any(|hash| {
        hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err("Invalid source debug preview resource ID".into());
    }
    Ok(reference)
}
