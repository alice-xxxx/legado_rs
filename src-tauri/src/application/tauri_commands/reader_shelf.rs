//! 阅读状态和书架整理 Tauri 命令。

use super::*;

#[tauri::command]
pub async fn prepare_chapters(
    app: tauri::AppHandle,
    book_id: String,
    from_index: usize,
    count: usize,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .prepare_chapters(&book_id, from_index, count)
        .await?;
    let _ = app.emit(
        "book-updated",
        json!({ "requestId": 0, "resource": result["book"] }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn refresh_chapter_content(
    app: tauri::AppHandle,
    book_id: String,
    chapter_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .refresh_chapter_content(&book_id, &chapter_id)
        .await?;
    if result["commitState"].as_str() == Some("committed")
        && result["recoveryRequired"].as_bool() == Some(false)
    {
        let _ = app.emit(
            "book-updated",
            json!({ "requestId": 0, "resource": result["book"] }),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn save_progress(
    app: tauri::AppHandle,
    book_id: String,
    progress: Value,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.save_progress(&book_id, progress).await?;
    let _ = app.emit("book-updated", json!({ "requestId": 0, "resource": result }));
    Ok(result)
}

#[tauri::command]
pub async fn reset_book_progress(
    app: tauri::AppHandle,
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.reset_book_progress(&book_id).await?;
    if result["commitState"] == "committed" {
        let _ = app.emit(
            "book-updated",
            json!({ "requestId": 0, "resource": result["book"] }),
        );
    }
    Ok(result)
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

#[tauri::command]
pub async fn save_configuration_rule(
    app: tauri::AppHandle,
    kind: String,
    rule_id: String,
    rule: Value,
    request_id: u64,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.save_configuration_rule(&kind, &rule_id, rule).await?;
    let result = json!({ "kind": kind, "ruleId": rule_id, "requestId": request_id, "resource": resource });
    let _ = app.emit("resource-updated", result.clone());
    Ok(result)
}

#[tauri::command]
pub async fn save_shelf(
    app: tauri::AppHandle,
    shelf: Value,
    request_id: u64,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.save_shelf(shelf).await?;
    let result = json!({ "requestId": request_id, "resource": resource });
    let _ = app.emit("shelf-updated", result.clone());
    Ok(result)
}

#[tauri::command]
pub async fn save_history(
    app: tauri::AppHandle,
    history: Value,
    request_id: u64,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.save_history(history).await?;
    let result = json!({ "requestId": request_id, "resource": resource });
    let _ = app.emit("history-updated", result.clone());
    Ok(result)
}
