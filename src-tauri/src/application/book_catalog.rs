//! 书籍加入、信息刷新和书籍详情资源。

use super::*;

struct AddBookSnapshot {
    public: Value,
    private: Value,
}

impl ApplicationService {
    pub async fn add_book(&self, result_id: &str) -> Result<Value, String> {
        self.materialize_search_result_book(result_id, true).await
    }

    pub async fn prepare_search_result_book(&self, result_id: &str) -> Result<Value, String> {
        self.materialize_search_result_book(result_id, false).await
    }

    async fn materialize_search_result_book(
        &self,
        result_id: &str,
        add_to_shelf: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(result_id, "resultId")?;
        let cache = self.read_search_result_cache(result_id).await?;
        let source_id = cache
            .get("sourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| "Search result has no source ID".to_owned())?
            .to_owned();
        let expected_revision = cache
            .get("sourceRevision")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Search result is stale; search again".to_owned())?;
        let expected_source_fingerprint = cache
            .get("sourceDefinitionFingerprint")
            .and_then(Value::as_str)
            .ok_or_else(|| "Search result is stale; search again".to_owned())?
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
        let book_id = format!("book-{:016x}", stable_hash(&format!("{source_id}\0{url}")));
        let book_ref = self
            .store
            .book_ref(&book_id)
            .map_err(|error| error.to_string())?;
        let (source, source_revision, initial_state) = {
            let _sources = self.sources_lock.lock().await;
            let source = self
                .read_sources()
                .await?
                .into_iter()
                .find(|source| source.id == source_id)
                .ok_or_else(|| "Search result source was removed; search again".to_owned())?;
            if !source.enabled {
                return Err(
                    "Search result source is disabled; search again after enabling it".into(),
                );
            }
            let source_revision = self.source_revision(&source_id).await?;
            if source_revision != expected_revision
                || source_definition_fingerprint(&source.source)? != expected_source_fingerprint
            {
                return Err("Search result source changed; search again".into());
            }
            let _book = self.book_lock(&book_id).await;
            let state = self.read_add_book_snapshot(&book_id).await?;
            if let Some(state) = state.as_ref() {
                if state.private.get("sourceId").and_then(Value::as_str) != Some(source_id.as_str())
                {
                    return Err("A different book already uses this catalog identity".into());
                }
            }
            (source, source_revision, state)
        };
        let legacy_rss = crate::rss::is_legacy_rss_source(&source.source);
        let info = self
            .executor
            .execute(engine_request(
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
        let chapters = self
            .executor
            .execute(engine_request(
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

        // The source and book locks are intentionally reacquired only after
        // both KMP calls have completed. Revisions include source deletion and
        // re-import tombstones, even when the same ID and JSON definition return.
        let _sources = self.sources_lock.lock().await;
        let current_source = self
            .read_sources()
            .await?
            .into_iter()
            .find(|candidate| candidate.id == source_id)
            .ok_or_else(|| "Search result source was removed during book lookup".to_owned())?;
        if !current_source.enabled {
            return Err("Search result source was disabled during book lookup".into());
        }
        let current_revision = self.source_revision(&source_id).await?;
        if current_revision != source_revision
            || current_source.source != source.source
            || source_definition_fingerprint(&current_source.source)? != expected_source_fingerprint
        {
            return Err("Search result source changed during book lookup; search again".into());
        }
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
            if let (Some(target), Some(previous)) =
                (book_json.as_object_mut(), existing.public.as_object())
            {
                for key in ["displayBase", "displayOverrides"] {
                    if let Some(value) = previous.get(key) {
                        target.insert(key.to_owned(), value.clone());
                    }
                }
            }
        }
        crate::book_display_metadata::update_base(&mut book_json, &display_metadata)?;
        let mut private_json = latest_state
            .as_ref()
            .map(|state| state.private.clone())
            .unwrap_or_else(|| json!({}));
        private_json["sourceId"] = json!(source_id);
        private_json["bookInstanceId"] = json!(book_instance_id);
        private_json["catalogGeneration"] = json!(uuid::Uuid::new_v4().simple().to_string());
        private_json["book"] = engine_book;
        private_json["chapters"] = json!(raw_chapters);
        self.write_private_json(
            Path::new("books").join(format!("{book_id}.json")),
            &private_json,
        )
        .await?;
        if let Err(error) = self.store.write_json_ref(&book_ref, &book_json).await {
            let rollback = self
                .restore_add_book_snapshot(&book_id, &book_ref, initial_state.as_ref(), false)
                .await;
            let rollback = rollback
                .map(|_| "ok".to_owned())
                .unwrap_or_else(|error| error);
            return Err(format!(
                "Cannot save prepared book resource: {error}; private rollback: {}",
                rollback
            ));
        }
        if add_to_shelf {
            if let Err(error) = self.upsert_shelf(&book_id).await {
                let rollback = self
                    .restore_add_book_snapshot(&book_id, &book_ref, initial_state.as_ref(), true)
                    .await;
                let rollback = rollback
                    .map(|_| "ok".to_owned())
                    .unwrap_or_else(|error| error);
                return Err(format!(
                    "Cannot update shelf after adding book: {error}; book rollback: {}",
                    rollback
                ));
            }
        }
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
        if add_to_shelf {
            Ok(json!({
                "book": self.resource_descriptor(&book_ref),
                "shelf": self.resource_descriptor(&self.store.shelf_ref()),
            }))
        } else {
            Ok(json!({ "book": self.resource_descriptor(&book_ref) }))
        }
    }

    async fn read_add_book_snapshot(
        &self,
        book_id: &str,
    ) -> Result<Option<AddBookSnapshot>, String> {
        let public_path = self.root.join("books").join(book_id).join("book.json");
        let private_relative = Path::new("books").join(format!("{book_id}.json"));
        let private_path = self.private_root.join(&private_relative);
        let public_exists = tokio::fs::try_exists(&public_path)
            .await
            .map_err(|error| format!("Cannot inspect book resource: {error}"))?;
        let private_exists = tokio::fs::try_exists(&private_path)
            .await
            .map_err(|error| format!("Cannot inspect private book resource: {error}"))?;
        match (public_exists, private_exists) {
            (false, false) => return Ok(None),
            (true, true) => {}
            _ => return Err("Book resource is incomplete; repair it before adding again".into()),
        }
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let public = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|error| error.to_string())?;
        let private = self.read_private_json(&private_relative).await?;
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

    async fn restore_add_book_snapshot(
        &self,
        book_id: &str,
        book_ref: &ResourceRef,
        previous: Option<&AddBookSnapshot>,
        restore_public: bool,
    ) -> Result<(), String> {
        let private_relative = Path::new("books").join(format!("{book_id}.json"));
        if let Some(previous) = previous {
            self.write_private_json(&private_relative, &previous.private)
                .await?;
            if restore_public {
                self.store
                    .write_json_ref(book_ref, &previous.public)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            return Ok(());
        }
        remove_file_if_present(&self.private_root.join(private_relative)).await?;
        if restore_public {
            remove_file_if_present(&self.root.join("books").join(book_id).join("book.json"))
                .await?;
        }
        Ok(())
    }

    pub async fn get_book(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let reference = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let _sources = self.sources_lock.lock().await;
        let sources = self.read_sources().await?;
        let _book = self.book_lock(book_id).await;
        let mut book = self
            .store
            .read_json_ref(&reference)
            .await
            .map_err(|error| error.to_string())?;
        let original_book = book.clone();
        let private = self
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .ok();
        let can_change_source = private.as_ref().is_some_and(can_change_source_from_private);
        if let Some(private) = private.as_ref() {
            if let (Some(source_id), Some(engine_book)) = (
                private.get("sourceId").and_then(Value::as_str),
                private.get("book").filter(|value| value.is_object()),
            ) {
                let source = sources.iter().find(|source| source.id == source_id);
                let fallback_source = SourceRecord {
                    id: source_id.to_owned(),
                    name: book
                        .get("sourceName")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    group: book
                        .get("sourceGroup")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    enabled: false,
                    source: Value::Null,
                };
                let metadata = crate::book_metadata::project_book_metadata(
                    engine_book,
                    source.unwrap_or(&fallback_source),
                );
                apply_cached_book_metadata(&mut book, &metadata, source.is_some())?;
                if let Some(source) = source {
                    set_optional_public_field(
                        &mut book,
                        "mediaType",
                        crate::source_metadata::media_type(&source.source)
                            .map(|media_type| json!(media_type)),
                    );
                }
            }
        }
        if book.get("canChangeSource").and_then(Value::as_bool) != Some(can_change_source) {
            book["canChangeSource"] = json!(can_change_source);
        }
        if book != original_book {
            self.store
                .write_json_ref(&reference, &book)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(self.resource_descriptor(&reference))
    }

    /// Refresh a book's processed details through the configured source
    /// engine. Source and book locks are held only while capturing and
    /// validating snapshots; the KMP/network call runs without either lock.
    pub async fn refresh_book_info(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;

        let (
            source,
            source_revision,
            book_instance_id,
            catalog_generation,
            engine_book,
            raw_chapters,
            public_latest_chapter,
        ) = {
            let _sources = self.sources_lock.lock().await;
            let sources = self.read_sources().await?;
            let first_private = self.read_private_json(&private_path).await?;
            let source_id = first_private
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
            let source_revision = self.source_revision(&source.id).await?;

            let _book = self.book_lock(book_id).await;
            let private = self
                .read_private_json(&private_path)
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            if private.get("sourceId").and_then(Value::as_str) != Some(source.id.as_str()) {
                return Err("Book source changed before metadata refresh; retry".into());
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
                source_revision,
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

        let _sources = self.sources_lock.lock().await;
        let current_source = self
            .read_sources()
            .await?
            .into_iter()
            .find(|candidate| candidate.id == source.id)
            .ok_or_else(|| "Book source was removed during metadata refresh".to_owned())?;
        if !current_source.enabled {
            return Err("Book source was disabled during metadata refresh".into());
        }
        if self.source_revision(&source.id).await? != source_revision
            || current_source.source != source.source
        {
            return Err("Book source changed during metadata refresh; refresh again".into());
        }

        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let old_private = self
            .read_private_json(&private_path)
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
        let catalog_changed_while_refreshing = old_private
            .get("chapters")
            .and_then(Value::as_array)
            .is_none_or(|chapters| chapters != &raw_chapters)
            || old_book.get("latestChapter").cloned() != public_latest_chapter;
        if catalog_changed_while_refreshing {
            if let Some(refreshed_fields) = refreshed_engine_book.as_object_mut() {
                for key in ["lastChapter", "latestChapter", "latestChapterTitle"] {
                    refreshed_fields.remove(key);
                }
            }
        }
        let display_metadata =
            crate::book_metadata::project_book_metadata(&refreshed_engine_book, &current_source);

        let mut next_private = old_private.clone();
        next_private["book"] = refreshed_engine_book;
        let mut next_book = old_book.clone();
        apply_refreshed_book_metadata(&mut next_book, &display_metadata)?;
        let shelf_ref = self.store.shelf_ref();
        let mut next_shelf = writer_guard
            .read_json_ref(&shelf_ref)
            .map_err(|error| error.to_string())?;
        crate::book_commit::upsert_shelf_entry(&mut next_shelf, book_id, &next_book)?;

        let private_bytes = serde_json::to_vec_pretty(&next_private)
            .map_err(|error| format!("Cannot encode refreshed private book: {error}"))?;
        let replacements = vec![
            crate::resource_transactions::Replacement::private_book_json(book_id, private_bytes)
                .map_err(|error| error.to_string())?,
            writer_guard
                .public_json_replacement(&book_ref, &next_book)
                .map_err(|error| error.to_string())?,
            writer_guard
                .public_json_replacement(&shelf_ref, &next_shelf)
                .map_err(|error| error.to_string())?,
        ];
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
                (
                    self.resource_descriptor(&book_ref),
                    self.resource_descriptor(&shelf_ref),
                )
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

pub(super) fn book_source_fingerprint(
    source_id: &str,
    source_definition: &Value,
    private: &Value,
    book: &Value,
) -> Result<String, String> {
    let chapters = book
        .get("chapters")
        .and_then(Value::as_array)
        .ok_or_else(|| "Book chapter directory is missing".to_owned())?
        .iter()
        .map(|chapter| {
            json!({
                "id": chapter.get("id"),
                "title": chapter.get("title"),
                "index": chapter.get("index"),
            })
        })
        .collect::<Vec<_>>();
    let identity = json!({
        "sourceId": source_id,
        "sourceDefinition": source_definition,
        "bookInstanceId": private.get("bookInstanceId"),
        "catalogGeneration": private.get("catalogGeneration"),
        "engineBook": private.get("book"),
        "rawChapters": private.get("chapters"),
        "chapters": chapters,
    });
    let bytes = serde_json::to_vec(&identity)
        .map_err(|error| format!("Cannot fingerprint book catalog: {error}"))?;
    Ok(sha256_hex(&bytes))
}

pub(super) fn apply_cached_book_metadata(
    book: &mut Value,
    metadata: &crate::book_metadata::ProcessedBookMetadata,
    source_is_imported: bool,
) -> Result<(), String> {
    crate::book_display_metadata::update_base(book, metadata)?;
    if let Some(kind) = metadata.kind.as_deref() {
        book["kind"] = json!(kind);
    }
    if let Some(word_count) = metadata.word_count.as_deref() {
        book["wordCount"] = json!(word_count);
    }
    if !metadata.source_id.is_empty() {
        book["sourceId"] = json!(metadata.source_id.as_str());
    }
    if !metadata.source_name.is_empty() {
        book["sourceName"] = json!(metadata.source_name.as_str());
    }
    if source_is_imported {
        set_optional_public_field(
            book,
            "sourceGroup",
            metadata.source_group.as_deref().map(|group| json!(group)),
        );
    } else if let Some(group) = metadata.source_group.as_deref() {
        book["sourceGroup"] = json!(group);
    }
    Ok(())
}

pub(super) fn apply_refreshed_book_metadata(
    book: &mut Value,
    metadata: &crate::book_metadata::ProcessedBookMetadata,
) -> Result<(), String> {
    crate::book_display_metadata::update_base(book, metadata)?;
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
