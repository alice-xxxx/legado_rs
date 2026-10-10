//! 书籍移除、书签及书架内的书籍状态变更。

use super::*;

impl ApplicationService {
    pub async fn remove_book(&self, book_id: &str) -> Result<Value, String> {
        self.remove_book_with_outcome(book_id)
            .await
            .map(BookRemovalOutcome::into_value)
            .map_err(|error| error.ui_message())
    }

    pub(crate) async fn remove_book_with_outcome(
        &self,
        book_id: &str,
    ) -> Result<BookRemovalOutcome, BookRemovalError> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")
            .map_err(|error| BookRemovalError::not_committed(error, false))?;
        let _book_lock = self.book_lock(book_id).await;

        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?;
        let progress_ref = self
            .store
            .progress_ref(book_id)
            .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?;
        let bookmarks_ref = self.store.bookmarks_ref();
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| BookRemovalError::not_committed(error.to_string(), true))?;

        let shelf_ref = self.store.shelf_ref();
        let mut next_shelf = writer_guard
            .read_json_ref(&shelf_ref)
            .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?;
        let removed_shelf_entries =
            crate::book_commit::remove_shelf_entry(&mut next_shelf, book_id)
                .map_err(|error| BookRemovalError::not_committed(error, false))?;
        if removed_shelf_entries == 0 {
            return Err(BookRemovalError::not_committed(
                format!("Book '{book_id}' is not on the shelf; no deletion was prepared"),
                false,
            ));
        }

        let mut next_bookmarks = writer_guard
            .read_optional_json_ref(&bookmarks_ref)
            .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?
            .unwrap_or_else(|| {
                json!({
                    "schemaVersion": CURRENT_SCHEMA_VERSION,
                    "bookmarks": [],
                })
            });
        crate::book_commit::remove_bookmarks(&mut next_bookmarks, book_id)
            .map_err(|error| BookRemovalError::not_committed(error, false))?;

        let replacements = vec![
            crate::resource_transactions::Replacement::delete_book_json(book_ref)
                .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?,
            crate::resource_transactions::Replacement::delete_progress_json(progress_ref)
                .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?,
            crate::resource_transactions::Replacement::delete_private_book_json(book_id)
                .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?,
            writer_guard
                .public_json_replacement(&shelf_ref, &next_shelf)
                .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?,
            writer_guard
                .public_json_replacement(&bookmarks_ref, &next_bookmarks)
                .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?,
        ];
        let post_commit_deletes = vec![
            crate::resource_transactions::PostCommitDelete::book_directory(book_id)
                .map_err(|error| BookRemovalError::not_committed(error.to_string(), false))?,
        ];

        let transaction_root = writer_guard.data_root().to_path_buf();
        let transaction_result = tokio::task::spawn_blocking(move || {
            let transaction = crate::resource_transactions::FileTransaction::prepare(
                &transaction_root,
                "remove-book",
                replacements,
                post_commit_deletes,
                &writer_guard,
            )?;
            transaction.commit()
        })
        .await;

        let (recovery_required, warning) = match transaction_result {
            Ok(Ok(())) => (false, None),
            Ok(Err(error))
                if error.commit_state() == crate::resource_transactions::CommitState::Committed =>
            {
                let recovery_required = error.recovery_required_flag();
                let warning = recovery_required.then(|| {
                    format!(
                        "Book removal was committed and the book is no longer in the library, but cached-file cleanup or journal recovery is pending. Restart before further writes. Details: {error}"
                    )
                });
                (recovery_required, warning)
            }
            Ok(Err(error)) => return Err(BookRemovalError::transaction(error)),
            Err(error) => {
                return Err(BookRemovalError {
                    commit_state: crate::resource_transactions::CommitState::Indeterminate,
                    recovery_required: true,
                    detail: format!(
                        "The file worker stopped before reporting whether the book deletion committed: {error}"
                    ),
                });
            }
        };

        Ok(BookRemovalOutcome {
            book_id: book_id.to_owned(),
            shelf: self.resource_descriptor(&shelf_ref),
            recovery_required,
            warning,
        })
    }

    /// Create or update a bookmark only while its book is still present in the
    /// shelf. The per-book lock serializes this check and the bookmark write
    /// with deletion, which takes the same lock before committing removal.
    pub async fn upsert_bookmark(
        &self,
        input: crate::reading_tools::BookmarkInput,
    ) -> Result<ResourceRef, String> {
        let _operation = self.operation_read().await;
        validate_id(&input.book_id, "bookId")?;
        let _book_lock = self.book_lock(&input.book_id).await;

        let shelf_ref = self.store.shelf_ref();
        let shelf = self
            .store
            .read_json_ref(&shelf_ref)
            .await
            .map_err(|error| error.to_string())?;
        let on_shelf = shelf
            .get("books")
            .and_then(Value::as_array)
            .is_some_and(|books| {
                books.iter().any(|book| {
                    book.get("id").and_then(Value::as_str) == Some(input.book_id.as_str())
                })
            });
        if !on_shelf {
            return Err(format!(
                "Book '{}' is no longer on the shelf",
                input.book_id
            ));
        }

        let book_ref = self
            .store
            .book_ref(&input.book_id)
            .map_err(|error| error.to_string())?;
        let book = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|_| format!("Book '{}' is no longer available", input.book_id))?;
        if book.get("id").and_then(Value::as_str) != Some(input.book_id.as_str()) {
            return Err(format!("Book '{}' is no longer available", input.book_id));
        }

        crate::reading_tools::upsert_bookmark(&self.store, input).await
    }
}
