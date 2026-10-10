//! 阅读进度、章节缓存和阅读设置的持久化。

use super::*;

impl ApplicationService {
    pub async fn save_progress(&self, book_id: &str, progress: Value) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let _book_lock = self.book_lock(book_id).await;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let chapter_id = match progress.get("chapterId") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => return Err("Progress chapterId must be a string or null".into()),
        };
        let chapter_index = strict_unsigned_field(&progress, "chapterIndex")?
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| "Progress chapterIndex must be a non-negative integer".to_owned())?;
        let offset = strict_unsigned_field(&progress, "offset")?
            .ok_or_else(|| "Progress offset must be a non-negative integer".to_owned())?;
        let updated_at_ms = strict_unsigned_field(&progress, "updatedAtMs")?.unwrap_or_else(now_ms);
        if let Some(chapter_id) = chapter_id.as_deref() {
            validate_id(chapter_id, "chapterId")?;
        }
        let progress_summary = json!({
            "chapterId": chapter_id,
            "chapterIndex": chapter_index,
            "offset": offset,
            "updatedAtMs": updated_at_ms,
        });
        let chapter_id_for_book = chapter_id.clone();
        self.store
            .update_json_ref(&book_ref, move |mut book| {
                let chapter_count = book
                    .get("chapterCount")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize;
                if chapter_index >= chapter_count {
                    return Err("Progress chapterIndex is outside the book chapter list".into());
                }
                if let Some(chapter_id) = chapter_id_for_book.as_deref() {
                    let belongs_to_book = book
                        .get("chapters")
                        .and_then(Value::as_array)
                        .and_then(|chapters| chapters.get(chapter_index))
                        .and_then(|chapter| chapter.get("id"))
                        .and_then(Value::as_str)
                        == Some(chapter_id);
                    if !belongs_to_book {
                        return Err("Progress chapterId does not match chapterIndex".into());
                    }
                }
                book["progress"] = progress_summary;
                Ok(book)
            })
            .await
            .map_err(|error| error.to_string())?;
        let record = ProgressDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            book_id: book_id.to_owned(),
            chapter_id,
            chapter_index,
            offset,
            updated_at_ms,
        };
        let progress_ref = self
            .store
            .progress_ref(book_id)
            .map_err(|error| error.to_string())?;
        let value = serde_json::to_value(record).map_err(|error| error.to_string())?;
        self.store
            .write_json_ref(&progress_ref, &value)
            .await
            .map_err(|error| error.to_string())?;
        let shelf = self
            .store
            .read_json_ref(&self.store.shelf_ref())
            .await
            .map_err(|error| error.to_string())?;
        let is_on_shelf = shelf
            .get("books")
            .and_then(Value::as_array)
            .ok_or_else(|| "Invalid shelf JSON: books must be an array".to_owned())?
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(book_id));
        if is_on_shelf {
            self.upsert_shelf(book_id).await?;
        }
        Ok(self.resource_descriptor(&book_ref))
    }

    pub async fn reset_book_progress(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let result: Result<Value, String> = async {
        validate_id(book_id, "bookId")?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let progress_ref = self
            .store
            .progress_ref(book_id)
            .map_err(|error| error.to_string())?;
        let shelf_ref = self.store.shelf_ref();

        // Keep the same source → book → resource-writer order used by catalog
        // commits so a reset cannot deadlock with a concurrent source change.
        let _sources = self.sources_lock.lock().await;
        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;

        let mut book = writer_guard
            .read_json_ref(&book_ref)
            .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
        if book.get("id").and_then(Value::as_str) != Some(book_id) {
            return Err("Book resource ID does not match its catalog path".into());
        }
        let mut shelf = writer_guard
            .read_json_ref(&shelf_ref)
            .map_err(|error| error.to_string())?;
        let listed_on_shelf = shelf
            .get("books")
            .and_then(Value::as_array)
            .ok_or_else(|| "Invalid shelf JSON: books must be an array".to_owned())?
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(book_id));
        if !listed_on_shelf {
            return Err(format!("Book '{book_id}' is no longer on the shelf"));
        }

        book["progress"] = serde_json::to_value(ProgressSummary::default())
            .map_err(|error| format!("Cannot encode reset progress: {error}"))?;
        crate::book_commit::upsert_shelf_entry(&mut shelf, book_id, &book)?;

        let replacements = vec![
            writer_guard
                .public_json_replacement(&book_ref, &book)
                .map_err(|error| error.to_string())?,
            crate::resource_transactions::Replacement::delete_progress_json(progress_ref)
                .map_err(|error| error.to_string())?,
            writer_guard
                .public_json_replacement(&shelf_ref, &shelf)
                .map_err(|error| error.to_string())?,
        ];
        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "reset-book-progress",
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
                        if recovery_required => Some(format!(
                            "Progress reset was not committed, but rollback is incomplete and resource files may be mixed. Restart before retrying. Details: {error}"
                        )),
                    crate::resource_transactions::CommitState::NotCommitted => Some(format!(
                        "Progress reset was not committed. Changes were not saved and it is safe to retry. Details: {error}"
                    )),
                    crate::resource_transactions::CommitState::Committed => Some(format!(
                        "Progress reset was committed, but journal recovery is pending. Restart before further writes. Details: {error}"
                    )),
                    crate::resource_transactions::CommitState::Indeterminate => Some(format!(
                        "Progress reset status is unknown. Do not retry; restart the application to recover the transaction first. Details: {error}"
                    )),
                };
                (commit_state, recovery_required, warning)
            }
            Err(error) => (
                crate::resource_transactions::CommitState::Indeterminate,
                true,
                Some(format!(
                    "Progress reset status is unknown. Do not retry; restart the application to recover the transaction first. Details: {error}"
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
                Ok(outcome)
            }
        }
    }

    pub async fn clear_book_chapter_cache(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let result: Result<Value, String> = async {
        validate_id(book_id, "bookId")?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let private_path = Path::new("books").join(format!("{book_id}.json"));

        // Take an unlocked catalog snapshot first, then acquire every chapter
        // lock before the source/book/writer locks. prepare_chapters holds a
        // chapter lock across its fetch and publish, so this prevents an
        // in-flight fetch from restoring a cache after the clear commits.
        let initial_book = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
        let initial_private = self.read_private_json(&private_path).await?;
        if initial_book.get("id").and_then(Value::as_str) != Some(book_id)
            || initial_book.get("canChangeSource").and_then(Value::as_bool) != Some(true)
            || initial_book
                .get("sourceId")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || initial_private
                .get("sourceId")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || initial_private.get("sourceId") != initial_book.get("sourceId")
        {
            return Err("Chapter cache clearing is available only for online books".into());
        }
        let initial_descriptors = initial_book
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
        let initial_raw_chapters = initial_private
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
        if initial_descriptors.len() != initial_raw_chapters.len()
            || initial_private
                .get("book")
                .is_none_or(|engine_book| !engine_book.is_object())
        {
            return Err("Book source and chapter directory are out of sync".into());
        }
        let mut initial_chapter_ids = Vec::with_capacity(initial_descriptors.len());
        let mut unique_chapter_ids = HashSet::with_capacity(initial_descriptors.len());
        for chapter in initial_descriptors {
            let chapter_id = chapter
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "Chapter ID missing".to_owned())?;
            validate_id(chapter_id, "chapterId")?;
            if !unique_chapter_ids.insert(chapter_id.to_owned()) {
                return Err("Book chapter directory contains duplicate chapter IDs".into());
            }
            initial_chapter_ids.push(chapter_id.to_owned());
        }
        let initial_catalog_generation = initial_private
            .get("catalogGeneration")
            .cloned()
            .unwrap_or(Value::Null);
        let initial_source_id = initial_private
            .get("sourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| "Book source ID is missing".to_owned())?
            .to_owned();

        let mut chapter_guards = Vec::with_capacity(initial_chapter_ids.len());
        for chapter_id in &initial_chapter_ids {
            chapter_guards.push(self.chapter_lock(book_id, chapter_id).await);
        }
        let _sources = self.sources_lock.lock().await;
        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;

        let latest_private = self
            .read_private_json(&private_path)
            .await
            .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
        let mut book = writer_guard
            .read_json_ref(&book_ref)
            .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
        if book.get("id").and_then(Value::as_str) != Some(book_id)
            || book.get("canChangeSource").and_then(Value::as_bool) != Some(true)
            || book.get("sourceId").and_then(Value::as_str) != Some(initial_source_id.as_str())
            || latest_private.get("sourceId").and_then(Value::as_str)
                != Some(initial_source_id.as_str())
            || latest_private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null)
                != initial_catalog_generation
        {
            let mut outcome = book_display_metadata_commit_outcome(
                crate::resource_transactions::CommitState::NotCommitted,
                false,
                None,
                None,
            );
            outcome["error"] = json!("Book source or catalog changed; no cache was cleared. Retry the operation.");
            outcome["clearedCount"] = json!(0);
            return Ok(outcome);
        }
        let latest_descriptors = book
            .get_mut("chapters")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
        let latest_raw_chapters = latest_private
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
        let latest_chapter_ids = latest_descriptors
            .iter()
            .map(|chapter| {
                chapter
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| "Chapter ID missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if latest_chapter_ids != initial_chapter_ids
            || latest_raw_chapters.len() != initial_raw_chapters.len()
            || latest_private
                .get("book")
                .is_none_or(|engine_book| !engine_book.is_object())
        {
            let mut outcome = book_display_metadata_commit_outcome(
                crate::resource_transactions::CommitState::NotCommitted,
                false,
                None,
                None,
            );
            outcome["error"] = json!("Book chapter directory changed; no cache was cleared. Retry the operation.");
            outcome["clearedCount"] = json!(0);
            return Ok(outcome);
        }

        let mut post_commit_deletes = Vec::with_capacity(initial_chapter_ids.len());
        let mut cleanup_paths = HashSet::new();
        let mut cached_count = 0usize;
        for chapter in latest_descriptors.iter_mut() {
            let chapter_ref = chapter
                .get("resource")
                .and_then(|resource| resource.get("resourceId"))
                .and_then(Value::as_str)
                .filter(|src| !src.is_empty())
                .map(|src| scoped_chapter_cache_ref(book_id, src))
                .transpose()?;
            if let Some(chapter_ref) = chapter_ref {
                let cache_path = writer_guard.data_root().join(chapter_ref.path());
                match tokio::fs::symlink_metadata(&cache_path).await {
                    Ok(metadata) if metadata.file_type().is_file() => cached_count += 1,
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err("Refusing to clear a chapter cache path that is a symbolic link".into());
                    }
                    Ok(_) => {
                        return Err("Chapter cache path is not a regular file".into());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(format!("Cannot inspect chapter cache path: {error}"));
                    }
                }
                if cleanup_paths.insert(chapter_ref.as_str().to_owned()) {
                    post_commit_deletes.push(
                        crate::resource_transactions::PostCommitDelete::chapter(chapter_ref)
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
            if let Some(object) = chapter.as_object_mut() {
                object.remove("resource");
            }
        }
        if latest_raw_chapters.len() != latest_descriptors.len() {
            return Err("Book source and chapter directory are out of sync".into());
        }

        collect_book_chapter_cache_deletes(
            writer_guard.data_root(),
            book_id,
            &mut cleanup_paths,
            &mut post_commit_deletes,
        ).await?;

        let replacements = vec![
            writer_guard
                .public_json_replacement(&book_ref, &book)
                .map_err(|error| error.to_string())?,
        ];
        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "clear-book-chapter-cache",
                replacements,
                post_commit_deletes,
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
                        if recovery_required => Some(format!(
                            "Chapter cache clear was not committed, but rollback is incomplete and resource files may be mixed. Restart before retrying. Details: {error}"
                        )),
                    crate::resource_transactions::CommitState::NotCommitted => Some(format!(
                        "Chapter cache clear was not committed. Changes were not saved and it is safe to retry. Details: {error}"
                    )),
                    crate::resource_transactions::CommitState::Committed => Some(format!(
                        "Chapter cache clear was committed, but cache cleanup or journal recovery is pending. Restart before further writes. Details: {error}"
                    )),
                    crate::resource_transactions::CommitState::Indeterminate => Some(format!(
                        "Chapter cache clear status is unknown. Do not retry; restart the application to recover the transaction first. Details: {error}"
                    )),
                };
                (commit_state, recovery_required, warning)
            }
            Err(error) => (
                crate::resource_transactions::CommitState::Indeterminate,
                true,
                Some(format!(
                    "Chapter cache clear status is unknown. Do not retry; restart the application to recover the transaction first. Details: {error}"
                )),
            ),
        };
        let mut outcome = book_display_metadata_commit_outcome(
            commit_state,
            recovery_required,
            warning,
            None,
        );
        if commit_state == crate::resource_transactions::CommitState::Committed {
            outcome["book"] = self.resource_descriptor(&book_ref);
            outcome["clearedCount"] = json!(cached_count);
        } else {
            outcome["clearedCount"] = json!(0);
        }
        drop(chapter_guards);
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
                outcome["clearedCount"] = json!(0);
                Ok(outcome)
            }
        }
    }

    /// Summarize cached chapter resources currently referenced by shelf books.
    /// This intentionally does not enumerate orphan files or non-chapter assets.
    pub async fn get_chapter_cache_usage(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let shelf = writer_guard
            .read_json_ref(&self.store.shelf_ref())
            .map_err(|error| error.to_string())?;
        let shelf_books = shelf
            .get("books")
            .and_then(Value::as_array)
            .ok_or_else(|| "Invalid shelf JSON: books must be an array".to_owned())?;

        let mut seen_books = HashSet::with_capacity(shelf_books.len());
        let mut seen_chapter_resources = HashSet::new();
        let mut chapter_count = 0u64;
        let mut chapter_bytes = 0u64;
        let mut online_chapter_count = 0u64;
        let mut online_chapter_bytes = 0u64;
        let mut local_chapter_count = 0u64;
        let mut local_chapter_bytes = 0u64;
        let root = writer_guard.data_root().to_path_buf();

        for shelf_book in shelf_books {
            let book_id = shelf_book
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "Invalid shelf JSON: book ID is missing".to_owned())?;
            validate_id(book_id, "bookId")?;
            if !seen_books.insert(book_id.to_owned()) {
                continue;
            }
            let book_ref = self
                .store
                .book_ref(book_id)
                .map_err(|error| error.to_string())?;
            let book = writer_guard
                .read_json_ref(&book_ref)
                .map_err(|_| format!("Cannot read book '{book_id}' while checking cache usage"))?;
            if book.get("id").and_then(Value::as_str) != Some(book_id) {
                return Err(format!(
                    "Book resource ID does not match its catalog path for '{book_id}'"
                ));
            }
            let online = book.get("canChangeSource").and_then(Value::as_bool) == Some(true)
                && book
                    .get("sourceId")
                    .and_then(Value::as_str)
                    .is_some_and(|source_id| !source_id.is_empty());
            let chapters = book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Book '{book_id}' has no valid chapter directory"))?;

            for chapter in chapters {
                let Some(src) = chapter
                    .get("resource")
                    .and_then(|resource| resource.get("resourceId"))
                    .and_then(Value::as_str)
                    .filter(|src| !src.is_empty())
                else {
                    continue;
                };
                let reference = scoped_chapter_cache_ref(book_id, src)?;
                if !seen_chapter_resources.insert(reference.as_str().to_owned()) {
                    continue;
                }

                let path_components = reference.path().split('/').collect::<Vec<_>>();
                let mut path = root.clone();
                let mut file_bytes = None;
                let mut unsafe_path = false;
                for (index, component) in path_components.iter().enumerate() {
                    path.push(*component);
                    match tokio::fs::symlink_metadata(&path).await {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            unsafe_path = true;
                            break;
                        }
                        Ok(metadata) if index + 1 < path_components.len() => {
                            if !metadata.is_dir() {
                                unsafe_path = true;
                                break;
                            }
                        }
                        Ok(metadata) if metadata.file_type().is_file() => {
                            file_bytes = Some(metadata.len());
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                        Err(error) => {
                            return Err(format!(
                                "Cannot inspect a chapter cache entry for book '{book_id}': {error}"
                            ));
                        }
                    }
                }
                if unsafe_path {
                    continue;
                }
                if let Some(bytes) = file_bytes {
                    chapter_count = chapter_count.saturating_add(1);
                    chapter_bytes = chapter_bytes.saturating_add(bytes);
                    if online {
                        online_chapter_count = online_chapter_count.saturating_add(1);
                        online_chapter_bytes = online_chapter_bytes.saturating_add(bytes);
                    } else {
                        local_chapter_count = local_chapter_count.saturating_add(1);
                        local_chapter_bytes = local_chapter_bytes.saturating_add(bytes);
                    }
                }
            }
        }

        Ok(json!({
            "chapterResourceCount": chapter_count,
            "chapterResourceBytes": chapter_bytes,
            "onlineChapterResourceCount": online_chapter_count,
            "onlineChapterResourceBytes": online_chapter_bytes,
            "localChapterResourceCount": local_chapter_count,
            "localChapterResourceBytes": local_chapter_bytes,
        }))
    }

    pub async fn save_settings(&self, settings: Value) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let mut document = if settings.get("reader").is_some() {
            settings
        } else {
            json!({ "reader": settings })
        };
        if !document.is_object() {
            return Err("Settings must be a JSON object".into());
        }
        let source_http_timeout_seconds = match document.get("sourceHttpTimeoutSeconds") {
            None => 15,
            Some(value) => value
                .as_u64()
                .filter(|seconds| [15, 30, 60, 120].contains(seconds))
                .ok_or_else(|| {
                    "Source HTTP timeout must be 15, 30, 60, or 120 seconds".to_owned()
                })?,
        };
        document["sourceHttpTimeoutSeconds"] = json!(source_http_timeout_seconds);
        let reader = document
            .get("reader")
            .and_then(Value::as_object)
            .ok_or_else(|| "Settings reader must be an object".to_owned())?;
        if let Some(config_id) = reader.get("httpTtsConfigId") {
            match config_id {
                Value::Null => {}
                Value::String(value)
                    if !value.is_empty()
                        && value.len() <= 128
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte)) => {}
                _ => return Err("HTTP TTS configuration ID is invalid".to_owned()),
            }
        }
        if let Some(engine) = reader.get("ttsEngine") {
            match engine.as_str() {
                Some("system" | "http") => {}
                _ => return Err("TTS engine must be system or http".to_owned()),
            }
        }
        if reader.contains_key("fontSizePx") {
            validate_number_range(reader, "fontSizePx", 12.0, 36.0)?;
        } else {
            // Accept the pre-canonical field when reading older settings, then
            // write only `fontSizePx` below.
            validate_number_range(reader, "fontSize", 12.0, 36.0)?;
        }
        validate_number_range(reader, "lineHeight", 1.2, 2.8)?;
        validate_integer_range(reader, "preloadCount", 1, 20)?;
        if let Some(skip_seconds) = reader.get("audioSkipSeconds") {
            if skip_seconds
                .as_u64()
                .is_none_or(|seconds| ![10, 15, 30].contains(&seconds))
            {
                return Err("Audio skip seconds must be 10, 15, or 30".to_owned());
            }
        }
        if reader.contains_key("videoPlaybackRate") {
            validate_number_range(reader, "videoPlaybackRate", 0.5, 3.0)?;
        }
        if let Some(theme) = reader.get("theme") {
            match theme.as_str() {
                Some("paper" | "sepia" | "dark" | "system" | "light") => {}
                _ => return Err("Settings theme must be paper, sepia, dark, or system".into()),
            }
        }
        let reader = document
            .get_mut("reader")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "Settings reader must be an object".to_owned())?;
        if reader.get("httpTtsConfigId").is_some_and(Value::is_null) {
            reader.remove("httpTtsConfigId");
        }
        if !reader.contains_key("fontSizePx") {
            if let Some(legacy_size) = reader.remove("fontSize") {
                reader.insert("fontSizePx".to_owned(), legacy_size);
            }
        } else {
            reader.remove("fontSize");
        }
        if reader.get("theme").and_then(Value::as_str) == Some("light") {
            reader.insert("theme".to_owned(), json!("system"));
        }
        // Replacement rules have their own resource and command surface. Keep
        // this legacy settings field empty so there is only one stored copy.
        reader.insert("replacements".to_owned(), json!([]));
        document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
        let settings_ref = self.store.settings_ref();
        self.store
            .update_json_ref(&settings_ref, move |current| {
                // The reader background is written only by the native picker.
                // A settings JSON read materializes this stable ref as a
                // process-local URL, so never persist the client copy.
                let background_image_src = current
                    .get("reader")
                    .and_then(|reader| reader.get("backgroundImageSrc"))
                    .cloned();
                if let Some(reader) = document.get_mut("reader").and_then(Value::as_object_mut) {
                    reader.remove("backgroundImageSrc");
                    if let Some(background_image_src) = background_image_src {
                        reader.insert("backgroundImageSrc".to_owned(), background_image_src);
                    }
                }
                // This value is maintained by successful picker exports. Never
                // accept a stale or client-provided copy from reader settings.
                let last_backup_at_ms = current.get("lastBackupAtMs").cloned();
                if let Some(last_backup_at_ms) = last_backup_at_ms {
                    document["lastBackupAtMs"] = last_backup_at_ms;
                } else if let Some(object) = document.as_object_mut() {
                    object.remove("lastBackupAtMs");
                }
                Ok(document)
            })
            .await
            .map_err(|error| error.to_string())?;
        crate::source_http::set_network_timeout_seconds(source_http_timeout_seconds);
        Ok(self.resource_descriptor(&settings_ref))
    }
}

/// Explicit cache clearing includes stale content-addressed chapter JSON
/// versions that are not referenced in the current public catalog. Only the
/// owner book's chapter namespace is enumerated; source definitions, imported
/// original files and other assets are not touched.
async fn collect_book_chapter_cache_deletes(
    root: &Path,
    book_id: &str,
    cleanup_paths: &mut HashSet<String>,
    deletions: &mut Vec<crate::resource_transactions::PostCommitDelete>,
) -> Result<(), String> {
    validate_id(book_id, "bookId")?;
    let chapter_directory = root.join("books").join(book_id).join("chapters");
    match tokio::fs::symlink_metadata(&chapter_directory).await {
        Ok(metadata) if metadata.file_type().is_dir() => {
            let mut files = tokio::fs::read_dir(&chapter_directory).await
                .map_err(|error| format!("Cannot inspect chapter cache directory: {error}"))?;
            while let Some(entry) = files.next_entry().await
                .map_err(|error| format!("Cannot list chapter cache: {error}"))?
            {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if !name.ends_with(".json") { continue; }
                let file_type = entry.file_type().await
                    .map_err(|error| format!("Cannot inspect chapter cache entry: {error}"))?;
                if !file_type.is_file() || file_type.is_symlink() {
                    return Err("Refusing to clear a non-regular chapter cache file".into());
                }
                let reference = ResourceRef::new(format!(
                    "resource://books/{book_id}/chapters/{name}"
                )).map_err(|error| error.to_string())?;
                if cleanup_paths.insert(reference.as_str().to_owned()) {
                    deletions.push(
                        crate::resource_transactions::PostCommitDelete::chapter(reference)
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => return Err("Refusing to clear a non-directory chapter cache path".into()),
        Err(error) => return Err(format!("Cannot inspect chapter cache directory: {error}")),
    }
    Ok(())
}
