//! Switch between cached books without replacing either cache's identity.
use super::*;

impl ApplicationService {
    pub async fn change_book_source(
        &self,
        book_id: &str,
        source_book_id: &str,
        confirm_missing_author: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        validate_id(source_book_id, "sourceBookId")?;
        if book_id == source_book_id {
            return Err("Choose a different cached book".into());
        }
        let sources = self.read_sources().await?;
        let mut ids = [book_id, source_book_id];
        ids.sort();
        let _first = self.book_lock(ids[0]).await;
        let _second = self.book_lock(ids[1]).await;
        let book_ref = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        let next_ref = self.store.book_ref(source_book_id).map_err(|error| error.to_string())?;
        let writer = self.store.transaction_writer_guard().await.map_err(|error| error.to_string())?;
        let old_book = writer.read_json_ref(&book_ref).map_err(|error| error.to_string())?;
        let old_private = self.read_internal_json(self.book_engine_path(book_id)?).await?;
        let mut next_book = writer.read_json_ref(&next_ref).map_err(|error| error.to_string())?;
        let next_private = self.read_internal_json(self.book_engine_path(source_book_id)?).await?;
        let source_id = next_private["sourceId"].as_str().ok_or_else(|| "Cached book has no source".to_owned())?;
        let target_url = text_at(&next_private["book"], &["bookUrl", "url"]).unwrap_or_default();
        if !old_book["alternatives"].as_array().is_some_and(|items| items.iter().any(|item|
            item["sourceId"] == source_id && item["engineurl"].as_str() == Some(target_url.as_str()))) {
            return Err("Selected content is not a saved alternative source".into());
        }
        let next_source = sources.iter().find(|source| source.id == source_id && source.enabled)
            .ok_or_else(|| "Selected book source is unavailable".to_owned())?;
        let old_source = sources.iter().find(|source| old_private["sourceId"].as_str() == Some(source.id.as_str()));
        let missing_source = Value::Null;
        let old_definition = old_source.map(|source| &source.source).unwrap_or(&missing_source);
        let content_type = crate::source_metadata::content_type(&next_source.source);
        if content_type != crate::source_metadata::book_content_type(&old_book, old_definition) {
            return Err("Selected book source has a different content type".into());
        }
        next_book["contentType"] = json!(content_type);
        set_optional_public_field(&mut next_book, "mediaType",
            crate::source_metadata::media_type(&next_source.source).map(|kind| json!(kind)));
        if next_book["id"].as_str() != Some(source_book_id)
            || next_book["sourceId"].as_str() != Some(source_id)
            || next_private["sourceId"].as_str() != Some(source_id)
            || next_book["chapters"].as_array().is_none_or(Vec::is_empty)
        {
            return Err("Selected source cache is incomplete".into());
        }
        let identity = json!({
            "targetTitle": text_at(&old_private["book"], &["name", "title"]).unwrap_or_default(),
            "targetAuthor": text_at(&old_private["book"], &["author"]).unwrap_or_default(),
        });
        let author_unverified = candidate_identity_match(&identity, &next_private["book"])
            .ok_or_else(|| "Cached source book identity does not match".to_owned())?;
        if author_unverified && !confirm_missing_author {
            return Err("Confirm the book because its author could not be verified".into());
        }
        let mut alternatives = old_book["alternatives"].as_array().cloned().unwrap_or_default();
        alternatives.extend(next_book["alternatives"].as_array().cloned().unwrap_or_default());
        let mut old_engine = old_private["book"].clone();
        let old_url = text_at(&old_engine, &["bookUrl", "url"]).unwrap_or_default();
        if let Some(raw) = old_engine.as_object_mut() { raw.remove("bookUrl"); raw.remove("url"); }
        alternatives.push(json!({"sourceId": old_private["sourceId"], "sourceName": old_book["sourceName"],
            "title": old_book["title"], "author": old_book["author"], "cover": old_book["coverSrc"],
            "latest": old_book["latestChapter"], "engineurl": old_url, "engine": old_engine}));
        let mut seen = HashSet::new();
        alternatives.retain(|item| {
            let key = (item["sourceId"].as_str().unwrap_or_default().to_owned(), item["engineurl"].as_str().unwrap_or_default().to_owned());
            !(key.0 == source_id && key.1 == target_url) && seen.insert(key)
        });
        next_book["alternatives"] = json!(alternatives);
        let old_index = old_book["progress"]["chapterIndex"].as_u64().unwrap_or(0) as usize;
        let old_title = old_book["chapters"][old_index]["title"].as_str().unwrap_or_default();
        let next_chapters = next_book["chapters"].as_array().ok_or("Target content has no catalog")?;
        let matches = next_chapters.iter().enumerate().filter(|(_, chapter)|
            normalize_identity(chapter["title"].as_str().unwrap_or_default()) == normalize_identity(old_title)
        ).map(|(index, _)| index).collect::<Vec<_>>();
        let index = if matches.len() == 1 { matches[0] } else { old_index.min(next_chapters.len().saturating_sub(1)) };
        let chapter_id = next_chapters[index]["id"].clone();
        let mut progress = old_book["progress"].clone();
        progress["chapterId"] = chapter_id;
        progress["chapterIndex"] = json!(index);
        progress["updatedAtMs"] = json!(now_ms());
        if matches.len() != 1 { progress["offset"] = json!(0); }
        if let Some(values) = progress.as_object_mut() { values.remove("pageId"); values.remove("anchor"); }
        next_book["progress"] = progress;
        let replacements = vec![writer.public_json_replacement(&next_ref, &next_book).map_err(|error| error.to_string())?];
        let root = writer.data_root().to_path_buf();
        tokio::task::spawn_blocking(move || {
            crate::resource_transactions::FileTransaction::prepare(
                &root, "change-book-source", replacements, Vec::new(), &writer
            )?.commit()
        }).await.map_err(|error| format!("Source switch worker stopped: {error}"))?
            .map_err(|error| format_source_change_transaction_error(error.commit_state(), error.recovery_required_flag(), &error.to_string()))?;
        Ok(json!({
            "book": self.resource_descriptor(&next_ref),
            "progress": next_book["progress"],
        }))
    }
}
