//! 整本书更换书源的校验和提交流程。

use super::*;

impl ApplicationService {
    /// Confirm a source replacement. Both source engine calls happen without
    /// source/book locks; the method then reacquires locks in the established
    /// order and validates the candidate's book/source/catalog snapshot again
    /// before publishing any new resources.
    pub async fn change_book_source(
        &self,
        book_id: &str,
        result_id: &str,
        confirm_missing_author: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        validate_id(result_id, "resultId")?;
        let candidate_cache = self
            .read_search_result_cache(result_id)
            .await
            .map_err(|_| "Replacement candidate is unavailable; search again".to_owned())?;
        let context = candidate_cache
            .get("replacementContext")
            .cloned()
            .ok_or_else(|| "This search result is not a replacement candidate".to_owned())?;
        if context.get("targetBookId").and_then(Value::as_str) != Some(book_id) {
            return Err("Replacement candidate belongs to a different book".into());
        }
        let expires_at_ms = context["expiresAtMs"]
            .as_u64()
            .ok_or_else(|| "Replacement candidate has invalid expiry metadata".to_owned())?;
        if now_ms() > expires_at_ms {
            return Err("Replacement candidate expired; search again".into());
        }
        if context["requiresIdentityConfirmation"].as_bool() == Some(true)
            && !confirm_missing_author
        {
            return Err(
                "Confirm that this title is the intended book because its author could not be verified".into(),
            );
        }
        let target_source_id = candidate_cache["sourceId"]
            .as_str()
            .ok_or_else(|| "Replacement candidate has no source ID".to_owned())?
            .to_owned();
        let candidate_book = candidate_cache
            .get("book")
            .cloned()
            .ok_or_else(|| "Replacement candidate has no engine result".to_owned())?;
        let search_id = context["searchId"]
            .as_str()
            .ok_or_else(|| "Replacement candidate has no search binding".to_owned())?;
        validate_id(search_id, "searchId")?;
        let search_document = self
            .read_search_document(search_id)
            .await
            .map_err(|_| "Replacement candidate search is unavailable; search again".to_owned())?;
        let belongs_to_completed_search = search_document["complete"].as_bool() == Some(true)
            && search_document["status"].as_str() == Some("completed")
            && search_document["cancelled"].as_bool() != Some(true)
            && search_document["results"]
                .as_array()
                .is_some_and(|results| {
                    results.iter().any(|result| {
                        result["resultId"].as_str() == Some(result_id)
                            && result["sourceId"].as_str() == Some(target_source_id.as_str())
                    })
                });
        if !belongs_to_completed_search {
            return Err("Replacement candidate is not part of a completed search".into());
        }

        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let (old_source_id, old_source_definition, new_source, old_book, old_title, old_author) = {
            let _sources = self.sources_lock.lock().await;
            let sources = self.read_sources().await?;
            let old_source_id = context["originalSourceId"]
                .as_str()
                .ok_or_else(|| "Replacement candidate has no original source binding".to_owned())?;
            let original_source_present = context["originalSourcePresent"]
                .as_bool()
                .ok_or_else(|| "Replacement candidate has no source-presence binding".to_owned())?;
            let old_source = sources.iter().find(|source| source.id == old_source_id);
            if original_source_present != old_source.is_some() {
                return Err(
                    "Current book source availability changed after the search; search again"
                        .into(),
                );
            }
            let old_source_definition = old_source
                .map(|source| source.source.clone())
                .unwrap_or(Value::Null);
            let original_source_fingerprint = context["originalSourceFingerprint"].as_str();
            match (
                original_source_present,
                old_source,
                original_source_fingerprint,
            ) {
                (true, Some(source), Some(expected))
                    if source_definition_fingerprint(&source.source)? == expected => {}
                (true, _, _) => {
                    return Err("Current book source changed after the search; search again".into());
                }
                (false, None, None) => {}
                (false, _, _) => {
                    return Err(
                        "Current book source availability changed after the search; search again"
                            .into(),
                    );
                }
            }
            if self.source_revision(old_source_id).await?
                != context["originalSourceRevision"].as_u64().unwrap_or(0)
            {
                return Err("Current book source changed after the search; search again".into());
            }
            let new_source = sources
                .iter()
                .find(|source| source.id == target_source_id)
                .cloned()
                .ok_or_else(|| "The replacement source was removed; search again".to_owned())?;
            if !new_source.enabled
                || crate::source_metadata::is_rss_source_metadata(&new_source.source)
            {
                return Err("Replacement source is disabled or is not a novel source".into());
            }
            if old_source_id == new_source.id {
                return Err("Choose a different source for this book".into());
            }
            if source_definition_fingerprint(&new_source.source)?
                != context["candidateSourceFingerprint"]
                    .as_str()
                    .unwrap_or_default()
            {
                return Err("Replacement source changed after the search; search again".into());
            }
            let _book = self.book_lock(book_id).await;
            let old_private = self
                .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            let canonical_book = old_private
                .get("book")
                .filter(|value| value.is_object())
                .ok_or_else(|| "Private book is missing canonical engine metadata".to_owned())?;
            let old_title = text_at(canonical_book, &["name", "title"])
                .unwrap_or_default()
                .trim()
                .to_owned();
            let old_author = text_at(canonical_book, &["author"])
                .unwrap_or_default()
                .trim()
                .to_owned();
            let old_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            let actual_fingerprint = book_source_fingerprint(
                old_source_id,
                &old_source_definition,
                &old_private,
                &old_book,
            )?;
            if actual_fingerprint != context["catalogFingerprint"].as_str().unwrap_or_default()
                || old_private
                    .get("catalogGeneration")
                    .cloned()
                    .unwrap_or(Value::Null)
                    != context["catalogGeneration"]
            {
                return Err("Book catalog changed after the search; search again".into());
            }
            (
                old_source_id.to_owned(),
                old_source_definition,
                new_source,
                old_book,
                old_title,
                old_author,
            )
        };

        let raw_new_book = if candidate_book.is_object() {
            candidate_book
        } else {
            return Err("Replacement source returned invalid book metadata".into());
        };
        let rss = crate::rss::is_legacy_rss_source(&new_source.source);
        let engine_book = self
            .executor
            .execute(engine_request(
                if rss { "rssBookInfo" } else { "bookInfo" },
                &new_source.source,
                None,
                None,
                Some(raw_new_book.clone()),
                None,
                None,
            ))
            .await?;
        if !engine_book.is_object() {
            return Err("Replacement source returned invalid book details".into());
        }
        let target_title = text_at(&engine_book, &["name", "title"])
            .ok_or_else(|| "Replacement source did not confirm the book title".to_owned())?;
        if normalize_identity(&target_title) != normalize_identity(&old_title) {
            return Err("Replacement source book title does not match the current book".into());
        }
        let engine_author = text_at(&engine_book, &["author"]);
        if !old_author.trim().is_empty()
            && engine_author
                .as_deref()
                .is_some_and(|author| !author.trim().is_empty())
            && normalize_identity(engine_author.as_deref().unwrap_or_default())
                != normalize_identity(&old_author)
        {
            return Err("Replacement source author does not match the current book".into());
        }
        let author_unverified = old_author.trim().is_empty()
            || engine_author
                .as_deref()
                .is_none_or(|author| author.trim().is_empty());
        if author_unverified && !confirm_missing_author {
            return Err(
                "Confirm that this title is the intended book because its author could not be verified".into(),
            );
        }
        // Keep known metadata when the new source omits it; the source rule
        // engine remains the authority when it supplies a new value.
        let target_author = engine_author.unwrap_or(old_author.clone());
        let display_metadata =
            crate::book_metadata::project_book_metadata(&engine_book, &new_source);
        let raw_chapters = self
            .executor
            .execute(engine_request(
                if rss { "rssChapters" } else { "chapters" },
                &new_source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?
            .as_array()
            .cloned()
            .ok_or_else(|| "Replacement source returned an invalid chapter catalog".to_owned())?;
        if raw_chapters.is_empty() {
            return Err(
                "Replacement source returned no chapters; the current book was kept".into(),
            );
        }
        crate::catalog::reconcile_catalog(
            book_id,
            &[],
            &[],
            &raw_chapters,
            &ProgressSummary::default(),
            &HashSet::new(),
            now_ms(),
        )
        .map_err(|error| format!("Replacement chapter catalog is invalid: {error}"))?;

        let generation = uuid::Uuid::new_v4().simple().to_string();
        let new_chapters = raw_chapters
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                let raw_url =
                    text_at(raw, &["url", "chapterUrl"]).unwrap_or_else(|| format!("@{index}"));
                ChapterDescriptor {
                    id: chapter_id_for_generation(book_id, &generation, &raw_url),
                    title: text_at(raw, &["title", "chapterName", "name"])
                        .unwrap_or_else(|| format!("Chapter {}", index + 1)),
                    index,
                    // A new source always begins uncached, even if it returns
                    // URLs that happen to match the previous source.
                    resource: None,
                }
            })
            .collect::<Vec<_>>();
        let mut new_ids = HashSet::with_capacity(new_chapters.len());
        if new_chapters
            .iter()
            .any(|chapter| !new_ids.insert(chapter.id.clone()))
        {
            return Err("Replacement catalog produces duplicate chapter IDs".into());
        }

        let old_chapters = serde_json::from_value::<Vec<ChapterDescriptor>>(
            old_book
                .get("chapters")
                .cloned()
                .ok_or_else(|| "Current book chapter directory is missing".to_owned())?,
        )
        .map_err(|error| format!("Cannot read current chapter directory: {error}"))?;
        let old_titles = old_chapters
            .iter()
            .map(|chapter| chapter.title.clone())
            .collect::<Vec<_>>();
        let new_titles = new_chapters
            .iter()
            .map(|chapter| chapter.title.clone())
            .collect::<Vec<_>>();
        let bookmarks_ref = crate::reading_tools::bookmarks_resource(&self.store).await?;
        let progress_ref = self
            .store
            .progress_ref(book_id)
            .map_err(|error| error.to_string())?;
        let private_path = Path::new("books").join(format!("{book_id}.json"));

        let _sources = self.sources_lock.lock().await;
        let latest_sources = self.read_sources().await?;
        let latest_old_source = latest_sources
            .iter()
            .find(|source| source.id == old_source_id);
        let latest_new_source = latest_sources
            .iter()
            .find(|source| source.id == new_source.id)
            .ok_or_else(|| "Replacement source was removed during replacement".to_owned())?;
        let original_source_present = context["originalSourcePresent"].as_bool().unwrap_or(false);
        let source_presence_unchanged = latest_old_source.is_some() == original_source_present;
        let source_definition_unchanged = match (original_source_present, latest_old_source) {
            (true, Some(source)) => source.source == old_source_definition,
            (false, None) => true,
            _ => false,
        };
        if !source_presence_unchanged
            || !source_definition_unchanged
            || self.source_revision(&old_source_id).await?
                != context["originalSourceRevision"].as_u64().unwrap_or(0)
            || latest_new_source.source != new_source.source
            || !latest_new_source.enabled
        {
            return Err("A source changed during replacement; search again".into());
        }
        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let latest_private = self
            .read_private_json(&private_path)
            .await
            .map_err(|_| format!("Book '{book_id}' was removed during replacement"))?;
        let latest_book = writer_guard
            .read_json_ref(&book_ref)
            .map_err(|_| format!("Book '{book_id}' was removed during replacement"))?;
        if latest_private.get("sourceId").and_then(Value::as_str) != Some(old_source_id.as_str())
            || book_source_fingerprint(
                &old_source_id,
                &old_source_definition,
                &latest_private,
                &latest_book,
            )? != context["catalogFingerprint"].as_str().unwrap_or_default()
        {
            return Err("Book source or catalog changed during replacement; search again".into());
        }

        // Progress may have advanced while the source engine was resolving
        // the new metadata. Re-read it under the commit lock and remap the
        // latest value so a slow network request never rolls the reader back.
        let old_private = latest_private;
        let old_book = latest_book;
        let summary_progress = serde_json::from_value::<ProgressSummary>(
            old_book
                .get("progress")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .unwrap_or_default();
        let progress_document = writer_guard
            .read_json_ref(&progress_ref)
            .ok()
            .and_then(|value| serde_json::from_value::<ProgressDocument>(value).ok())
            .filter(|progress| progress.book_id == book_id);
        let old_progress = progress_document
            .filter(|document| document.updated_at_ms > summary_progress.updated_at_ms)
            .map(|document| ProgressSummary {
                chapter_id: document.chapter_id,
                chapter_index: document.chapter_index,
                offset: document.offset,
                updated_at_ms: document.updated_at_ms,
            })
            .unwrap_or(summary_progress);
        let progress_old_index = old_progress
            .chapter_id
            .as_deref()
            .and_then(|id| old_chapters.iter().position(|chapter| chapter.id == id))
            .unwrap_or_else(|| {
                old_progress
                    .chapter_index
                    .min(old_chapters.len().saturating_sub(1))
            });
        let progress_title = old_chapters
            .get(progress_old_index)
            .map(|chapter| chapter.title.as_str())
            .unwrap_or_default();
        let old_media_type = old_book
            .get("mediaType")
            .and_then(Value::as_str)
            .or_else(|| crate::source_metadata::media_type(&old_source_definition));
        let media_type = crate::source_metadata::media_type(&new_source.source);
        let media_type_changed = old_media_type != media_type
            || (old_book.get("mediaType").is_none() && media_type.is_some());
        let mapped_index = crate::book_commit::unique_chapter_title_index(
            progress_title,
            &old_titles,
            &new_titles,
        );
        let (progress_index, mapped_offset) = match mapped_index {
            Some(index) => (index, old_progress.offset),
            None => (progress_old_index.min(new_chapters.len() - 1), 0),
        };
        let progress_offset = if media_type_changed { 0 } else { mapped_offset };
        let progress = ProgressSummary {
            chapter_id: Some(new_chapters[progress_index].id.clone()),
            chapter_index: progress_index,
            offset: progress_offset,
            updated_at_ms: now_ms().max(old_progress.updated_at_ms.saturating_add(1)),
        };
        let moved_progress = progress_index != old_progress.chapter_index
            || old_progress.chapter_id.as_deref() != progress.chapter_id.as_deref()
            || progress_offset != old_progress.offset;
        let mut next_private = old_private.clone();
        next_private["sourceId"] = json!(new_source.id);
        next_private["book"] = engine_book.clone();
        next_private["chapters"] = json!(raw_chapters);
        next_private["catalogGeneration"] = json!(generation);
        let mut next_book = old_book.clone();
        let mut display_base = display_metadata.clone();
        if display_base.author.is_none() {
            display_base.author = Some(target_author.clone());
        }
        crate::book_display_metadata::update_base(&mut next_book, &display_base)?;
        if let Some(kind) = display_metadata.kind {
            next_book["kind"] = json!(kind);
        }
        if let Some(word_count) = display_metadata.word_count {
            next_book["wordCount"] = json!(word_count);
        }
        next_book["sourceId"] = json!(display_metadata.source_id);
        next_book["sourceName"] = json!(display_metadata.source_name);
        set_optional_public_field(
            &mut next_book,
            "mediaType",
            media_type.map(|value| json!(value)),
        );
        set_optional_public_field(
            &mut next_book,
            "sourceGroup",
            display_metadata.source_group.map(|group| json!(group)),
        );
        next_book["chapterCount"] = json!(new_chapters.len());
        next_book["latestChapter"] = new_chapters
            .last()
            .map(|chapter| json!(chapter.title))
            .unwrap_or(Value::Null);
        next_book["chapters"] =
            serde_json::to_value(&new_chapters).map_err(|error| error.to_string())?;
        next_book["progress"] =
            serde_json::to_value(&progress).map_err(|error| error.to_string())?;
        let next_progress = serde_json::to_value(ProgressDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            book_id: book_id.to_owned(),
            chapter_id: progress.chapter_id.clone(),
            chapter_index: progress.chapter_index,
            offset: progress.offset,
            updated_at_ms: progress.updated_at_ms,
        })
        .map_err(|error| error.to_string())?;

        let shelf_ref = self.store.shelf_ref();
        let mut next_shelf = writer_guard
            .read_json_ref(&shelf_ref)
            .map_err(|error| error.to_string())?;
        crate::book_commit::upsert_shelf_entry(&mut next_shelf, book_id, &next_book)?;

        let mut next_bookmarks = writer_guard
            .read_json_ref(&bookmarks_ref)
            .map_err(|error| error.to_string())?;
        let bookmark_counts = crate::book_commit::migrate_bookmarks(
            &mut next_bookmarks,
            book_id,
            &old_titles,
            &new_titles,
        )?;

        let private_bytes = serde_json::to_vec_pretty(&next_private)
            .map_err(|error| format!("Cannot encode replacement private book: {error}"))?;
        let private_replacement =
            crate::resource_transactions::Replacement::private_book_json(book_id, private_bytes)
                .map_err(|error| error.to_string())?;
        let replacements = vec![
            private_replacement,
            writer_guard
                .public_json_replacement(&book_ref, &next_book)
                .map_err(|error| error.to_string())?,
            writer_guard
                .public_json_replacement(&progress_ref, &next_progress)
                .map_err(|error| error.to_string())?,
            writer_guard
                .public_json_replacement(&shelf_ref, &next_shelf)
                .map_err(|error| error.to_string())?,
            writer_guard
                .public_json_replacement(&bookmarks_ref, &next_bookmarks)
                .map_err(|error| error.to_string())?,
        ];
        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "change-book-source",
                replacements,
                Vec::new(),
                &writer_guard,
            )?;
            transaction.commit()
        })
        .await;
        match transaction_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Err(format_source_change_transaction_error(
                    error.commit_state(),
                    error.recovery_required_flag(),
                    &error.to_string(),
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Source replacement commit status is unknown because its file worker stopped before reporting a result. Do not retry yet; restart the application to recover the transaction first. Details: {error}"
                ));
            }
        }

        Ok(json!({
            "book": self.resource_descriptor(&book_ref),
            "shelf": self.resource_descriptor(&shelf_ref),
            "progress": serde_json::to_value(&progress).map_err(|error| error.to_string())?,
            "movedProgress": moved_progress,
            "bookmarks": {
                "resource": self.resource_descriptor(&bookmarks_ref),
                "migratedCount": bookmark_counts.migrated,
                "orphanedCount": bookmark_counts.orphaned,
            },
        }))
    }
}
