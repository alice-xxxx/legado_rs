//! 阅读进度、章节缓存和阅读设置的持久化。

use super::*;

#[cfg(test)]
mod reading_notification_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingExecutor(Arc<AtomicUsize>);
    impl SourceExecutor for CountingExecutor {
        fn execute<'a>(&'a self, _request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(json!("　　Cached text\n　　Next paragraph"))
            })
        }
    }

    async fn fixture() -> (tempfile::TempDir, ApplicationService, Arc<AtomicUsize>) {
        let directory = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ApplicationService::open_with_executor(directory.path(), Arc::new(CountingExecutor(calls.clone()))).await.unwrap();
        service.write_sources(&[SourceRecord {
            id: "source-a".into(), name: "Source".into(), group: None, enabled: true,
            source: json!({"bookSourceName":"Source","bookSourceUrl":"https://example.invalid","bookSourceType":0}),
        }]).await.unwrap();
        let chapters: Vec<Value> = (0..5).map(|index| json!({"id":format!("chapter-{index}"),"index":index,"title":format!("Chapter {index}")})).collect();
        let raw: Vec<Value> = (0..5).map(|index| json!({"url":format!("https://example.invalid/{index}"),"title":format!("Chapter {index}")})).collect();
        service.store.write_json_ref(&service.store.book_ref("book-a").unwrap(), &json!({
            "schemaVersion":1,"id":"book-a","title":"Book","sourceId":"source-a","chapterCount":5,"chapters":chapters
        })).await.unwrap();
        service.write_internal_json(Path::new("books/book-a/source-state.json"), &json!({
            "sourceId":"source-a","book":{"bookUrl":"https://example.invalid/book","name":"Book"},"chapters":raw
        })).await.unwrap();
        service.store.write_json_ref(&service.store.settings_ref(), &json!({"reader":{"preloadCount":2}})).await.unwrap();
        (directory, service, calls)
    }

    #[tokio::test]
    async fn reading_saves_position_and_prepares_current_plus_configured_chapters() {
        let (_directory, service, calls) = fixture().await;
        service.save_progress("book-a", json!({"chapterId":"chapter-0","chapterIndex":0,"offset":3,"updatedAtMs":1})).await.unwrap();
        let published = std::sync::Mutex::new(Vec::new());
        let epoch = service.restore_epoch.load(Ordering::Acquire);
        service.prepare_reading_chapters("book-a", 0, epoch, |resource| published.lock().unwrap().push(resource)).await.unwrap();
        let book = service.store.read_json_ref(&service.store.book_ref("book-a").unwrap()).await.unwrap();
        assert_eq!(book["progress"]["offset"], 3);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(published.lock().unwrap().len(), 3);
        assert!(book["chapters"][2]["resource"].is_object());
        assert!(book["chapters"][3]["resource"].is_null());
        service.save_progress("book-a", json!({"chapterId":"chapter-0","chapterIndex":0,"offset":4,"updatedAtMs":2})).await.unwrap();
        published.lock().unwrap().clear();
        service.prepare_reading_chapters("book-a", 0, epoch, |resource| published.lock().unwrap().push(resource)).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(published.lock().unwrap().is_empty());
        service.save_progress("book-a", json!({"chapterId":"chapter-0","chapterIndex":0,"offset":1,"updatedAtMs":1})).await.unwrap();
        let book = service.store.read_json_ref(&service.store.book_ref("book-a").unwrap()).await.unwrap();
        assert_eq!(book["progress"]["offset"], 4, "A late notification must not roll progress back");
    }

    #[tokio::test]
    async fn stale_reading_window_does_not_prepare_old_chapters() {
        let (_directory, service, calls) = fixture().await;
        service.save_progress("book-a", json!({"chapterId":"chapter-3","chapterIndex":3,"offset":0})).await.unwrap();
        let epoch = service.restore_epoch.load(Ordering::Acquire);
        service.prepare_reading_chapters("book-a", 0, epoch, |_| panic!("Stale window must not publish")).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

impl ApplicationService {
    pub(super) async fn prepare_reading_chapters(
        &self,
        book_id: &str,
        chapter_index: usize,
        epoch: u64,
        updated: impl Fn(Value),
    ) -> Result<(), String> {
        let _operation = self.operation_read().await;
        let settings = self.store.read_json_ref(&self.store.settings_ref()).await.ok();
        let count = reader_defaults(settings).preload_count;
        let book_ref = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        let book = self.store.read_json_ref(&book_ref).await.map_err(|error| error.to_string())?;
        let end = chapter_index.saturating_add(count + 1)
            .min(book["chapters"].as_array().map_or(0, Vec::len));
        for index in chapter_index..end {
            if self.restore_epoch.load(std::sync::atomic::Ordering::Acquire) != epoch { break; }
            let current = self.store.read_json_ref(&book_ref).await.map_err(|error| error.to_string())?;
            if current["progress"]["chapterIndex"].as_u64() != Some(chapter_index as u64) { break; }
            if let Some(src) = current["chapters"][index]["resource"]["resourceId"].as_str() {
                let reference = scoped_chapter_cache_ref(book_id, src)?;
                if self.cached_chapter_format(&reference).await?.is_some() { continue; }
            }
            let result = self.prepare_chapters_unlocked(book_id, index, 1).await?;
            if result["changed"].as_bool() == Some(true) { updated(result["book"].clone()); }
        }
        Ok(())
    }

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
                if book["progress"]["updatedAtMs"].as_u64().unwrap_or(0) > updated_at_ms {
                    return Ok(book);
                }
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
        book["progress"] = serde_json::to_value(ProgressSummary::default())
            .map_err(|error| format!("Cannot encode reset progress: {error}"))?;
        let replacements = vec![
            writer_guard
                .public_json_replacement(&book_ref, &book)
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
                self.resource_descriptor(&book_ref)
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
        let private_path = Path::new("books").join(book_id).join("source-state.json");

        // Take an unlocked catalog snapshot first, then acquire every chapter
        // lock before the source/book/writer locks. prepare_chapters holds a
        // chapter lock across its fetch and publish, so this prevents an
        // in-flight fetch from restoring a cache after the clear commits.
        let initial_book = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
        let initial_private = self.read_internal_json(&private_path).await?;
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
            .read_internal_json(&private_path)
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
        let mut document = settings;
        if !document.is_object() {
            return Err("Settings must be a JSON object".into());
        }
        let home_value = document
            .get("home")
            .cloned()
            .ok_or_else(|| "Settings home is required".to_owned())?;
        let mut home: crate::models::HomeConfigDocument =
            serde_json::from_value(home_value.clone())
                .map_err(|error| format!("Invalid settings home: {error}"))?;
        crate::discovery::home_config::validate_home_document(&home)?;
        let settings_ref = self.store.settings_ref();
        let current = self
            .store
            .read_json_ref(&settings_ref)
            .await
            .map_err(|error| error.to_string())?;
        if current.get("home") != Some(&home_value) {
            crate::discovery::home_config::validate_home_sources(self, &home).await?;
        }
        crate::discovery::home_config::normalize_home_document(&mut home);
        document["home"] = serde_json::to_value(home)
            .map_err(|error| format!("Cannot encode settings home: {error}"))?;

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
        validate_number_range(reader, "fontSizePx", 12.0, 36.0)?;
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
                Some("paper" | "sepia" | "dark" | "system") => {}
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
        if let Some(src) = reader.get("backgroundImageSrc").and_then(Value::as_str) {
            let raw = if let Some(path) = src.strip_prefix(self.server.base_url()) {
                format!("resource://{path}")
            } else { src.to_owned() };
            let reference = super::book_display::user_reader_background_reference(&raw)
                .ok_or_else(|| "Invalid reader background resource".to_owned())?;
            if !self.store.regular_file_exists(&reference).await.map_err(|error| error.to_string())? {
                return Err("Reader background image is missing".into());
            }
            reader.insert("backgroundImageSrc".into(), json!(reference.as_str()));
        }
        let background = document["reader"].get("backgroundImageSrc").and_then(Value::as_str).map(str::to_owned);
        let mut previous_background = None;
        document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
        let _config = self.webdav_backup_config_gate.lock().await;
        self.store
            .update_json_ref(&settings_ref, |current| {
                previous_background = current["reader"].get("backgroundImageSrc").and_then(Value::as_str).map(str::to_owned);
                // This value is maintained by successful picker exports. Never
                // accept a stale or client-provided copy from reader settings.
                let last_backup_at_ms = current.get("lastBackupAtMs").cloned();
                if let Some(last_backup_at_ms) = last_backup_at_ms {
                    document["lastBackupAtMs"] = last_backup_at_ms;
                } else if let Some(object) = document.as_object_mut() {
                    object.remove("lastBackupAtMs");
                }
                super::backup_service::normalize_webdav_settings(&current, &mut document)?;
                Ok(document)
            })
            .await
            .map_err(|error| error.to_string())?;
        self.cleanup_unreferenced_reader_background_asset(previous_background, background.as_deref()).await;
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
