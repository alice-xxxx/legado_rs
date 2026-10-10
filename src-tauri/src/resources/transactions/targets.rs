//! 构造并校验事务目标与清理目标。

use super::*;

impl Replacement {
    pub(crate) fn public_json(
        reference: ResourceRef,
        bytes: Vec<u8>,
    ) -> Result<Self, TransactionError> {
        let target = TransactionTarget::public_json(reference)?;
        validate_public_json_bytes(&bytes)?;
        Ok(Self {
            target,
            bytes: Some(bytes),
        })
    }

    /// Delete exactly one public book document. The before-image remains in
    /// the journal so a prepared transaction can restore it on recovery.
    pub(crate) fn delete_book_json(reference: ResourceRef) -> Result<Self, TransactionError> {
        let target = TransactionTarget::public_json(reference)?;
        if !is_book_json_path(target.relative_path().as_str()) {
            return Err(TransactionError::new(
                "Book deletion accepts only a public book JSON resource",
            ));
        }
        Ok(Self {
            target,
            bytes: None,
        })
    }

    /// Delete exactly one public progress document.
    pub(crate) fn delete_progress_json(reference: ResourceRef) -> Result<Self, TransactionError> {
        let target = TransactionTarget::public_json(reference)?;
        if !is_progress_json_path(target.relative_path().as_str()) {
            return Err(TransactionError::new(
                "Book deletion accepts only a public progress JSON resource",
            ));
        }
        Ok(Self {
            target,
            bytes: None,
        })
    }

    pub(crate) fn private_book_json(
        book_id: &str,
        bytes: Vec<u8>,
    ) -> Result<Self, TransactionError> {
        let target = TransactionTarget::private_book_json(book_id)?;
        // Private book data may contain source-engine fields named `src` or
        // `*Src`; only require valid JSON here, not public resource semantics.
        validate_json_bytes(&bytes)?;
        Ok(Self {
            target,
            bytes: Some(bytes),
        })
    }

    /// Delete exactly one private source-engine book snapshot.
    pub(crate) fn delete_private_book_json(book_id: &str) -> Result<Self, TransactionError> {
        let target = TransactionTarget::private_book_json(book_id)?;
        Ok(Self {
            target,
            bytes: None,
        })
    }
}

impl PostCommitDelete {
    pub(crate) fn chapter(reference: ResourceRef) -> Result<Self, TransactionError> {
        if !reference.is_local() || !is_chapter_resource_path(reference.path()) {
            return Err(TransactionError::new(
                "Post-commit cleanup accepts only typed chapter resources",
            ));
        }
        ResourceRef::new(format!("resource://{}", reference.path()))
            .map_err(|_| TransactionError::new("Invalid chapter cleanup path"))?;
        Ok(Self {
            path: reference.path().to_owned(),
        })
    }

    /// Remove the cached files beneath exactly one `books/<book-id>` folder.
    /// The tree is checked for symlinks and special files before removal.
    pub(crate) fn book_directory(book_id: &str) -> Result<Self, TransactionError> {
        validate_identifier(book_id)?;
        let path = format!("books/{book_id}");
        safe_components(&path)?;
        Ok(Self { path })
    }
}
