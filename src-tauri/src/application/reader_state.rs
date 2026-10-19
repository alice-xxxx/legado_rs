//! 阅读进度、章节缓存和阅读设置的持久化。

use super::*;

#[cfg(test)]
mod reading_notification_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingExecutor(Arc<AtomicUsize>, &'static str);
    impl SourceExecutor for CountingExecutor {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::SeqCst);
                if request.operation == "resolveMedia" {
                    Ok(json!({"url":request.media_url, "headers":{}}))
                } else {
                    Ok(json!(self.1))
                }
            })
        }
    }

    async fn fixture() -> (tempfile::TempDir, ApplicationService, Arc<AtomicUsize>) {
        fixture_with_content(0, "　　Cached text\n　　Next paragraph").await
    }

    async fn fixture_with_content(source_type: u32, content: &'static str) -> (tempfile::TempDir, ApplicationService, Arc<AtomicUsize>) {
        let directory = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ApplicationService::open_with_executor(directory.path(), Arc::new(CountingExecutor(calls.clone(), content))).await.unwrap();
        service.write_sources(&[SourceRecord {
            id: "source-a".into(), name: "Source".into(), group: None, enabled: true,
            source: json!({"bookSourceName":"Source","bookSourceUrl":"https://example.invalid","bookSourceType":source_type}),
        }]).await.unwrap();
        service.store.bind_book_type("book-a", match source_type { 1 => "audio", 2 => "image", 4 => "video", _ => "text" }).await.unwrap();
        // Media reuse cases exercise one chapter; the supported preload minimum is one.
        let chapter_count = if source_type == 0 { 5 } else { 1 };
        let chapters: Vec<Value> = (0..chapter_count).map(|index| json!({"id":format!("chapter-{index}"),"index":index,"title":format!("Chapter {index}")})).collect();
        let raw: Vec<Value> = (0..chapter_count).map(|index| json!({"url":format!("https://example.invalid/{index}"),"title":format!("Chapter {index}")})).collect();
        service.store.write_json_ref(&service.store.book_ref("book-a").unwrap(), &json!({
            "schemaVersion":1,"id":"book-a","title":"Book","sourceId":"source-a","chapterCount":chapter_count,"chapters":chapters
        })).await.unwrap();
        service.write_internal_json(service.book_engine_path("book-a").unwrap(), &json!({
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

    const COMIC: &str = "<img src=\"https://images.example.invalid/1.webp\"><img src=\"https://images.example.invalid/2.webp\"><script>alert(1)</script>";

    #[tokio::test]
    async fn comic_img_list_is_indexed_as_pages_without_a_chapter_json() {
        let (_directory, service, calls) = fixture_with_content(2, COMIC).await;
        service.get_book("book-a").await.unwrap();
        service.main_text("book-a", "chapter-0", false).await.unwrap();
        let reference = service.store.book_ref("book-a").unwrap();
        let book = service.store.read_json_ref(&reference).await.unwrap();
        assert_eq!(book["contentType"], "image");
        assert_eq!(book["chapters"][0]["pages"].as_array().unwrap().len(), 2);
        assert!(book["chapters"][0]["resource"].is_null());
        let public = crate::resources::content::public_book_view(&service.store.read_raw_document(&reference).unwrap(), "image").unwrap();
        assert!(public.get("media").is_none());
        assert!(public.get("engine").is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn old_comic_text_cache_is_replaced_by_the_unified_content_request() {
        let (_directory, service, calls) = fixture_with_content(2, COMIC).await;
        let original = service.store.write_chapter_text("book-a", "chapter-0", COMIC).await.unwrap();
        let descriptor = serde_json::to_value(crate::models::ChapterResourceDescriptor::for_chapter(&original, "text")).unwrap();
        let book_ref = service.store.book_ref("book-a").unwrap();
        service.store.update_json_ref(&book_ref, |mut book| {
            book["chapters"][0]["resource"] = descriptor;
            Ok(book)
        }).await.unwrap();
        service.store.write_json_ref(&service.store.settings_ref(), &json!({"reader":{"preloadCount":1}})).await.unwrap();
        service.save_progress("book-a", json!({"chapterIndex":0,"offset":0})).await.unwrap();
        let published = std::sync::Mutex::new(Vec::new());
        let epoch = service.restore_epoch.load(Ordering::Acquire);
        service.prepare_reading_chapters("book-a", 0, epoch, |resource| published.lock().unwrap().push(resource)).await.unwrap();
        let book = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(book["contentType"], "image");
        assert!(book["chapters"][0]["resource"].is_null());
        assert_eq!(book["chapters"][0]["pages"].as_array().unwrap().len(), 2);
        assert_eq!(service.store.read_json_ref(&original).await.unwrap()["kind"], "text");
        assert_eq!(calls.load(Ordering::SeqCst), 3, "One content request and two image resolutions replace the old cache");
        assert_eq!(published.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn comic_url_lists_become_private_image_resources() {
        for content in [
            "https://images.example.invalid/1.webp\nhttps://images.example.invalid/2.webp",
            "[\"https://images.example.invalid/1.webp\",\"https://images.example.invalid/2.webp\"]",
        ] {
            let (_directory, service, calls) = fixture_with_content(2, content).await;
            service.main_text("book-a", "chapter-0", false).await.unwrap();
            let book = service.store.read_json_ref(&service.store.book_ref("book-a").unwrap()).await.unwrap();
            assert_eq!(book["contentType"], "image");
            let pages = book["chapters"][0]["pages"].as_array().unwrap();
            assert_eq!(pages.len(), 2);
            assert!(pages.iter().all(|page| page["src"].as_str().unwrap().starts_with("resource://media/")));
            assert_eq!(calls.load(Ordering::SeqCst), 3);
        }
    }

    #[tokio::test]
    async fn changed_source_type_does_not_reuse_an_audio_cache_for_a_comic() {
        let (_directory, service, calls) = fixture_with_content(2, "https://images.example.invalid/1.webp").await;
        let original = ResourceRef::new("resource://media/obsolete-audio").unwrap();
        let descriptor = serde_json::to_value(crate::models::ChapterResourceDescriptor::for_chapter(&original, "audio")).unwrap();
        let book_ref = service.store.book_ref("book-a").unwrap();
        service.store.update_json_ref(&book_ref, |mut book| {
            book["contentType"] = json!("audio");
            book["mediaType"] = json!("audio");
            book["chapters"][0]["resource"] = descriptor;
            Ok(book)
        }).await.unwrap();
        service.store.write_json_ref(&service.store.settings_ref(), &json!({"reader":{"preloadCount":1}})).await.unwrap();
        service.save_progress("book-a", json!({"chapterIndex":0,"offset":0})).await.unwrap();
        let epoch = service.restore_epoch.load(Ordering::Acquire);
        service.prepare_reading_chapters("book-a", 0, epoch, |_| {}).await.unwrap();
        let book = service.store.read_json_ref(&book_ref).await.unwrap();
        assert_eq!(book["contentType"], "image");
        assert!(book["mediaType"].is_null());
        assert!(book["chapters"][0]["resource"].is_null());
        assert!(book["chapters"][0]["pages"].as_array().is_some_and(|pages| !pages.is_empty()));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn audio_and_video_are_resolved_to_player_resources_and_reused() {
        for (source_type, expected) in [(1, "audio"), (4, "video")] {
            let (_directory, service, calls) = fixture_with_content(source_type, "https://media.example.invalid/play.mp4").await;
            service.get_book("book-a").await.unwrap();
            service.main_text("book-a", "chapter-0", false).await.unwrap();
            let book = service.store.read_json_ref(&service.store.book_ref("book-a").unwrap()).await.unwrap();
            assert_eq!(book["contentType"], expected);
            assert_eq!(book["mediaType"], expected);
            assert_eq!(book["chapters"][0]["resource"]["contentFormat"], expected);
            let content = &book["chapters"][0]["media"];
            assert_eq!(content["kind"], "media");
            assert_eq!(content["mediaType"], expected);
            assert!(content.get("text").is_none());
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            service.store.write_json_ref(&service.store.settings_ref(), &json!({"reader":{"preloadCount":1}})).await.unwrap();
            service.save_progress("book-a", json!({"chapterIndex":0,"offset":0})).await.unwrap();
            let epoch = service.restore_epoch.load(Ordering::Acquire);
            service.prepare_reading_chapters("book-a", 0, epoch, |_| panic!("Ready media must be reused")).await.unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
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
            if current["chapters"][index]["volume"] == true { continue; }
            if self.indexed_chapter_ready(&current, index).await? { continue; }
            let chapter_id = current["chapters"][index]["id"].as_str().ok_or("Reading chapter ID is missing")?;
            self.main_text(book_id, chapter_id, false).await?;
            updated(self.resource_descriptor(&book_ref));
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
        let page_id = progress["pageId"].as_str().map(str::to_owned);
        let anchor = progress["anchor"].as_str().map(str::to_owned);
        if let Some(page_id) = &page_id { validate_id(page_id, "pageId")?; }
        let progress_summary = json!({"pageId": page_id, "anchor": anchor, "quality": progress["quality"],
            "chapterId": chapter_id,
            "chapterIndex": chapter_index,
            "offset": offset,
            "updatedAtMs": updated_at_ms,
        });
        let db = self.store.database().clone();
        let path = self.store.canonical_reference(&book_ref).map_err(|e|e.to_string())?.path().to_owned();
        let kind = self.store.book_type(book_id).map_err(|e|e.to_string())?;
        let saved = tokio::task::spawn_blocking(move || db.save_progress(&path,&kind,&progress_summary))
            .await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        let mut descriptor = self.resource_descriptor(&book_ref);
        descriptor["readingUpdate"] = saved.unwrap_or(Value::Null);
        Ok(descriptor)
    }

    pub async fn clear_book_chapter_cache(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let result: Result<Value, String> = async {
        validate_id(book_id, "bookId")?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let private_path = self.book_engine_path(book_id)?;

        // 清缓存前取消在途正文，避免旧请求重新写入。
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

        for chapter_id in &initial_chapter_ids { self.cancel_main_text(book_id, chapter_id); }
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
            if chapter["pages"].as_array().is_some_and(|pages| !pages.is_empty()) { cached_count += 1; }
            let chapter_ref = chapter
                .get("resource")
                .and_then(|resource| resource.get("resourceId"))
                .and_then(Value::as_str)
                .filter(|src| !src.is_empty())
                .map(|src| ResourceRef::new(src).map_err(|error|error.to_string()))
                .transpose()?;
            if chapter_ref.as_ref().is_some_and(|reference| reference.path().starts_with("media/")) {
                cached_count += 1;
            }
            if let Some(chapter_ref) = chapter_ref.filter(|reference| crate::resources::content::content_path(reference.path()).is_some_and(|(_, owner, _)|owner == book_id)) {
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
                for key in ["resource", "url", "pages", "sources", "media", "protocol", "files"] { object.remove(key); }
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

        let replacement = writer_guard.public_json_replacement(&book_ref, &book).map_err(|error|error.to_string())?;
        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "clear-book-chapter-cache",
                vec![replacement],
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
        let mut chapter_count = 0u64;
        let mut chapter_bytes = 0u64;
        let mut online_chapter_count = 0u64;
        let mut online_chapter_bytes = 0u64;
        let mut local_chapter_count = 0u64;
        let mut local_chapter_bytes = 0u64;
        let root = writer_guard.data_root();
        for kind in crate::resources::content::CONTENT_TYPES {
            let directory = root.join("cache").join(kind);
            crate::resources::check_path_no_symlink(root, &directory, true).map_err(|error| error.to_string())?;
            if !directory.exists() { continue; }
            for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                if !entry.file_type().map_err(|error| error.to_string())?.is_dir() { continue; }
                let id = entry.file_name().to_string_lossy().into_owned();
                if id == "debug-source" || id == "tts-audio" || id.starts_with("rss-") || id == READER_BACKGROUND_BOOK_ID { continue; }
                let reference = self.store.book_ref(&id).map_err(|error| error.to_string())?;
                let Some(book) = writer_guard.read_optional_json_ref(&reference).map_err(|error| error.to_string())? else { continue; };
                let online = book["canChangeSource"] == true;
                let cover = book["coverSrc"].as_str().and_then(|url| ResourceRef::new(url).ok());
                for file in std::fs::read_dir(entry.path()).map_err(|error| error.to_string())? {
                    let file = file.map_err(|error| error.to_string())?;
                    let name = file.file_name().to_string_lossy().into_owned();
                    let path = file.path();
                    crate::resources::check_path_no_symlink(root, &path, false).map_err(|error| error.to_string())?;
                    if name == format!("{id}.json") || name.starts_with("original.")
                        || cover.as_ref().is_some_and(|cover| root.join(cover.path()) == path)
                        || name.starts_with("user-book-cover-") { continue; }
                    let metadata = file.metadata().map_err(|error| error.to_string())?;
                    if !metadata.is_file() { continue; }
                    chapter_count = chapter_count.saturating_add(1);
                    chapter_bytes = chapter_bytes.saturating_add(metadata.len());
                    if online {
                        online_chapter_count = online_chapter_count.saturating_add(1);
                        online_chapter_bytes = online_chapter_bytes.saturating_add(metadata.len());
                    } else {
                        local_chapter_count = local_chapter_count.saturating_add(1);
                        local_chapter_bytes = local_chapter_bytes.saturating_add(metadata.len());
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
        crate::models::upgrade_settings_document(&mut document);
        serde_json::from_value::<crate::models::SettingsDocument>(document.clone())
            .map_err(|error| format!("Invalid settings: {error}"))?;
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
        document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
        let _config = self.webdav_backup_config_gate.lock().await;
        let previous_background = self.store.read_json_ref(&settings_ref).await.map_err(|e|e.to_string())?["reader"]
            .get("backgroundImageSrc").and_then(Value::as_str).map(str::to_owned);
        let saved=self.store
            .update_json_ref(&settings_ref, move |current| {
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
        Ok(saved)
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
    let Some(chapter_directory) = crate::resources::content::find_content_dir(root, book_id).map_err(|e| e.to_string())? else { return Ok(()); };
    let kind = chapter_directory.parent().and_then(|p| p.file_name()).and_then(|p| p.to_str()).ok_or("Invalid cache directory")?;
    match tokio::fs::symlink_metadata(&chapter_directory).await {
        Ok(metadata) if metadata.file_type().is_dir() => {
            let mut files = tokio::fs::read_dir(&chapter_directory).await
                .map_err(|error| format!("Cannot inspect chapter cache directory: {error}"))?;
            while let Some(entry) = files.next_entry().await
                .map_err(|error| format!("Cannot list chapter cache: {error}"))?
            {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if name == format!("{book_id}.json") || name.starts_with("original.") || name.starts_with("user-cover-") || name.starts_with("user-book-cover-") || name.starts_with("cover-") || name.starts_with("cover.") { continue; }
                let file_type = entry.file_type().await
                    .map_err(|error| format!("Cannot inspect chapter cache entry: {error}"))?;
                if !file_type.is_file() || file_type.is_symlink() {
                    return Err("Refusing to clear a non-regular chapter cache file".into());
                }
                let reference = ResourceRef::new(format!(
                    "resource://cache/{kind}/{book_id}/{name}"
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
