//! 书籍加入、信息刷新和书籍详情资源。

use super::*;

struct AddBookSnapshot {
    public: Value,
    private: Value,
}

#[cfg(test)]
mod detail_tests {
    use super::*;

    struct DetailExecutor {
        stall_operation: &'static str,
        entered: tokio::sync::Notify,
        cancellation: std::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl SourceExecutor for DetailExecutor {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if request.source["bookSourceName"] == "slow" && request.operation == self.stall_operation {
                    *self.cancellation.lock().unwrap() = request.cancellation.clone();
                    self.entered.notify_one();
                    return std::future::pending().await;
                }
                match request.operation.as_str() {
                    "bookInfo" => Ok(request.book.unwrap()),
                    "chapters" => Ok(json!([{"title": "Chapter", "url": "https://fixture/chapter"}])),
                    _ => Err("Unexpected detail operation".into()),
                }
            })
        }
    }

    #[tokio::test]
    async fn selecting_another_source_cancels_pending_info_or_catalog_without_waiting() {
        for stall_operation in ["bookInfo", "chapters"] {
            let directory = tempfile::tempdir().unwrap();
            let executor = Arc::new(DetailExecutor { stall_operation, entered: tokio::sync::Notify::new(), cancellation: std::sync::Mutex::new(None), calls: 0.into() });
            let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
            let records = ["slow", "fast"].map(|id| SourceRecord {
                id: id.into(), name: id.into(), group: None, enabled: true,
                source: json!({"bookSourceName": id, "bookSourceUrl": format!("https://fixture/{id}"), "bookSourceType": 0}),
            });
            service.write_sources(&records).await.unwrap();
            for source in &records {
                let cache = json!({"sourceId": source.id,
                    "book": {"name": "Fixture", "author": "Author", "bookUrl": format!("https://fixture/{}/book", source.id)}});
                service.tasks.lock().await.search_result_documents.insert(format!("result-{}", source.id), cache);
            }
            let worker = service.clone();
            let slow = tokio::spawn(async move { worker.prepare_search_result_book_with_revision("result-slow", Some(10)).await });
            tokio::time::timeout(Duration::from_secs(2), executor.entered.notified()).await.unwrap();
            let mut edited = records.to_vec();
            edited[1].source["ruleSearch"] = json!({"name": "changed"});
            service.write_sources(&edited).await.unwrap();
            let sources_guard = service.sources_lock.lock().await;
            let fast = tokio::time::timeout(Duration::from_secs(2), service.prepare_search_result_book_with_revision("result-fast", Some(11)))
                .await.unwrap().unwrap();
            drop(sources_guard);
            assert!(fast["book"]["bookId"].as_str().unwrap().contains("cache-"));
            assert!(tokio::time::timeout(Duration::from_secs(1), slow).await.unwrap().unwrap().unwrap_err().contains("cancelled"));
            assert!(executor.cancellation.lock().unwrap().as_ref().unwrap().is_cancelled());
            // Out-of-order cancellation must not cancel a newer request, and a
            // cancel received before command admission must still take effect.
            service.cancel_book_preparation(10);
            assert_eq!(service.detail_preparation.lock().unwrap().0, 11);
            service.cancel_book_preparation(12);
            assert!(service.prepare_search_result_book_with_revision("result-fast", Some(12)).await.unwrap_err().contains("superseded"));
            let id = fast["book"]["bookId"].as_str().unwrap();
            let reference=service.store.book_ref(id).unwrap();
            let path=reference.path();
            service.save_progress(id, json!({"chapterIndex": 0, "offset": 17, "updatedAtMs": 100})).await.unwrap();
            let before = service.store.database().read(path).unwrap().unwrap();
            let calls = executor.calls.load(std::sync::atomic::Ordering::SeqCst);
            service.prepare_search_result_book_with_revision("result-fast", Some(13)).await.unwrap();
            assert_eq!(executor.calls.load(std::sync::atomic::Ordering::SeqCst), calls, "Cached SQLite book must not rerun bookInfo/chapters");
            let after = service.store.database().read(path).unwrap().unwrap();
            for field in ["progress", "bookInstanceId", "catalogGeneration", "chapters"] {
                assert_eq!(after[field], before[field], "Reopening must preserve {field}");
            }
            service.store.update_json_ref(&service.store.book_ref(id).unwrap(), |mut book| {
                book["title"] = json!("Saved title");
                Ok(book)
            }).await.unwrap();
            let before_read = service.store.database().read(path).unwrap().unwrap();
            let sources_guard = service.sources_lock.lock().await;
            tokio::time::timeout(Duration::from_secs(1), service.get_book(id)).await.unwrap().unwrap();
            assert_eq!(service.store.database().read(path).unwrap().unwrap(), before_read,
                "Opening cached details must not rewrite stored metadata or scan sources");
            drop(sources_guard);
        }
    }
}

impl ApplicationService {
    pub(super) async fn saved_alternative_candidate(&self, book_id: &str, index: usize, expected_source: Option<&str>) -> Result<Value, String> {
        let records = self.read_sources().await?;
        let _book = self.book_lock(book_id).await;
        let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        let book = self.store.read_json_ref(&reference).await.map_err(|error| error.to_string())?;
        let private = self.read_internal_json(self.book_engine_path(book_id)?).await?;
        let alternative = book["alternatives"].as_array().and_then(|items| items.get(index)).ok_or("Saved alternative no longer exists")?;
        let source_id = alternative["sourceId"].as_str().ok_or("Saved alternative has no source")?;
        if Some(source_id) != expected_source { return Err("Alternative list changed; choose the source again".into()); }
        if !records.iter().any(|source| source.id == source_id && source.enabled) { return Err("Alternative source is unavailable".into()); }
        let original_id = private["sourceId"].as_str().ok_or("Content has no source")?;
        let mut engine = alternative["engine"].clone();
        engine["bookUrl"] = alternative["engineurl"].clone();
        let identity = json!({"targetTitle": text_at(&private["book"], &["name", "title"]).unwrap_or_default(),
            "targetAuthor": text_at(&private["book"], &["author"]).unwrap_or_default()});
        let confirmation = candidate_identity_match(&identity, &engine).ok_or("Alternative content identity differs")?;
        Ok(json!({"sourceId": source_id, "book": engine,
            "replacementContext": {"targetBookId": book_id, "originalSourceId": original_id,
                "catalogGeneration": private["catalogGeneration"], "bookInstanceId": private["bookInstanceId"],
                "requiresIdentityConfirmation": confirmation,
                "expiresAtMs": now_ms().saturating_add(BOOK_SOURCE_CANDIDATE_TTL_MS)}}))
    }

    /// Resolve original /read and /addToBookshelf links through the existing book preparation flow.
    pub async fn prepare_book_from_url(&self, book_url: &str, read: bool) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let book_url = book_url.trim();
        let option_start = regex::Regex::new(r",\s*\{")
            .unwrap()
            .find(book_url)
            .map(|entry| entry.start());
        let url = parse_http_json_import_url(
            &book_url[..option_start.unwrap_or(book_url.len())],
            "书籍",
        )?;
        let origin = option_start
            .map(|start| {
                serde_json::from_str::<Value>(book_url[start + 1..].trim())
                    .map_err(|error| format!("书籍链接参数不正确：{error}"))
            })
            .transpose()?
            .and_then(|options| text_at(&options, &["origin"]));
        let shelf = self
            .store
            .read_json_ref(&self.store.shelf_ref())
            .await
            .map_err(|error| error.to_string())?;
        for entry in shelf["books"].as_array().into_iter().flatten() {
            let Some(id) = entry["id"].as_str() else {
                continue;
            };
            let _book = self.book_lock(id).await;
            let Ok(private) = self
                .read_internal_json(&self.book_engine_path(id)?)
                .await
            else {
                continue;
            };
            if text_at(&private["book"], &["bookUrl", "url"]).as_deref() == Some(book_url) {
                if !read {
                    return Err("已在书架".into());
                }
                let reference = self.store.book_ref(id).map_err(|error| error.to_string())?;
                return Ok(
                    json!({ "book": self.resource_descriptor(&reference), "inShelf": true }),
                );
            }
        }
        let cache = {
            let records = self.read_sources().await?;
            let candidates = records
                .iter()
                .filter(|source| source.enabled && !crate::rss::is_rss_source(&source.source))
                .collect::<Vec<_>>();
            let source = if let Some(origin) = origin.as_deref() {
                candidates.iter().copied().find(|source| {
                    text_at(&source.source, &["bookSourceUrl"]).as_deref() == Some(origin)
                })
            } else {
                candidates
                    .iter()
                    .copied()
                    .find(|source| {
                        text_at(&source.source, &["bookSourceUrl"])
                            .and_then(|base| reqwest::Url::parse(&base).ok())
                            .is_some_and(|base| base.origin() == url.origin())
                    })
                    .or_else(|| {
                        candidates.iter().copied().find(|source| {
                            text_at(&source.source, &["bookUrlPattern"])
                                .and_then(|pattern| {
                                    regex::Regex::new(&format!("^(?:{pattern})$")).ok()
                                })
                                .is_some_and(|pattern| pattern.is_match(book_url))
                        })
                    })
            }
            .ok_or_else(|| "未找到匹配书源".to_owned())?;
            json!({
                "sourceId": source.id,
                "book": { "bookUrl": book_url, "origin": text_at(&source.source, &["bookSourceUrl"]), "originName": source.name },
            })
        };
        let mut result = self.materialize_source_book("linked-book", cache).await?;
        result["inShelf"] = json!(false);
        Ok(result)
    }

    pub async fn prepare_search_result_book(&self, result_id: &str) -> Result<Value, String> {
        self.prepare_search_result_book_with_revision(result_id, None).await
    }

    pub fn cancel_book_preparation(&self, revision: u64) {
        let mut current = self.detail_preparation.lock().unwrap_or_else(|error| error.into_inner());
        if revision >= current.0 {
            if let Some(token) = current.1.take() { token.cancel(); }
            current.0 = revision;
        }
    }

    pub async fn prepare_search_result_book_with_revision(&self, result_id: &str, revision: Option<u64>) -> Result<Value, String> {
        let token = tokio_util::sync::CancellationToken::new();
        if let Some(revision) = revision {
            let mut current = self.detail_preparation.lock().unwrap_or_else(|error| error.into_inner());
            // A cancel or a newer selection can arrive before this command is admitted.
            if revision <= current.0 { return Err("Book preparation superseded".into()); }
            if let Some(previous) = current.1.replace(token.clone()) { previous.cancel(); }
            current.0 = revision;
        }
        let _cancel_on_drop = token.clone().drop_guard();
        let _operation = self.operation_read().await;
        validate_id(result_id, "resultId")?;
        let cache = self.read_search_result_cache(result_id).await?;
        let selected = self.materialize_source_book_cancellable(result_id, cache, token.clone()).await?;
        if token.is_cancelled() { return Err("Book preparation cancelled".into()); }
        let book_id = selected["book"]["bookId"].as_str().ok_or_else(|| "Prepared content has no book ID".to_owned())?;
        let alternatives = {
            let registry = self.tasks.lock().await;
            registry.search_documents.values().filter_map(|document| document["results"].as_array()).flatten()
                .find(|result| result["resultId"] == result_id || result["sources"].as_array().is_some_and(|sources| sources.iter().any(|source| source["resultId"] == result_id)))
                .and_then(|result| result["sources"].as_array()).into_iter().flatten()
                .filter(|leaf| leaf["resultId"] != result_id)
                .filter_map(|leaf| registry.search_result_documents.get(leaf["resultId"].as_str()?).map(|cache| alternative_from_search(leaf, cache)))
                .collect::<Vec<_>>()
        };
        self.set_alternatives(book_id, alternatives).await?;
        Ok(selected)
    }

    pub async fn prepare_alternative_book(&self, book_id: &str, index: usize, expected_source_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        let current = self.store.read_json_ref(&reference).await.map_err(|error| error.to_string())?;
        let alternative = current["alternatives"].as_array().and_then(|items| items.get(index)).cloned()
            .ok_or_else(|| "Alternative source changed; reopen the source list".to_owned())?;
        if alternative["sourceId"] != expected_source_id { return Err("Alternative source changed; reopen the source list".into()); }
        let mut engine_book = alternative["engine"].clone();
        if !engine_book.is_object() { return Err("Alternative is missing source execution parameters".into()); }
        engine_book["bookUrl"] = alternative["engineurl"].clone();
        let cache = json!({"sourceId": expected_source_id, "book": engine_book});
        self.materialize_source_book("alternative", cache).await
    }

    async fn materialize_source_book(&self, result_id: &str, cache: Value) -> Result<Value, String> {
        let token = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = token.clone().drop_guard();
        self.materialize_source_book_cancellable(result_id, cache, token).await
    }

    async fn materialize_source_book_cancellable(&self, result_id: &str, cache: Value, token: tokio_util::sync::CancellationToken) -> Result<Value, String> {
        if token.is_cancelled() { return Err("Book preparation cancelled".into()); }
        let source_id = cache
            .get("sourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| "Search result has no source ID".to_owned())?
            .to_owned();
        let raw_book = cache
            .get("book")
            .cloned()
            .ok_or_else(|| "Search result has no book".to_owned())?;
        if !raw_book.is_object() {
            return Err("Search result has invalid book data".into());
        }
        let url = text_at(&raw_book, &["bookUrl", "url", "origin"])
            .unwrap_or_else(|| result_id.to_owned());
        let source = self.find_source(&source_id).await?;
        if !source.enabled { return Err("Search result source is disabled".into()); }
        let source_kind = crate::source_metadata::content_type(&source.source);
        let book_id = format!("cache-{:016x}", stable_hash(&format!("{source_id}\0{source_kind}\0{url}")));
        let initial_state = {
            let _book = self.book_lock(&book_id).await;
            let state = self.read_add_book_snapshot(&book_id).await?;
            if let Some(state) = state.as_ref() {
                if state.private.get("sourceId").and_then(Value::as_str) != Some(source_id.as_str())
                {
                    return Err("A different book already uses this catalog identity".into());
                }
            }
            state
        };
        if initial_state.is_none() {
            self.store.bind_book_type(&book_id, crate::source_metadata::content_type(&source.source)).await.map_err(|error| error.to_string())?;
        }
        let book_ref = self.store.book_ref(&book_id).map_err(|error| error.to_string())?;
        if initial_state.is_some() {
            return Ok(json!({ "book": self.resource_descriptor(&book_ref) }));
        }
        let legacy_rss = crate::rss::is_legacy_rss_source(&source.source);
        let execute = |mut request: SourceEngineRequest| {
            request.cancellation = Some(token.clone());
            let token = token.clone();
            async move {
                tokio::select! {
                    biased;
                    _ = token.cancelled() => Err("Book preparation cancelled".to_owned()),
                    result = self.executor.execute(request) => result,
                }
            }
        };
        let info = execute(engine_request(
                if legacy_rss {
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
            .await?;
        let engine_book = if info.is_object() && !info.as_object().is_some_and(Map::is_empty) {
            info
        } else {
            raw_book.clone()
        };
        let chapters = execute(engine_request(
                if legacy_rss {
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
            .await?;
        let raw_chapters = chapters
            .as_array()
            .cloned()
            .ok_or_else(|| "Source engine returned an invalid chapter catalog".to_owned())?;
        // Reject ambiguous first-import catalogs too. The same pure planner
        // used by refresh catches duplicate canonical URLs and generated IDs
        // before any public book or private engine data is written.
        crate::catalog::reconcile_catalog(
            &book_id,
            &[],
            &[],
            &raw_chapters,
            &ProgressSummary::default(),
            &HashSet::new(),
            now_ms(),
        )
        .map_err(|error| format!("Cannot add book with invalid chapter catalog: {error}"))?;

        if token.is_cancelled() { return Err("Book preparation cancelled".into()); }
        let _book = self.book_lock(&book_id).await;
        let latest_state = self.read_add_book_snapshot(&book_id).await?;
        if !add_book_catalog_unchanged(initial_state.as_ref(), latest_state.as_ref()) {
            return Err("Book catalog changed during book lookup; retry adding it".into());
        }

        let display_metadata = crate::book_metadata::project_book_metadata(&engine_book, &source);
        let title = display_metadata.title.clone();
        let author = display_metadata.author.clone().unwrap_or_default();
        let latest = raw_chapters
            .last()
            .and_then(|chapter| text_at(chapter, &["title", "chapterName", "name"]));
        let old_raw_chapters = latest_state
            .as_ref()
            .and_then(|state| state.private.get("chapters"))
            .and_then(Value::as_array);
        let old_chapters = latest_state
            .as_ref()
            .and_then(|state| state.public.get("chapters"))
            .and_then(Value::as_array);
        let old_by_url: HashMap<String, Value> = old_raw_chapters
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, raw)| {
                let url = text_at(raw, &["url", "chapterUrl"])?;
                Some((url, old_chapters?.get(index)?.clone()))
            })
            .collect();
        let mut descriptors = Vec::with_capacity(raw_chapters.len());
        for (index, chapter) in raw_chapters.iter().enumerate() {
            let raw_url =
                text_at(chapter, &["url", "chapterUrl"]).unwrap_or_else(|| index.to_string());
            let previous = old_by_url.get(&raw_url);
            let id = previous
                .and_then(|old| old.get("id").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| chapter_id(&book_id, &raw_url));
            let declared_ref = previous
                .filter(|old| old.get("id").and_then(Value::as_str) == Some(&id))
                .and_then(|old| old.get("resource"))
                .and_then(|resource| resource.get("resourceId"))
                .and_then(Value::as_str)
                .filter(|src| !src.is_empty())
                .map(|src| scoped_chapter_cache_ref(&book_id, src))
                .transpose()?;
            let resource = if let Some(reference) = declared_ref {
                self.cached_chapter_format(&reference)
                    .await?
                    .map(|format| crate::models::ChapterResourceDescriptor::for_chapter(&reference, format))
            } else {
                None
            };
            descriptors.push(ChapterDescriptor {
                id,
                title: text_at(chapter, &["title", "chapterName", "name"])
                    .unwrap_or_else(|| format!("Chapter {}", index + 1)),
                index,
                resource,
            });
        }
        let progress = latest_state
            .as_ref()
            .and_then(|state| serde_json::from_value(state.public.get("progress")?.clone()).ok())
            .unwrap_or_default();
        let book_instance_id = latest_state
            .as_ref()
            .and_then(|state| state.private.get("bookInstanceId"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let book = BookDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: book_id.clone(),
            title,
            can_change_source: true,
            author,
            cover_src: display_metadata.cover_src.clone(),
            intro: display_metadata.intro.clone(),
            kind: display_metadata.kind.clone(),
            content_type: display_metadata.content_type.clone(),
            media_type: crate::source_metadata::media_type(&source.source)
                .and_then(BookMediaType::from_source_value),
            word_count: display_metadata.word_count.clone(),
            source_id: Some(display_metadata.source_id.clone()),
            source_name: Some(display_metadata.source_name.clone()),
            source_group: display_metadata.source_group.clone(),
                display_base: None,
            display_overrides: None,
            chapter_count: descriptors.len(),
            latest_chapter: latest,
            progress,
            chapters: descriptors,
        };
        let mut book_json = serde_json::to_value(book).map_err(|error| error.to_string())?;
        if let Some(existing) = latest_state.as_ref() {
            for chapter in book_json["chapters"].as_array_mut().into_iter().flatten() {
                if let Some(previous) = existing.public["chapters"].as_array().into_iter().flatten().find(|old| old["id"] == chapter["id"]) {
                    for key in ["pages", "sources", "protocol", "files", "url", "media"] {
                        if let Some(value) = previous.get(key) { chapter[key] = value.clone(); }
                    }
                    if previous["media"].is_object() { chapter["resource"] = previous["resource"].clone(); }
                }
            }
            if let (Some(target), Some(previous)) =
                (book_json.as_object_mut(), existing.public.as_object())
            {
                for key in [
                    "displayBase",
                    "displayOverrides",
                    "alternatives",
                ] {
                    if let Some(value) = previous.get(key) {
                        target.insert(key.to_owned(), value.clone());
                    }
                }
            }
        }
        crate::book_display_metadata::update_base(&mut book_json, &display_metadata)?;
        book_json["updatedAtMs"] = if latest_state.as_ref().is_some_and(|old| old.private["chapters"] == json!(raw_chapters)) {
            latest_state.as_ref().map(|old| old.public["updatedAtMs"].clone()).unwrap_or(Value::Null)
        } else { json!(now_ms()) };
        let mut private_json = latest_state
            .as_ref()
            .map(|state| state.private.clone())
            .unwrap_or_else(|| json!({}));
        private_json["sourceId"] = json!(source_id);
        private_json["bookInstanceId"] = json!(book_instance_id);
        private_json["catalogGeneration"] = json!(uuid::Uuid::new_v4().simple().to_string());
        private_json["book"] = engine_book;
        private_json["chapters"] = json!(raw_chapters);
        self.store.write_book_state(&book_id, &book_json, &private_json).await
            .map_err(|error| format!("Cannot save prepared content: {error}"))?;
        if let Some(search_id) = cache.get("searchId").and_then(Value::as_str) {
            let mut registry = self.tasks.lock().await;
            let group_root = registry.search_documents.get(search_id)
                .and_then(|document| document.get("results"))
                .and_then(Value::as_array)
                .and_then(|results| results.iter().find(|result| {
                    result["resultId"].as_str() == Some(result_id)
                        || result["sources"].as_array().is_some_and(|sources| {
                            sources.iter().any(|source| {
                                source["resultId"].as_str() == Some(result_id)
                            })
                        })
                }))
                .and_then(|result| result.get("resultId"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(group_root) = group_root {
                registry.search_book_group_roots.insert(book_id.clone(), group_root);
            }
        }
        Ok(json!({ "book": self.resource_descriptor(&book_ref) }))
    }

    pub(super) async fn set_alternatives(&self, book_id: &str, alternatives: Vec<Value>) -> Result<(), String> {
        if alternatives.is_empty() { return Ok(()); }
        let _book = self.book_lock(book_id).await;
        let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        let database = self.store.database().clone();
        let path = reference.path().to_owned();
        let changed = tokio::task::spawn_blocking(move || database.merge_alternatives(&path, alternatives))
            .await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        if !changed { return Ok(()); }
        if let Ok(notifier) = self.book_notifier.read() {
            if let Some(notifier) = notifier.as_ref() { notifier(json!({"requestId": 0, "resource": self.resource_descriptor(&reference)})); }
        }
        Ok(())
    }

    pub(super) async fn cache_source_candidates(&self, search_id: &str) -> Result<(), String> {
        let (book_id, alternatives) = {
            let registry = self.tasks.lock().await;
            let context = registry.search_replacement_contexts.get(search_id).ok_or_else(|| "Candidate search expired".to_owned())?;
            let book_id = context["targetBookId"].as_str().ok_or_else(|| "Candidate search has no content".to_owned())?.to_owned();
            let alternatives = registry.search_documents.get(search_id).and_then(|doc| doc["results"].as_array())
                .into_iter().flatten().filter_map(|leaf| registry.search_result_documents.get(leaf["resultId"].as_str()?).map(|cache| alternative_from_search(leaf, cache)))
                .collect::<Vec<_>>();
            (book_id, alternatives)
        };
        self.set_alternatives(&book_id, alternatives).await
    }

    async fn read_add_book_snapshot(
        &self,
        book_id: &str,
    ) -> Result<Option<AddBookSnapshot>, String> {
        // Metadata and engine state share one SQLite document. Checking the old
        // JSON file always reports a cache miss and refetches/reset cached books.
        let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        let database = self.store.database().clone();
        let path = reference.path().to_owned();
        let document = tokio::task::spawn_blocking(move || database.read(&path))
            .await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        let Some(document) = document else { return Ok(None); };
        let kind = self.store.book_type(book_id).map_err(|error| error.to_string())?;
        let public = crate::resources::content::book_view(&document, &kind).map_err(|error| error.to_string())?;
        let private = crate::resources::content::source_state(&document);
        if public.get("id").and_then(Value::as_str) != Some(book_id)
            || public.get("chapters").and_then(Value::as_array).is_none()
            || private.get("sourceId").and_then(Value::as_str).is_none()
            || private
                .get("bookInstanceId")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || private.get("book").is_none_or(|book| !book.is_object())
            || private.get("chapters").and_then(Value::as_array).is_none()
        {
            return Err(
                "Existing book catalog is incomplete; repair it before adding again".into(),
            );
        }
        Ok(Some(AddBookSnapshot { public, private }))
    }

    pub async fn get_book(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        if !self.store.regular_file_exists(&reference).await.map_err(|error| error.to_string())? {
            return Err("Book is no longer available".into());
        }
        Ok(self.resource_descriptor(&reference))
    }

    /// Refresh a book's processed details without holding its guard over network work.
    pub async fn refresh_book_info(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let private_path = self.book_engine_path(book_id)?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;

        let (
            source,
            book_instance_id,
            catalog_generation,
            engine_book,
            raw_chapters,
            public_latest_chapter,
        ) = {
            let sources = self.read_sources().await?;
            let _book = self.book_lock(book_id).await;
            let private = self.read_internal_json(&private_path).await?;
            let source_id = private
                .get("sourceId")
                .and_then(Value::as_str)
                .filter(|source_id| !source_id.trim().is_empty())
                .ok_or_else(|| "This book is not linked to an online source".to_owned())?;
            let source = sources
                .iter()
                .find(|source| source.id == source_id)
                .cloned()
                .ok_or_else(|| format!("Book source '{source_id}' is no longer imported"))?;
            if !source.enabled {
                return Err("Book source is disabled; enable it before refreshing details".into());
            }
            if !can_change_source_from_private(&private) {
                return Err("This book has no refreshable online source metadata".into());
            }
            let book_instance_id = private
                .get("bookInstanceId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Private book is missing its instance identity".to_owned())?
                .to_owned();
            let engine_book = private
                .get("book")
                .filter(|value| value.is_object())
                .cloned()
                .ok_or_else(|| "Private book is missing processed engine metadata".to_owned())?;
            let raw_chapters = private
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| "Private book is missing its source chapter catalog".to_owned())?;
            let catalog_generation = private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null);
            let public_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            if public_book.get("id").and_then(Value::as_str) != Some(book_id) {
                return Err("Book resource ID does not match its catalog path".into());
            }
            (
                source,
                book_instance_id,
                catalog_generation,
                engine_book,
                raw_chapters,
                public_book.get("latestChapter").cloned(),
            )
        };

        let operation = if crate::rss::is_legacy_rss_source(&source.source) {
            "rssBookInfo"
        } else {
            "bookInfo"
        };
        let response = self
            .executor
            .execute(engine_request(
                operation,
                &source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?;
        let refreshed_detail_keys = [
            "name",
            "title",
            "author",
            "coverUrl",
            "cover",
            "coverSrc",
            "intro",
            "introduction",
            "kind",
            "category",
            "wordCount",
            "word_count",
            "lastChapter",
            "latestChapter",
            "latestChapterTitle",
        ];
        let response_fields = response
            .as_object()
            .ok_or_else(|| "Source engine returned invalid book details".to_owned())?;
        let response_metadata = crate::book_metadata::project_book_metadata(&response, &source);
        let has_usable_detail = response_metadata.title != "Untitled"
            || response_metadata.author.is_some()
            || response_metadata.cover_src.is_some()
            || response_metadata.intro.is_some()
            || response_metadata.kind.is_some()
            || response_metadata.word_count.is_some()
            || response_metadata.latest_chapter.is_some();
        if !has_usable_detail {
            return Err("Source engine returned no refreshable book details".into());
        }
        let mut refreshed_fields = engine_book
            .as_object()
            .cloned()
            .ok_or_else(|| "Private book engine metadata is invalid".to_owned())?;
        // Keep the full prior engine object as the base so partial info
        // responses do not discard source-specific navigation/context data.
        // Catalog identity is owned by the existing book and cannot be changed
        // by a metadata refresh; a new URL requires an explicit catalog flow.
        const CATALOG_IDENTITY_KEYS: [&str; 5] = ["bookUrl", "url", "origin", "articleId", "id"];
        for (key, value) in response_fields {
            if CATALOG_IDENTITY_KEYS.contains(&key.as_str()) || value.is_null() {
                continue;
            }
            if refreshed_detail_keys.contains(&key.as_str()) {
                let valid = match key.as_str() {
                    "name" | "title" | "author" | "coverUrl" | "cover" | "coverSrc" | "intro"
                    | "introduction" | "lastChapter" | "latestChapter" | "latestChapterTitle" => {
                        value.as_str().is_some_and(|text| !text.trim().is_empty())
                    }
                    "kind" | "category" => {
                        value.as_str().is_some_and(|text| !text.trim().is_empty())
                            || value.as_array().is_some_and(|values| {
                                values.iter().any(|value| {
                                    value.as_str().is_some_and(|text| !text.trim().is_empty())
                                })
                            })
                    }
                    "wordCount" | "word_count" => {
                        value.as_number().is_some()
                            || value.as_str().is_some_and(|text| !text.trim().is_empty())
                    }
                    _ => true,
                };
                if !valid {
                    continue;
                }
            }
            refreshed_fields.insert(key.clone(), value.clone());
        }
        let mut refreshed_engine_book = Value::Object(refreshed_fields);

        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let old_private = self
            .read_internal_json(&private_path)
            .await
            .map_err(|_| format!("Book '{book_id}' was removed during metadata refresh"))?;
        if old_private.get("sourceId").and_then(Value::as_str) != Some(source.id.as_str())
            || old_private.get("bookInstanceId").and_then(Value::as_str)
                != Some(book_instance_id.as_str())
            || old_private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null)
                != catalog_generation
            || old_private.get("book") != Some(&engine_book)
        {
            return Err("Book changed during metadata refresh; refresh again".into());
        }
        let old_book = writer_guard
            .read_json_ref(&book_ref)
            .map_err(|_| format!("Book '{book_id}' was removed during metadata refresh"))?;
        if old_book.get("id").and_then(Value::as_str) != Some(book_id) {
            return Err("Book resource ID does not match its catalog path".into());
        }
        let catalog_changed_while_refreshing = (catalog_generation.is_null() && old_private
            .get("chapters")
            .and_then(Value::as_array)
            .is_none_or(|chapters| chapters != &raw_chapters))
            || old_book.get("latestChapter").cloned() != public_latest_chapter;
        if catalog_changed_while_refreshing {
            if let Some(refreshed_fields) = refreshed_engine_book.as_object_mut() {
                for key in ["lastChapter", "latestChapter", "latestChapterTitle"] {
                    refreshed_fields.remove(key);
                }
            }
        }
        let display_metadata =
            crate::book_metadata::project_book_metadata(&refreshed_engine_book, &source);

        let mut next_private = old_private.clone();
        next_private["book"] = refreshed_engine_book;
        let mut next_book = old_book.clone();
        apply_refreshed_book_metadata(&mut next_book, &display_metadata)?;
        let replacements = vec![writer_guard.content_replacement(&book_ref, &next_book, &next_private)
            .map_err(|error| error.to_string())?];
        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "refresh-book-info",
                replacements,
                Vec::new(),
                &writer_guard,
            )?;
            transaction.commit()
        })
        .await;
        let (commit_state, recovery_required, warning) = match transaction_result {
            Ok(Ok(())) => (
                crate::resource_transactions::CommitState::Committed,
                false,
                None,
            ),
            Ok(Err(error)) => {
                let commit_state = error.commit_state();
                let recovery_required = error.recovery_required_flag();
                let warning = match commit_state {
                    crate::resource_transactions::CommitState::NotCommitted
                        if recovery_required =>
                    {
                        Some(format!(
                            "Book detail refresh was not committed, but rollback is incomplete and files may be mixed. Restart before retrying. Details: {error}"
                        ))
                    }
                    crate::resource_transactions::CommitState::NotCommitted => Some(format!(
                        "Book detail refresh was not committed. Changes were not saved and it is safe to retry. Details: {error}"
                    )),
                    crate::resource_transactions::CommitState::Committed => Some(format!(
                        "Book detail refresh was committed and is active, but journal recovery is pending. Restart before further writes. Details: {error}"
                    )),
                    crate::resource_transactions::CommitState::Indeterminate => Some(format!(
                        "Book detail refresh status is unknown. Do not retry; restart the application to recover the transaction first. Details: {error}"
                    )),
                };
                (commit_state, recovery_required, warning)
            }
            Err(error) => (
                crate::resource_transactions::CommitState::Indeterminate,
                true,
                Some(format!(
                    "Book detail refresh status is unknown. Do not retry; restart the application to recover the transaction first. Details: {error}"
                )),
            ),
        };
        let resources = (commit_state == crate::resource_transactions::CommitState::Committed)
            .then(|| {
                self.resource_descriptor(&book_ref)
            });
        Ok(book_display_metadata_commit_outcome(
            commit_state,
            recovery_required,
            warning,
            resources,
        ))
    }
}

pub(super) fn add_book_catalog_identity(book: &Value) -> Option<Value> {
    let chapters = book.get("chapters")?.as_array()?;
    Some(json!(
        chapters
            .iter()
            .map(|chapter| json!({
                "id": chapter.get("id"),
                "title": chapter.get("title"),
                "index": chapter.get("index"),
            }))
            .collect::<Vec<_>>()
    ))
}

fn add_book_catalog_unchanged(
    before: Option<&AddBookSnapshot>,
    after: Option<&AddBookSnapshot>,
) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            before.public.get("id") == after.public.get("id")
                && before.private.get("sourceId") == after.private.get("sourceId")
                && before.private.get("bookInstanceId") == after.private.get("bookInstanceId")
                && before.private.get("catalogGeneration") == after.private.get("catalogGeneration")
                && before.private.get("book") == after.private.get("book")
                && before.private.get("chapters") == after.private.get("chapters")
                && before.public.get("latestChapter") == after.public.get("latestChapter")
                && add_book_catalog_identity(&before.public)
                    == add_book_catalog_identity(&after.public)
        }
        _ => false,
    }
}

pub(super) fn apply_refreshed_book_metadata(
    book: &mut Value,
    metadata: &crate::book_metadata::ProcessedBookMetadata,
) -> Result<(), String> {
    crate::book_display_metadata::update_base(book, metadata)?;
    if let Some(content_type) = metadata.content_type.as_deref() {
        book["contentType"] = json!(content_type);
        set_optional_public_field(book, "mediaType",
            matches!(content_type, "audio" | "video").then(|| json!(content_type)));
    }
    if let Some(kind) = metadata.kind.as_deref() {
        book["kind"] = json!(kind);
    }
    if let Some(word_count) = metadata.word_count.as_deref() {
        book["wordCount"] = json!(word_count);
    }
    if let Some(latest_chapter) = metadata.latest_chapter.as_deref() {
        book["latestChapter"] = json!(latest_chapter);
    }
    if !metadata.source_id.is_empty() {
        book["sourceId"] = json!(metadata.source_id.as_str());
    }
    if !metadata.source_name.is_empty() {
        book["sourceName"] = json!(metadata.source_name.as_str());
    }
    set_optional_public_field(
        book,
        "sourceGroup",
        metadata.source_group.as_deref().map(|group| json!(group)),
    );
    Ok(())
}

pub(super) fn alternative_from_search(leaf: &Value, cache: &Value) -> Value {
    let mut engine = cache["book"].clone();
    let engine_url = text_at(&engine, &["bookUrl", "url"]).unwrap_or_default();
    if let Some(engine) = engine.as_object_mut() { engine.remove("bookUrl"); engine.remove("url"); }
    let mut alternative = json!({"sourceId": leaf["sourceId"], "sourceName": leaf["sourceName"],
        "title": leaf["title"], "engineurl": engine_url, "engine": engine});
    for (from, to) in [("author", "author"), ("coverSrc", "cover"), ("latestChapter", "latest")] {
        if let Some(value) = leaf.get(from).filter(|value| !value.is_null()) { alternative[to] = value.clone(); }
    }
    alternative
}
