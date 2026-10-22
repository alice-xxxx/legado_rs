//! 封面与阅读背景图片的保存、校验和清理。

use super::{ResourceRef, ResourceStore, ResourceStoreWriterGuard};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use crate::file_access::pick_file_with_limit;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use crate::{application::ApplicationService, file_access::PickerFile, support::validate_id};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use image::{ImageFormat, ImageReader, Limits};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use serde_json::{json,Value};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use std::{
    io::{Cursor, Read},
    path::Path,
};

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::State;

pub(crate) async fn cleanup_unreferenced_reader_background_asset(
    store: &ResourceStore,
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
    let writer_guard = match store.transaction_writer_guard().await {
        Ok(writer_guard) => writer_guard,
        Err(error) => {
            eprintln!("Cannot check old reader background references: {error}");
            return;
        }
    };
    match public_book_resources_reference_cover(store, &writer_guard, previous_ref.as_str()) {
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

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) async fn discard_selected_book_cover(
    store: &ResourceStore,
    book_id: &str,
    asset_id: &str,
) -> Result<Value, String> {
    validate_id(book_id, "bookId")?;
    if !is_user_book_cover_asset_id(asset_id) {
        return Err("Only a temporary user-selected cover can be discarded".into());
    }
    let resource = store
        .asset_ref(book_id, asset_id)
        .map_err(|error| error.to_string())?;
    let writer_guard = store
        .transaction_writer_guard()
        .await
        .map_err(|error| error.to_string())?;
    if public_book_resources_reference_cover(&store, &writer_guard, resource.as_str())? {
        return Ok(json!({ "removed": false, "inUse": true }));
    }
    let removed = writer_guard
        .remove_asset(&resource)
        .map_err(|error| error.to_string())?;
    Ok(json!({ "removed": removed, "inUse": false }))
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
fn read_and_validate_user_image(
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

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
fn is_user_book_cover_asset_id(asset_id: &str) -> bool {
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

pub(crate) fn user_reader_background_reference(background_ref: &str) -> Option<ResourceRef> {
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

pub(crate) fn public_book_resources_reference_cover(
    store: &ResourceStore,
    _writer_guard: &ResourceStoreWriterGuard,
    cover_ref: &str,
) -> Result<bool, String> {
    store.database().asset_referenced(cover_ref).map_err(|e|e.to_string())
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn pick_book_cover(
    app: tauri::AppHandle,
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let Some(picked_file) = pick_file_with_limit(
        &app,
        "封面图片",
        &["jpg", "jpeg", "png", "webp"],
        MAX_BOOK_COVER_FILE_BYTES,
    )
    .await?
    else {
        return Ok(json!({ "cancelled": true }));
    };
    let _operation = service.operation_read().await;
    let _book = service.book_lock(&book_id).await;
    service
        .store
        .read_book(&book_id)
        .await
        .map_err(|e| e.to_string())?;
    let (asset_id, reference) =
        store_image(&service.store, &book_id, "user-cover-", picked_file).await?;
    Ok(
        json!({"cancelled":false,"assetId":asset_id,"coverSrc":reference.as_str(),"previewSrc":service.server.url_for(&reference)}),
    )
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn pick_reader_background_image(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let Some(picked_file) = pick_file_with_limit(
        &app,
        "阅读背景图片",
        &["jpg", "jpeg", "png", "webp"],
        MAX_BOOK_COVER_FILE_BYTES,
    )
    .await?
    else {
        return Ok(json!({ "cancelled": true }));
    };
    let _operation = service.operation_read().await;
    let (_, reference) = store_image(
        &service.store,
        READER_BACKGROUND_BOOK_ID,
        "user-reader-background-",
        picked_file,
    )
    .await?;
    let image = service.server.descriptor(&reference);
    Ok(json!({ "cancelled": false, "image": image }))
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn discard_book_cover_asset(
    book_id: String,
    asset_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let _book = service.book_lock(&book_id).await;
    discard_selected_book_cover(&service.store, &book_id, &asset_id).await
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
const MAX_BOOK_COVER_FILE_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
const MAX_BOOK_COVER_PIXELS: u64 = 16_000_000;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
const MAX_BOOK_COVER_DIMENSION: u32 = 16_384;
pub(crate) const READER_BACKGROUND_BOOK_ID: &str = "reader-theme-backgrounds";

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
async fn store_image(
    store: &ResourceStore,
    owner: &str,
    prefix: &str,
    file: PickerFile,
) -> Result<(String, ResourceRef), String> {
    // 将文件所有权带入工作线程，取消命令也不会提前清理正在读取的临时文件。
    let (bytes, extension) = tokio::task::spawn_blocking(move || {
        let image = read_and_validate_user_image(&file.path, "Image");
        drop(file);
        image
    })
    .await
    .map_err(|e| e.to_string())??;
    let id = format!("{prefix}{}.{}", uuid::Uuid::new_v4().simple(), extension);
    let reference = store
        .write_asset(owner, &id, &bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok((id, reference))
}
