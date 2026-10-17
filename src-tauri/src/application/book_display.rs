//! 用户可编辑的书籍展示信息、封面和阅读背景。

use super::*;

/// Accept only UI-editable metadata; chapters, progress, and cache data are not writable here.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BookMetadataUpdate {
    id: String,
    #[serde(default)]
    display_overrides: crate::models::BookDisplayOverrides,
}

impl ApplicationService {
    pub async fn save_book(&self, incoming: Value) -> Result<ResourceRef, String> {
        let _operation = self.operation_read().await;
        let incoming: BookMetadataUpdate = serde_json::from_value(incoming)
            .map_err(|error| format!("Invalid book metadata update: {error}"))?;
        let book_id = incoming.id;
        validate_id(&book_id, "bookId")?;
        let book_ref = self
            .store
            .book_ref(&book_id)
            .map_err(|error| error.to_string())?;
        let _book = self.book_lock(&book_id).await;
        let writer = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        let overrides = incoming.display_overrides;
        let mut patch = json!({
            "title": overrides.title,
            "author": overrides.author,
            "intro": overrides.intro,
            "coverSrc": overrides.cover_src,
        });
        if patch["coverSrc"]
            .as_str()
            .is_some_and(is_runtime_book_cover_url)
        {
            patch
                .as_object_mut()
                .expect("book display patch is an object")
                .remove("coverSrc");
        }
        verify_selected_cover_asset(&writer, &patch)?;
        let database=self.store.database().clone();
        let path=book_ref.path().to_owned();
        let kind=self.store.book_type(&book_id).map_err(|e|e.to_string())?;
        tokio::task::spawn_blocking(move ||database.save_display(&path,&kind,&patch)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        Ok(book_ref)
    }


    pub async fn store_selected_book_cover(
        &self,
        book_id: &str,
        picked_file: PickerFile,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let _book = self.book_lock(book_id).await;
        let book = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|_| format!("Book '{book_id}' is no longer available"))?;
        if book.get("id").and_then(Value::as_str) != Some(book_id) {
            return Err("Book resource ID does not match its catalog path".into());
        }
        let path = picked_file.path.clone();
        let (bytes, extension) =
            tokio::task::spawn_blocking(move || read_and_validate_user_image(&path, "Cover"))
                .await
                .map_err(|error| format!("Cover image worker failed: {error}"))??;
        let asset_id = format!("user-cover-{}.{}", uuid::Uuid::new_v4().simple(), extension);
        let resource = self
            .store
            .write_asset(book_id, &asset_id, &bytes)
            .await
            .map_err(|error| error.to_string())?;
        let descriptor = self.resource_descriptor(&resource);
        Ok(json!({
            "cancelled": false,
            "assetId": asset_id,
            "coverSrc": resource.as_str(),
            "previewSrc": descriptor["src"],
        }))
    }

    pub async fn store_selected_reader_background(
        &self,
        picked_file: PickerFile,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let path = picked_file.path.clone();
        let (bytes, extension) =
            tokio::task::spawn_blocking(move || read_and_validate_user_image(&path, "Background"))
                .await
                .map_err(|error| format!("Background image worker failed: {error}"))??;
        let asset_id = format!(
            "user-reader-background-{}.{}",
            uuid::Uuid::new_v4().simple(),
            extension
        );
        let resource = self
            .store
            .write_asset(READER_BACKGROUND_BOOK_ID, &asset_id, &bytes)
            .await
            .map_err(|error| error.to_string())?;
        Ok(self.resource_descriptor(&resource))
    }

    pub(super) async fn cleanup_unreferenced_reader_background_asset(
        &self,
        previous_ref: Option<String>,
        current_ref: Option<&str>,
    ) {
        let Some(previous_ref) = previous_ref else {
            return;
        };
        if current_ref == Some(previous_ref.as_str()) {
            return;
        }
        let Some(reference) = user_reader_background_reference(&previous_ref) else {
            return;
        };
        let writer_guard = match self.store.transaction_writer_guard().await {
            Ok(writer_guard) => writer_guard,
            Err(error) => {
                eprintln!("Cannot check old reader background references: {error}");
                return;
            }
        };
        match public_book_resources_reference_cover(
            &self.store,
            &writer_guard,
            previous_ref.as_str(),
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                eprintln!("Cannot check old reader background references: {error}");
                return;
            }
        }
        if let Err(error) = writer_guard.remove_asset(&reference) {
            eprintln!("Cannot remove unused reader background image: {error}");
        }
    }

    pub async fn discard_selected_book_cover(
        &self,
        book_id: &str,
        asset_id: &str,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        if !is_user_book_cover_asset_id(asset_id) {
            return Err("Only a temporary user-selected cover can be discarded".into());
        }
        let resource = self
            .store
            .asset_ref(book_id, asset_id)
            .map_err(|error| error.to_string())?;
        let _book = self.book_lock(book_id).await;
        let writer_guard = self
            .store
            .transaction_writer_guard()
            .await
            .map_err(|error| error.to_string())?;
        if public_book_resources_reference_cover(&self.store, &writer_guard, resource.as_str())? {
            return Ok(json!({ "removed": false, "inUse": true }));
        }
        let removed = writer_guard
            .remove_asset(&resource)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "removed": removed, "inUse": false }))
    }
}

fn is_runtime_book_cover_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub(super) fn book_display_metadata_commit_outcome(
    commit_state: crate::resource_transactions::CommitState,
    recovery_required: bool,
    warning: Option<String>,
    resources: Option<Value>,
) -> Value {
    let commit_state = match commit_state {
        crate::resource_transactions::CommitState::NotCommitted => "notCommitted",
        crate::resource_transactions::CommitState::Committed => "committed",
        crate::resource_transactions::CommitState::Indeterminate => "indeterminate",
    };
    let mut outcome = json!({
        "commitState": commit_state,
        "recoveryRequired": recovery_required,
        "warning": warning,
    });
    if let Some(book) = resources {
        outcome["book"] = book;
    }
    outcome
}

pub(super) fn read_and_validate_user_image(
    path: &Path,
    image_kind: &str,
) -> Result<(Vec<u8>, &'static str), String> {
    let file = std::fs::File::open(path)
        .map_err(|_| format!("Cannot read selected {image_kind} image"))?;
    let size = file
        .metadata()
        .map_err(|_| format!("Cannot inspect selected {image_kind} image"))?
        .len();
    if size == 0 || size > MAX_BOOK_COVER_FILE_BYTES {
        return Err(format!(
            "Selected {image_kind} image must be non-empty and no larger than 16 MiB"
        ));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_BOOK_COVER_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("Cannot read selected {image_kind} image"))?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BOOK_COVER_FILE_BYTES {
        return Err(format!(
            "Selected {image_kind} image must be non-empty and no larger than 16 MiB"
        ));
    }

    let mut reader = ImageReader::new(Cursor::new(bytes.as_slice()))
        .with_guessed_format()
        .map_err(|_| format!("Selected {image_kind} image format could not be detected"))?;
    let extension = match reader.format() {
        Some(ImageFormat::Jpeg) => "jpg",
        Some(ImageFormat::Png) => "png",
        Some(ImageFormat::WebP) => "webp",
        _ => {
            return Err(format!(
                "Selected {image_kind} must be a valid JPEG, PNG, or WebP image"
            ));
        }
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_BOOK_COVER_DIMENSION);
    limits.max_image_height = Some(MAX_BOOK_COVER_DIMENSION);
    limits.max_alloc = Some(MAX_BOOK_COVER_PIXELS.saturating_mul(4));
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| format!("Selected {image_kind} image is invalid or exceeds decoder limits"))?;
    let width = decoded.width();
    let height = decoded.height();
    if width == 0
        || height == 0
        || u64::from(width)
            .checked_mul(u64::from(height))
            .is_none_or(|pixels| pixels > MAX_BOOK_COVER_PIXELS)
    {
        return Err(format!(
            "Selected {image_kind} image exceeds the 16 megapixel limit"
        ));
    }
    Ok((bytes, extension))
}

pub(super) fn is_user_book_cover_asset_id(asset_id: &str) -> bool {
    let Some((stem, extension)) = asset_id.rsplit_once('.') else {
        return false;
    };
    let Some(token) = stem.strip_prefix("user-cover-") else {
        return false;
    };
    token.len() == 32
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && matches!(extension, "jpg" | "png" | "webp")
}

pub(super) fn verify_selected_cover_asset(
    writer_guard: &ResourceStoreWriterGuard,
    patch: &Value,
) -> Result<(), String> {
    let Some(cover_ref) = patch.get("coverSrc").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(reference) = user_selected_cover_reference(cover_ref) else {
        return Ok(());
    };
    if !writer_guard
        .asset_exists(&reference)
        .map_err(|error| error.to_string())?
    {
        return Err("Selected cover image is no longer available; choose it again".into());
    }
    Ok(())
}

pub(super) fn user_selected_cover_reference(cover_ref: &str) -> Option<ResourceRef> {
    let reference = ResourceRef::new(cover_ref).ok()?;
    let components = reference.path().split('/').collect::<Vec<_>>();
    if components.len() != 4
        || components[0] != "cache"
        || !crate::resources::content::CONTENT_TYPES.contains(&components[1])
        || !is_user_book_cover_asset_id(components[3])
    {
        return None;
    }
    Some(reference)
}

pub(super) fn user_reader_background_reference(background_ref: &str) -> Option<ResourceRef> {
    let reference = ResourceRef::new(background_ref).ok()?;
    if !reference.is_local() {
        return None;
    }
    let components = reference.path().split('/').collect::<Vec<_>>();
    if components.len() != 4
        || components[0] != "cache"
        || components[1] != "image"
        || components[2] != READER_BACKGROUND_BOOK_ID
    {
        return None;
    }
    let (stem, extension) = components[3].rsplit_once('.')?;
    let token = stem.strip_prefix("user-reader-background-")?;
    if token.len() != 32
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !matches!(extension, "jpg" | "png" | "webp")
    {
        return None;
    }
    Some(reference)
}

pub(super) fn public_book_resources_reference_cover(
    store: &ResourceStore,
    writer_guard: &ResourceStoreWriterGuard,
    cover_ref: &str,
) -> Result<bool, String> {
    for path in store.database().paths().map_err(|e|e.to_string())? {
        if !crate::resources::content::is_content_document(&path) {continue;}
        let (_,book_id,_)=crate::resources::content::content_path(&path).expect("content path");
        if book_id==READER_BACKGROUND_BOOK_ID {continue;}
        let book_ref=ResourceRef::new(format!("resource://{path}")).map_err(|e|e.to_string())?;
        let book = writer_guard
            .read_optional_json_ref(&book_ref)
            .map_err(|_| {
                "Cannot verify cover references; selected cover was retained".to_owned()
            })?;
        if book
            .as_ref()
            .is_some_and(|book| book_display_references_cover(book, cover_ref))
        {
            return Ok(true);
        }
    }

    let settings = writer_guard
        .read_json_ref(&store.settings_ref())
        .map_err(|_| {
            "Cannot verify reader settings before cover cleanup; selected cover was retained"
                .to_owned()
        })?;
    if settings
        .get("reader")
        .and_then(|reader| reader.get("backgroundImageSrc"))
        .and_then(Value::as_str)
        == Some(cover_ref)
    {
        return Ok(true);
    }

    let shelf = writer_guard
        .read_json_ref(&store.shelf_ref())
        .map_err(|_| {
            "Cannot verify shelf cover references; selected cover was retained".to_owned()
        })?;
    let shelf_books = shelf
        .get("books")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "Cannot verify shelf cover references; selected cover was retained".to_owned()
        })?;
    Ok(shelf_books
        .iter()
        .any(|book| book_display_references_cover(book, cover_ref)))
}

pub(super) fn book_display_references_cover(book: &Value, cover_ref: &str) -> bool {
    book.get("coverSrc").and_then(Value::as_str) == Some(cover_ref)
        || book
            .get("displayBase")
            .and_then(|base| base.get("coverSrc"))
            .and_then(Value::as_str)
            == Some(cover_ref)
        || book
            .get("displayOverrides")
            .and_then(|overrides| overrides.get("coverSrc"))
            .and_then(Value::as_str)
            == Some(cover_ref)
}
