//! 书籍详情与书源切换 Tauri 命令。

use super::*;

#[tauri::command]
pub async fn prepare_book_from_url(
    url: String,
    read: bool,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.prepare_book_from_url(&url, read).await
}

#[tauri::command]
pub async fn prepare_search_result_book(
    result_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.prepare_search_result_book(&result_id).await
}

#[tauri::command]
pub async fn prepare_alternative_book(
    book_id: String,
    alternative_index: usize,
    source_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.prepare_alternative_book(&book_id, alternative_index, &source_id).await
}

#[tauri::command]
pub async fn change_book_source(
    app: tauri::AppHandle,
    book_id: String,
    source_book_id: String,
    confirm_missing_author: bool,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .change_book_source(&book_id, &source_book_id, confirm_missing_author)
        .await?;
    let _ = app.emit(
        "book-updated",
        json!({ "requestId": 0, "resource": result["book"] }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn change_chapter_source(
    app: tauri::AppHandle,
    book_id: String,
    chapter_id: String,
    result_id: String,
    confirm_missing_author: bool,
    alternative_index: Option<usize>,
    alternative_source_id: Option<String>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .change_chapter_source(&book_id, &chapter_id, &result_id, confirm_missing_author, alternative_index, alternative_source_id.as_deref())
        .await?;
    if result["commitState"] == "committed" {
        let _ = app.emit(
            "book-updated",
            json!({ "requestId": 0, "resource": result["book"] }),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn get_book(
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.get_book(&book_id).await
}

#[tauri::command]
pub async fn save_book(
    app: tauri::AppHandle,
    book: Value,
    request_id: u64,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.save_book(book).await?;
    let result = json!({
        "requestId": request_id,
        "resource": service.resource_descriptor(&resource),
    });
    let _ = app.emit("book-updated", result.clone());
    Ok(result)
}

#[tauri::command]
pub async fn refresh_book_info(
    app: tauri::AppHandle,
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.refresh_book_info(&book_id).await?;
    if result["commitState"] == "committed" {
        let _ = app.emit(
            "book-updated",
            json!({ "requestId": 0, "resource": result["book"] }),
        );
    }
    Ok(result)
}

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
