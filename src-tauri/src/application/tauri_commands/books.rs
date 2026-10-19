//! 书籍展示信息与章节缓存的存储命令。

use super::*;

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
    service
        .store_selected_book_cover(&book_id, picked_file)
        .await
}

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
    let image = service.store_selected_reader_background(picked_file).await?;
    Ok(json!({ "cancelled": false, "image": image }))
}

#[tauri::command]
pub async fn discard_book_cover_asset(
    book_id: String,
    asset_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .discard_selected_book_cover(&book_id, &asset_id)
        .await
}
