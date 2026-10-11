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
        let _sources = self.sources_lock.lock().await;
        let sources = self.read_sources().await?;
        let mut ids = [book_id, source_book_id];
        ids.sort();
        let _first = self.book_lock(ids[0]).await;
        let _second = self.book_lock(ids[1]).await;
        let book_ref = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        let next_ref = self.store.book_ref(source_book_id).map_err(|error| error.to_string())?;
        let writer = self.store.transaction_writer_guard().await.map_err(|error| error.to_string())?;
        let old_book = writer.read_json_ref(&book_ref).map_err(|error| error.to_string())?;
        let old_private = self.read_internal_json(Path::new("books").join(book_id).join("source-state.json")).await?;
        if !old_book["sourceBookIds"].as_array().is_some_and(|ids|
            ids.iter().any(|id| id.as_str() == Some(source_book_id))) {
            return Err("Selected cache is not an alternative source of this book".into());
        }
        let mut next_book = writer.read_json_ref(&next_ref).map_err(|error| error.to_string())?;
        let next_private = self.read_internal_json(Path::new("books").join(source_book_id).join("source-state.json")).await?;
        let source_id = next_private["sourceId"].as_str().ok_or_else(|| "Cached book has no source".to_owned())?;
        if !sources.iter().any(|source| source.id == source_id && source.enabled) {
            return Err("Selected book source is unavailable".into());
        }
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
        let mut alternatives = old_book["sourceBookIds"].as_array().cloned().unwrap_or_default();
        alternatives.extend(next_book["sourceBookIds"].as_array().cloned().unwrap_or_default());
        alternatives.push(json!(book_id));
        let mut seen = HashSet::new();
        alternatives.retain(|id| id.as_str().is_some_and(|id| id != source_book_id && seen.insert(id.to_owned())));
        next_book["sourceBookIds"] = json!(alternatives);
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
