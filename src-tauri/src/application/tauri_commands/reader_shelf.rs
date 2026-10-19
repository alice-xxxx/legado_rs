//! 阅读状态和书架整理 Tauri 命令。

use super::*;

#[tauri::command]
pub async fn notify_reading(
    app: tauri::AppHandle,
    book_id: String,
    progress: Value,
    refresh_media: bool,
    prepare_current: Option<bool>,
    service: State<'_, ApplicationService>,
) -> Result<(), String> {
    let chapter_index = progress["chapterIndex"].as_u64()
        .ok_or_else(|| "Reading chapterIndex is missing".to_owned())? as usize;
    let resource = service.save_progress(&book_id, progress).await?;
    let reading = &resource["readingUpdate"];
    if reading.is_null() { return Ok(()); }
    let _ = app.emit("reading-progress-updated", json!({ "bookId": book_id, "progress": reading["progress"], "read": reading["read"] }));
    // Reopening the saved chapter must check its body even if its ID is unchanged.
    let prepare = prepare_current.unwrap_or(false) || refresh_media || reading["chapterChanged"].as_bool()==Some(true);
    if refresh_media {
        let book_ref = service.store.book_ref(&book_id).map_err(|error| error.to_string())?;
        let book = service.store.read_json_ref(&book_ref).await.map_err(|error| error.to_string())?;
        let chapter_id = book["chapters"][chapter_index]["id"].as_str().ok_or("Reading chapter ID is missing")?;
        service.main_text(&book_id, chapter_id, false).await?;
        let _ = app.emit("book-updated", json!({"requestId":0,"resource":service.resource_descriptor(&book_ref)}));
    }
    if !prepare { return Ok(()); }
    let service = service.inner().clone();
    let epoch = service.restore_epoch.load(std::sync::atomic::Ordering::Acquire);
    tokio::spawn(async move {
        let result = service.prepare_reading_chapters(&book_id, chapter_index, epoch, |resource| {
            let _ = app.emit("book-updated", json!({ "requestId": 0, "resource": resource }));
        }).await;
        if let Err(error) = result {
            if service.restore_epoch.load(std::sync::atomic::Ordering::Acquire) == epoch {
                let _ = app.emit("reading-resource-error", json!({ "bookId": book_id, "chapterIndex": chapter_index, "error": error }));
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn clear_book_chapter_cache(
    app: tauri::AppHandle,
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.clear_book_chapter_cache(&book_id).await?;
    if result["commitState"] == "committed" {
        let _ = app.emit(
            "book-updated",
            json!({ "requestId": 0, "resource": result["book"] }),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn get_chapter_cache_usage(
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.get_chapter_cache_usage().await
}

#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    settings: Value,
    request_id: u64,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.save_settings(settings).await?;
    let result = json!({ "requestId": request_id, "resource": resource });
    let _ = app.emit("settings-updated", result.clone());
    Ok(result)
}
