//! 封面和阅读背景的业务入口，图片文件交给新 cache。
use crate::{application::ApplicationService, resources::ResourceRef};
pub(crate) const READER_BACKGROUND_BOOK_ID: &str = "reader-theme-backgrounds";

pub(crate) fn user_reader_background_reference(value: &str) -> Option<ResourceRef> {
    let reference = ResourceRef::new(value).ok()?;
    let name = reference
        .path()
        .strip_prefix("cache/image/reader-theme-backgrounds/user-reader-background-")?;
    let (id, extension) = name.rsplit_once('.')?;
    (id.len() == 32
        && id.bytes().all(|b| b.is_ascii_hexdigit())
        && matches!(extension, "jpg" | "png" | "webp"))
    .then_some(reference)
}

pub(crate) async fn cleanup_unreferenced_reader_background_asset(
    service: &ApplicationService,
    previous: Option<String>,
    current: Option<&str>,
) {
    let Some(previous) = previous.filter(|p| Some(p.as_str()) != current) else {
        return;
    };
    let Some(reference) = user_reader_background_reference(&previous) else {
        return;
    };
    if service.resources.asset_in_use(&reference).unwrap_or(true) {
        return;
    }
    if let Err(error) = service.resources.cache.remove_asset(&reference).await {
        eprintln!("清理旧阅读背景失败：{error}");
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
async fn pick(
    app: &tauri::AppHandle,
    service: &ApplicationService,
    owner: &str,
    prefix: &str,
    kind: &str,
) -> Result<Option<(String, ResourceRef)>, String> {
    let Some(file) = crate::file_access::pick_file_with_limit(
        app,
        "图片",
        &["jpg", "jpeg", "png", "webp"],
        16 * 1024 * 1024,
    )
    .await?
    else {
        return Ok(None);
    };
    let (bytes, extension) = tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&file.path).map_err(|e| e.to_string())?;
        if bytes.is_empty() || bytes.len() > 16 * 1024 * 1024 {
            return Err("图片必须在 16 MiB 内".to_owned());
        }
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let extension = match reader.format() {
            Some(image::ImageFormat::Jpeg) => "jpg",
            Some(image::ImageFormat::Png) => "png",
            Some(image::ImageFormat::WebP) => "webp",
            _ => return Err("请选择 JPEG、PNG 或 WebP 图片".into()),
        };
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|e| e.to_string())?;
        if u64::from(decoded.width()) * u64::from(decoded.height()) > 16_000_000 {
            return Err("图片超过 1600 万像素".into());
        }
        drop(file);
        Ok((bytes, extension))
    })
    .await
    .map_err(|e| e.to_string())??;
    let id = format!("{prefix}{}.{}", uuid::Uuid::new_v4().simple(), extension);
    let reference = service
        .resources
        .cache
        .save_asset(kind, owner, &id, &bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some((id, reference)))
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn pick_book_cover(
    app: tauri::AppHandle,
    book_id: String,
    service: tauri::State<'_, ApplicationService>,
) -> Result<serde_json::Value, String> {
    let _operation = service.operation_read().await;
    let _book = service.book_lock(&book_id).await;
    service.read_book(&book_id).await?;
    let kind = crate::book_files::kind(&service.database, &book_id).map_err(|e| e.to_string())?;
    let Some((id, reference)) = pick(&app, &service, &book_id, "user-cover-", &kind).await? else {
        return Ok(serde_json::json!({"cancelled":true}));
    };
    Ok(
        serde_json::json!({"cancelled":false,"assetId":id,"coverSrc":reference.as_str(),"previewSrc":service.resources.url_for(&reference)}),
    )
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn pick_reader_background_image(
    app: tauri::AppHandle,
    service: tauri::State<'_, ApplicationService>,
) -> Result<serde_json::Value, String> {
    let _operation = service.operation_read().await;
    let Some((_, reference)) = pick(
        &app,
        &service,
        READER_BACKGROUND_BOOK_ID,
        "user-reader-background-",
        "image",
    )
    .await?
    else {
        return Ok(serde_json::json!({"cancelled":true}));
    };
    Ok(serde_json::json!({"cancelled":false,"image":service.resources.descriptor(&reference)}))
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn discard_book_cover_asset(
    book_id: String,
    asset_id: String,
    service: tauri::State<'_, ApplicationService>,
) -> Result<serde_json::Value, String> {
    let _operation = service.operation_read().await;
    let _book = service.book_lock(&book_id).await;
    if !asset_id.starts_with("user-cover-") {
        return Err("只能丢弃临时选择的封面".into());
    }
    let kind = crate::book_files::kind(&service.database, &book_id).map_err(|e| e.to_string())?;
    let reference = ResourceRef::new(format!("resource://cache/{kind}/{book_id}/{asset_id}"))
        .map_err(|e| e.to_string())?;
    let in_use = service
        .resources
        .asset_in_use(&reference)
        .map_err(|e| e.to_string())?;
    let removed = if in_use {
        false
    } else {
        service
            .resources
            .cache
            .remove_asset(&reference)
            .await
            .map_err(|e| e.to_string())?
    };
    Ok(serde_json::json!({"removed":removed,"inUse":in_use}))
}
