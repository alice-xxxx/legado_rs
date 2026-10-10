//! 单个章节更换书源的流程。

use super::*;

impl ApplicationService {
    /// Replace only one cached chapter body using a source candidate for this
    /// book. The public chapter ID remains stable; private source-engine data
    /// stays in Rust and the chapter title must identify exactly one source
    /// chapter before any resource is written.
    pub async fn change_chapter_source(
        &self,
        book_id: &str,
        chapter_id: &str,
        result_id: &str,
        confirm_missing_author: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let result: Result<Value, String> = async {
        validate_id(book_id, "bookId")?;
        validate_id(chapter_id, "chapterId")?;
        validate_id(result_id, "resultId")?;

        let candidate = self
            .read_search_result_cache(result_id)
            .await
            .map_err(|_| "Replacement candidate is unavailable; search again".to_owned())?;
        let context = candidate
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
        let replacement_source_id = candidate["sourceId"]
            .as_str()
            .ok_or_else(|| "Replacement candidate has no source ID".to_owned())?
            .to_owned();
        let replacement_book = candidate
            .get("book")
            .filter(|book| book.is_object())
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
                            && result["sourceId"].as_str() == Some(replacement_source_id.as_str())
                    })
                });
        if !belongs_to_completed_search {
            return Err("Replacement candidate is not part of a completed search".into());
        }
        if candidate["sourceRevision"].as_u64().is_none()
            || candidate["sourceDefinitionFingerprint"]
                .as_str()
                .is_none_or(str::is_empty)
        {
            return Err("Replacement candidate has invalid source metadata".into());
        }
        let candidate_source_revision = candidate["sourceRevision"].as_u64().unwrap_or_default();
        let candidate_source_fingerprint = candidate["sourceDefinitionFingerprint"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if context["candidateSourceFingerprint"].as_str()
            != Some(candidate_source_fingerprint.as_str())
        {
            return Err("Replacement candidate source binding is invalid".into());
        }

        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        // Acquire the per-chapter lock before source/book locks. Chapter
        // preparation uses the same order, so an in-flight download cannot
        // overwrite this replacement after it commits.
        let _chapter = self.chapter_lock(book_id, chapter_id).await;
        let (
            original_source_id,
            original_source_definition,
            original_source_revision,
            replacement_source,
            old_chapter_raw,
            old_chapter_title,
            old_book_title,
            old_author,
            catalog_fingerprint,
            catalog_generation,
            book_instance_id,
        ) = {
            let _sources = self.sources_lock.lock().await;
            let sources = self.read_sources().await?;
            let original_source_id = context["originalSourceId"]
                .as_str()
                .ok_or_else(|| "Replacement candidate has no original source binding".to_owned())?
                .to_owned();
            let original_source_present = context["originalSourcePresent"]
                .as_bool()
                .ok_or_else(|| "Replacement candidate has no source-presence binding".to_owned())?;
            let original_source = sources.iter().find(|source| source.id == original_source_id);
            if original_source_present != original_source.is_some() {
                return Err(
                    "Current book source availability changed after the search; search again".into(),
                );
            }
            let original_source_definition = original_source
                .map(|source| source.source.clone())
                .unwrap_or(Value::Null);
            match (
                original_source_present,
                original_source,
                context["originalSourceFingerprint"].as_str(),
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
            let original_source_revision = self.source_revision(&original_source_id).await?;
            if original_source_revision
                != context["originalSourceRevision"].as_u64().unwrap_or_default()
            {
                return Err("Current book source changed after the search; search again".into());
            }
            let replacement_source = sources
                .iter()
                .find(|source| source.id == replacement_source_id)
                .cloned()
                .ok_or_else(|| "The replacement source was removed; search again".to_owned())?;
            if !replacement_source.enabled
                || crate::source_metadata::is_rss_source_metadata(&replacement_source.source)
            {
                return Err("Replacement source is disabled or is not a novel source".into());
            }
            if crate::source_metadata::media_type(&replacement_source.source).is_some() {
                return Err(
                    "Single-chapter replacement is not supported for audio or video books".into(),
                );
            }
            if original_source_id == replacement_source.id {
                return Err("Choose a different source for this chapter".into());
            }
            if source_definition_fingerprint(&replacement_source.source)?
                != candidate_source_fingerprint
                || self.source_revision(&replacement_source_id).await? != candidate_source_revision
            {
                return Err("Replacement source changed after the search; search again".into());
            }

            let _book = self.book_lock(book_id).await;
            let old_private = self
                .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            let old_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| format!("Book '{book_id}' was removed"))?;
            if matches!(
                old_book.get("mediaType").and_then(Value::as_str),
                Some("audio" | "video")
            ) || crate::source_metadata::media_type(&original_source_definition).is_some()
            {
                return Err(
                    "Single-chapter replacement is not supported for audio or video books".into(),
                );
            }
            let actual_fingerprint = book_source_fingerprint(
                &original_source_id,
                &original_source_definition,
                &old_private,
                &old_book,
            )?;
            if actual_fingerprint != context["catalogFingerprint"].as_str().unwrap_or_default()
                || old_private
                    .get("catalogGeneration")
                    .cloned()
                    .unwrap_or(Value::Null)
                    != context["catalogGeneration"]
                || old_private
                    .get("bookInstanceId")
                    .and_then(Value::as_str)
                    != context["bookInstanceId"].as_str()
            {
                return Err("Book source or catalog changed after the search; search again".into());
            }
            let old_book_data = old_private
                .get("book")
                .filter(|value| value.is_object())
                .ok_or_else(|| "Private book is missing canonical engine metadata".to_owned())?;
            let old_book_title = text_at(old_book_data, &["name", "title"])
                .unwrap_or_default()
                .trim()
                .to_owned();
            let old_author = text_at(old_book_data, &["author"])
                .unwrap_or_default()
                .trim()
                .to_owned();
            let raw_chapters = old_private
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
            let descriptors = old_book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            if raw_chapters.len() != descriptors.len() {
                return Err(
                    "Book chapter directory is out of sync; refresh it before changing this chapter".into(),
                );
            }
            let index = descriptors
                .iter()
                .position(|chapter| chapter["id"].as_str() == Some(chapter_id))
                .ok_or_else(|| "Chapter is no longer in the book directory".to_owned())?;
            let old_chapter_title = descriptors[index]["title"]
                .as_str()
                .filter(|title| !normalize_identity(title).is_empty())
                .ok_or_else(|| "Chapter title is unavailable for source matching".to_owned())?
                .to_owned();
            let old_chapter_raw = raw_chapters[index].clone();
            let catalog_fingerprint = context["catalogFingerprint"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let catalog_generation = old_private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null);
            let book_instance_id = old_private
                .get("bookInstanceId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            (
                original_source_id,
                original_source_definition,
                original_source_revision,
                replacement_source,
                old_chapter_raw,
                old_chapter_title,
                old_book_title,
                old_author,
                catalog_fingerprint,
                catalog_generation,
                book_instance_id,
            )
        };

        let replacement_engine_book = self
            .executor
            .execute(engine_request(
                "bookInfo",
                &replacement_source.source,
                None,
                None,
                Some(replacement_book),
                None,
                None,
            ))
            .await?;
        if !replacement_engine_book.is_object() {
            return Err("Replacement source returned invalid book details".into());
        }
        let replacement_title = text_at(&replacement_engine_book, &["name", "title"])
            .ok_or_else(|| "Replacement source did not confirm the book title".to_owned())?;
        if normalize_identity(&replacement_title) != normalize_identity(&old_book_title) {
            return Err("Replacement source book title does not match the current book".into());
        }
        let replacement_author = text_at(&replacement_engine_book, &["author"]);
        if !old_author.is_empty()
            && replacement_author
                .as_deref()
                .is_some_and(|author| !author.trim().is_empty())
            && normalize_identity(replacement_author.as_deref().unwrap_or_default())
                != normalize_identity(&old_author)
        {
            return Err("Replacement source author does not match the current book".into());
        }
        let author_unverified = old_author.is_empty()
            || replacement_author
                .as_deref()
                .is_none_or(|author| author.trim().is_empty());
        if author_unverified && !confirm_missing_author {
            return Err(
                "Confirm that this title is the intended book because its author could not be verified".into(),
            );
        }
        let replacement_chapters = self
            .executor
            .execute(engine_request(
                "chapters",
                &replacement_source.source,
                None,
                None,
                Some(replacement_engine_book.clone()),
                None,
                None,
            ))
            .await?
            .as_array()
            .cloned()
            .ok_or_else(|| "Replacement source returned an invalid chapter catalog".to_owned())?;
        let normalized_target_title = normalize_identity(&old_chapter_title);
        let matching_indexes = replacement_chapters
            .iter()
            .enumerate()
            .filter_map(|(index, chapter)| {
                text_at(chapter, &["title", "chapterName", "name"])
                    .filter(|title| normalize_identity(title) == normalized_target_title)
                    .map(|_| index)
            })
            .collect::<Vec<_>>();
        let replacement_index = match matching_indexes.as_slice() {
            [index] => *index,
            [] => {
                return Err(format!(
                    "Replacement source has no chapter matching '{old_chapter_title}'"
                ));
            }
            _ => {
                return Err(format!(
                    "Replacement source has multiple chapters matching '{old_chapter_title}'; choose a unique chapter title"
                ));
            }
        };
        let replacement_chapter = replacement_chapters[replacement_index].clone();
        let content_base_url = text_at(&replacement_chapter, &["url", "chapterUrl"])
            .or_else(|| text_at(&replacement_engine_book, &["bookUrl", "url"]));
        let next_chapter_url = replacement_chapters
            .get(replacement_index + 1)
            .and_then(|chapter| text_at(chapter, &["url", "chapterUrl"]));
        let replacement_content = self
            .executor
            .execute(engine_request(
                "content",
                &replacement_source.source,
                None,
                None,
                Some(replacement_engine_book),
                Some(replacement_chapter),
                next_chapter_url,
            ))
            .await?;
        let replacement_content = replacement_content
            .as_str()
            .ok_or_else(|| "Source engine returned non-text chapter content".to_owned())?;
        if replacement_content.trim().is_empty() {
            return Err(
                "Source engine returned empty chapter content; the existing chapter cache was preserved"
                    .to_owned(),
            );
        }
        let (chapter_ref, chapter_replacement, content_format) = if looks_like_html(replacement_content) {
            let (chapter_ref, document) = self
                .store
                .online_chapter_rich_text_document(
                    book_id,
                    chapter_id,
                    replacement_content,
                    content_base_url.as_deref(),
                )
                .map_err(|error| error.to_string())?;
            let bytes = serde_json::to_vec_pretty(&document)
                .map_err(|error| format!("Cannot encode replacement chapter rich text: {error}"))?;
            let replacement = crate::resource_transactions::Replacement::public_json(
                chapter_ref.clone(),
                bytes,
            )
            .map_err(|error| error.to_string())?;
            (chapter_ref, replacement, "richText")
        } else {
            let (chapter_ref, document) = self
                .store
                .chapter_text_document(book_id, chapter_id, replacement_content)
                .map_err(|error| error.to_string())?;
            let bytes = serde_json::to_vec_pretty(&document)
                .map_err(|error| format!("Cannot encode replacement chapter text: {error}"))?;
            let replacement = crate::resource_transactions::Replacement::public_json(
                chapter_ref.clone(),
                bytes,
            )
            .map_err(|error| error.to_string())?;
            (chapter_ref, replacement, "text")
        };

        let _sources = self.sources_lock.lock().await;
        let latest_sources = self.read_sources().await?;
        let latest_original = latest_sources
            .iter()
            .find(|source| source.id == original_source_id);
        let original_presence_unchanged =
            latest_original.is_some() == context["originalSourcePresent"].as_bool().unwrap_or(false);
        let original_definition_unchanged = match (
            context["originalSourcePresent"].as_bool().unwrap_or(false),
            latest_original,
        ) {
            (true, Some(source)) => source.source == original_source_definition,
            (false, None) => true,
            _ => false,
        };
        let latest_replacement = latest_sources
            .iter()
            .find(|source| source.id == replacement_source.id)
            .ok_or_else(|| "Replacement source was removed during chapter replacement".to_owned())?;
        if !original_presence_unchanged
            || !original_definition_unchanged
            || self.source_revision(&original_source_id).await? != original_source_revision
            || latest_replacement.source != replacement_source.source
            || !latest_replacement.enabled
            || crate::source_metadata::is_rss_source_metadata(&latest_replacement.source)
            || self.source_revision(&replacement_source_id).await? != candidate_source_revision
        {
            return Err("A source changed during chapter replacement; search again".into());
        }

        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let latest_private = self
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await
            .map_err(|_| format!("Book '{book_id}' was removed during chapter replacement"))?;
        let latest_book = writer_guard
            .read_json_ref(&book_ref)
            .map_err(|_| format!("Book '{book_id}' was removed during chapter replacement"))?;
        if latest_private.get("sourceId").and_then(Value::as_str)
            != Some(original_source_id.as_str())
            || latest_private
                .get("bookInstanceId")
                .and_then(Value::as_str)
                != Some(book_instance_id.as_str())
            || latest_private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null)
                != catalog_generation
            || book_source_fingerprint(
                &original_source_id,
                &original_source_definition,
                &latest_private,
                &latest_book,
            )? != catalog_fingerprint
        {
            return Err("Book source or catalog changed during chapter replacement; search again".into());
        }
        let latest_raw_chapters = latest_private
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
        let mut next_book = latest_book;
        let latest_descriptors = next_book
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
        if latest_raw_chapters.len() != latest_descriptors.len() {
            return Err("Book chapter directory is out of sync; refresh it before retrying".into());
        }
        let index = latest_descriptors
            .iter()
            .position(|chapter| chapter["id"].as_str() == Some(chapter_id))
            .ok_or_else(|| "Chapter is no longer in the book directory".to_owned())?;
        if latest_raw_chapters[index] != old_chapter_raw
            || latest_descriptors[index]["title"].as_str() != Some(old_chapter_title.as_str())
        {
            return Err("Chapter changed during source replacement; search again".into());
        }
        next_book["chapters"][index]["resource"] = serde_json::to_value(
            crate::models::ChapterResourceDescriptor::for_chapter(&chapter_ref, content_format),
        )
        .map_err(|error| format!("Cannot encode replaced chapter descriptor: {error}"))?;
        let replacements = vec![
            chapter_replacement,
            writer_guard
                .public_json_replacement(&book_ref, &next_book)
                .map_err(|error| error.to_string())?,
        ];
        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "change-chapter-source",
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
                let warning = Some(format!(
                    "Chapter body replacement has {} commit status. {} Details: {error}",
                    match commit_state {
                        crate::resource_transactions::CommitState::NotCommitted => "no",
                        crate::resource_transactions::CommitState::Committed => "a committed",
                        crate::resource_transactions::CommitState::Indeterminate => "an unknown",
                    },
                    if recovery_required {
                        "Restart the application before retrying."
                    } else if commit_state == crate::resource_transactions::CommitState::NotCommitted {
                        "No changes were committed; it is safe to retry."
                    } else {
                        "The replacement is active; restart the application before further writes."
                    }
                ));
                (commit_state, recovery_required, warning)
            }
            Err(error) => (
                crate::resource_transactions::CommitState::Indeterminate,
                true,
                Some(format!(
                    "Chapter body replacement status is unknown. Do not retry; restart the application to recover the transaction first. Details: {error}"
                )),
            ),
        };
        let commit_state_json = match commit_state {
            crate::resource_transactions::CommitState::NotCommitted => "notCommitted",
            crate::resource_transactions::CommitState::Committed => "committed",
            crate::resource_transactions::CommitState::Indeterminate => "indeterminate",
        };
        let mut outcome = json!({
            "commitState": commit_state_json,
            "recoveryRequired": recovery_required,
            "warning": warning,
            "chapterId": chapter_id,
        });
        if commit_state == crate::resource_transactions::CommitState::Committed {
            outcome["book"] = self.resource_descriptor(&book_ref);
        }
        Ok(outcome)
        }
        .await;
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                let mut outcome = book_display_metadata_commit_outcome(
                    crate::resource_transactions::CommitState::NotCommitted,
                    false,
                    None,
                    None,
                );
                outcome["error"] = json!(error);
                outcome["chapterId"] = json!(chapter_id);
                Ok(outcome)
            }
        }
    }
}
