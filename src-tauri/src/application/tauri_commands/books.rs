//! 书籍详情与书源切换 Tauri 命令。

use super::*;

#[tauri::command]
pub async fn add_book(
    app: tauri::AppHandle,
    result_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.add_book(&result_id).await?;
    let _ = app.emit("book-added", result["book"].clone());
    let _ = app.emit("shelf-updated", result["shelf"].clone());
    Ok(result)
}

#[tauri::command]
pub async fn prepare_search_result_book(
    result_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.prepare_search_result_book(&result_id).await
}

#[tauri::command]
pub async fn change_book_source(
    app: tauri::AppHandle,
    book_id: String,
    result_id: String,
    confirm_missing_author: bool,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .change_book_source(&book_id, &result_id, confirm_missing_author)
        .await?;
    let _ = app.emit("book-source-changed", result.clone());
    let _ = app.emit("shelf-updated", result["shelf"].clone());
    let _ = app.emit(
        "progress-saved",
        json!({ "bookId": book_id, "book": result["book"] }),
    );
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "bookmarks", "resource": result["bookmarks"]["resource"] }),
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
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .change_chapter_source(&book_id, &chapter_id, &result_id, confirm_missing_author)
        .await?;
    if result["commitState"] == "committed" {
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "book", "resource": result["book"] }),
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
pub async fn update_book_display_metadata(
    app: tauri::AppHandle,
    book_id: String,
    patch: Value,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .update_book_display_metadata(&book_id, patch)
        .await?;
    if result["commitState"] == "committed" {
        let _ = app.emit(
            "resource-updated",
            json!({ "kind": "book", "resource": result["book"] }),
        );
        let _ = app.emit("shelf-updated", result["shelf"].clone());
    }
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
            "resource-updated",
            json!({ "kind": "book", "resource": result["book"] }),
        );
        let _ = app.emit("shelf-updated", result["shelf"].clone());
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
    let settings = service
        .store_selected_reader_background(picked_file)
        .await?;
    let _ = app.emit("settings-updated", settings.clone());
    Ok(json!({ "cancelled": false, "settings": settings }))
}

#[tauri::command]
pub async fn clear_reader_background_image(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let settings = service.clear_reader_background_image().await?;
    let _ = app.emit("settings-updated", settings.clone());
    Ok(settings)
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

#[tauri::command]
pub async fn remove_book(
    app: tauri::AppHandle,
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = match service.remove_book_with_outcome(&book_id).await {
        Ok(outcome) => outcome.into_value(),
        Err(error) => error.into_value(),
    };
    if result["commitState"] == "committed" {
        let _ = app.emit("shelf-updated", result["shelf"].clone());
    }
    Ok(result)
}
