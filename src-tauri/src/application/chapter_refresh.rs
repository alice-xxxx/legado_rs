//! 从当前书源重新获取单章正文，并用文件事务替换缓存。

use super::*;

impl ApplicationService {
    pub async fn refresh_chapter_content(
        &self,
        book_id: &str,
        chapter_id: &str,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        validate_id(chapter_id, "chapterId")?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        // 与普通正文准备和换源操作使用相同的锁顺序，避免旧请求覆盖新缓存。
        let _chapter = self.chapter_lock(book_id, chapter_id).await;
        let (
            source_id,
            source_definition,
            engine_book,
            chapter_raw,
            next_chapter_url,
            chapter_title,
            chapter_index,
            book_instance_id,
            catalog_generation,
        ) = {
            let sources = self.read_sources().await?;
            let _book = self.book_lock(book_id).await;
            let private = self
                .read_internal_json(self.book_engine_path(book_id)?)
                .await?;
            let book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|error| error.to_string())?;
            let source_id = private["sourceId"]
                .as_str()
                .ok_or_else(|| "Book source ID is missing".to_owned())?
                .to_owned();
            let source = sources
                .iter()
                .find(|source| source.id == source_id)
                .ok_or_else(|| "Book source is no longer available".to_owned())?;
            if crate::source_metadata::media_type(&source.source).is_some() {
                return Err("当前刷新操作只适用于文本章节".to_owned());
            }
            let engine_book = private
                .get("book")
                .cloned()
                .ok_or_else(|| "Book engine data is missing".to_owned())?;
            let raw_chapters = private
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
            let descriptors = book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            if raw_chapters.len() != descriptors.len() {
                return Err(
                    "Book chapter directory is out of sync; refresh it before reading".into(),
                );
            }
            let chapter_index = descriptors
                .iter()
                .position(|chapter| chapter["id"].as_str() == Some(chapter_id))
                .ok_or_else(|| "Chapter is no longer in the book directory".to_owned())?;
            let chapter_raw = raw_chapters[chapter_index].clone();
            let chapter_title = descriptors[chapter_index]["title"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let next_chapter_url = raw_chapters
                .get(chapter_index + 1)
                .and_then(|chapter| text_at(chapter, &["url", "chapterUrl"]));
            (
                source_id.clone(),
                source.source.clone(),
                engine_book,
                chapter_raw,
                next_chapter_url,
                chapter_title,
                chapter_index,
                private
                    .get("bookInstanceId")
                    .cloned()
                    .unwrap_or(Value::Null),
                private
                    .get("catalogGeneration")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        };

        let content_base_url = text_at(&chapter_raw, &["url", "chapterUrl"])
            .or_else(|| text_at(&engine_book, &["bookUrl", "url"]));
        let request = engine_request(
            if crate::rss::is_legacy_rss_source(&source_definition) {
                "rssContent"
            } else {
                "content"
            },
            &source_definition,
            None,
            None,
            Some(engine_book.clone()),
            Some(chapter_raw.clone()),
            next_chapter_url,
        );
        let content = self
            .executor
            .execute(request)
            .await?
            .as_str()
            .ok_or_else(|| "Source engine returned non-text chapter content".to_owned())?
            .to_owned();
        let content = crate::source_metadata::normalize_chapter_content(&source_definition, &content);
        if content.trim().is_empty() {
            return Err(
                "Source engine returned empty chapter content; the existing cache was preserved"
                    .into(),
            );
        }
        let (chapter_ref, chapter_document, content_format) = if crate::source_metadata::is_comic_source_metadata(&source_definition) {
            let pages = self.source_image_pages(book_id, chapter_id, &source_definition, &engine_book, &chapter_raw, &content, content_base_url.as_deref()).await?;
            (None, json!({"pages": pages}), "image")
        } else if looks_like_html(&content) {
            let (reference, document) = self.source_rich_text_document(book_id, chapter_id,
                &source_definition, &engine_book, &chapter_raw, &content, content_base_url.as_deref()).await?;
            (Some(reference), document, "richText")
        } else {
            let (reference, document) = self.store.chapter_text_document(book_id, chapter_id, &content)
                .map_err(|error| error.to_string())?;
            (Some(reference), document, "text")
        };

        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let latest_private = self
            .read_internal_json(self.book_engine_path(book_id)?)
            .await
            .map_err(|_| "Book was removed while refreshing the chapter".to_owned())?;
        let mut latest_book = writer_guard
            .read_json_ref(&book_ref)
            .map_err(|_| "Book was removed while refreshing the chapter".to_owned())?;
        if latest_private.get("sourceId").and_then(Value::as_str) != Some(source_id.as_str())
            || latest_private
                .get("book")
                .is_none_or(|book| book != &engine_book)
            || latest_private
                .get("bookInstanceId")
                .cloned()
                .unwrap_or(Value::Null)
                != book_instance_id
            || latest_private
                .get("catalogGeneration")
                .cloned()
                .unwrap_or(Value::Null)
                != catalog_generation
        {
            return Err("Book source or catalog changed while refreshing the chapter; the existing cache was preserved".into());
        }
        let latest_raw_chapters = latest_private
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
        let latest_descriptors = latest_book
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
        if latest_raw_chapters.len() != latest_descriptors.len() {
            return Err("Book chapter directory changed while refreshing the chapter".into());
        }
        let latest_index = latest_descriptors
            .iter()
            .position(|chapter| chapter["id"].as_str() == Some(chapter_id))
            .ok_or_else(|| "Chapter is no longer in the book directory".to_owned())?;
        if latest_index != chapter_index
            || latest_raw_chapters[latest_index] != chapter_raw
            || latest_descriptors[latest_index]["title"].as_str() != Some(chapter_title.as_str())
        {
            return Err(
                "Chapter changed while refreshing; the existing cache was preserved".into(),
            );
        }
        latest_book["contentType"] = json!(crate::source_metadata::content_type(&source_definition));
        set_optional_public_field(&mut latest_book, "mediaType", None);
        let mut replacements = Vec::new();
        if let Some(reference) = chapter_ref {
            latest_book["chapters"][latest_index]["resource"] = serde_json::to_value(
                crate::models::ChapterResourceDescriptor::for_chapter(&reference, content_format)
            ).map_err(|error| error.to_string())?;
            replacements.push(writer_guard.public_json_replacement(&reference, &chapter_document).map_err(|error| error.to_string())?);
        } else {
            let chapter = &mut latest_book["chapters"][latest_index];
            chapter["pages"] = chapter_document["pages"].clone();
            if let Some(values) = chapter.as_object_mut() { values.remove("resource"); values.remove("url"); }
        }
        replacements.push(writer_guard.public_json_replacement(&book_ref, &latest_book).map_err(|error| error.to_string())?);
        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "refresh-chapter-content",
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
            Ok(Err(error)) => (
                error.commit_state(),
                error.recovery_required_flag(),
                Some(format!(
                    "Chapter refresh transaction did not complete cleanly. Details: {error}"
                )),
            ),
            Err(error) => (
                crate::resource_transactions::CommitState::Indeterminate,
                true,
                Some(format!(
                    "Chapter refresh transaction status is unknown. Restart the application before retrying. Details: {error}"
                )),
            ),
        };
        let commit_state = match commit_state {
            crate::resource_transactions::CommitState::NotCommitted => "notCommitted",
            crate::resource_transactions::CommitState::Committed => "committed",
            crate::resource_transactions::CommitState::Indeterminate => "indeterminate",
        };
        if commit_state != "committed" || recovery_required {
            return Ok(json!({
                "commitState": commit_state,
                "recoveryRequired": recovery_required,
                "warning": warning.as_deref(),
                "error": warning.as_deref(),
                "chapterId": chapter_id,
                "fromIndex": chapter_index,
            }));
        }
        Ok(json!({
            "commitState": commit_state,
            "recoveryRequired": false,
            "warning": warning,
            "chapterId": chapter_id,
            "book": self.resource_descriptor(&book_ref),
            "fromIndex": chapter_index,
            "prepared": 1,
        }))
    }
}
