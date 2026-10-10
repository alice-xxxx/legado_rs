//! 阅读状态和书架整理 Tauri 命令。

use super::*;

#[tauri::command]
pub async fn get_search_history(service: State<'_, ApplicationService>) -> Result<Value, String> {
    service.get_search_history().await
}

#[tauri::command]
pub async fn delete_search_history(
    app: tauri::AppHandle,
    query: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.delete_search_history(&query).await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "searchHistory", "resource": resource }),
    );
    Ok(resource)
}

#[tauri::command]
pub async fn clear_search_history(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.clear_search_history().await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "searchHistory", "resource": resource }),
    );
    Ok(resource)
}

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
        "chapters-prepared",
        json!({
            "bookId": book_id,
            "fromIndex": from_index,
            "prepared": result["prepared"],
            "book": result["book"],
        }),
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
            "chapters-prepared",
            json!({
                "bookId": book_id,
                "fromIndex": result["fromIndex"],
                "prepared": result["prepared"],
                "book": result["book"],
            }),
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
    let _ = app.emit(
        "progress-saved",
        json!({ "bookId": book_id, "book": result }),
    );
    let _ = app.emit(
        "shelf-updated",
        service.resource_descriptor(&service.resource_store().shelf_ref()),
    );
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
            "resource-updated",
            json!({ "kind": "book", "resource": result["book"] }),
        );
        let _ = app.emit("shelf-updated", result["shelf"].clone());
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
            "resource-updated",
            json!({ "kind": "book", "resource": result["book"] }),
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
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.save_settings(settings).await?;
    let _ = app.emit("settings-updated", result.clone());
    Ok(result)
}

#[tauri::command]
pub async fn list_bookmarks(service: State<'_, ApplicationService>) -> Result<Value, String> {
    let resource = crate::reading_tools::bookmarks_resource(service.resource_store()).await?;
    Ok(service.resource_descriptor(&resource))
}

#[tauri::command]
pub async fn upsert_bookmark(
    app: tauri::AppHandle,
    input: crate::reading_tools::BookmarkInput,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = service.upsert_bookmark(input).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "bookmarks", "resource": descriptor }),
    );
    Ok(descriptor)
}

#[tauri::command]
pub async fn delete_bookmark(
    app: tauri::AppHandle,
    bookmark_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::delete_bookmark(service.resource_store(), &bookmark_id).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "bookmarks", "resource": descriptor }),
    );
    Ok(descriptor)
}

#[tauri::command]
pub async fn reading_history_resource(
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let resource = crate::reading_tools::reading_history_resource(service.resource_store()).await?;
    Ok(service.resource_descriptor(&resource))
}

#[tauri::command]
pub async fn get_reading_statistics(
    from_ms: u64,
    to_ms: u64,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.get_reading_statistics(from_ms, to_ms).await
}

#[tauri::command]
pub async fn record_reading_session(
    app: tauri::AppHandle,
    session: crate::reading_tools::ReadingSession,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::record_reading_session(service.resource_store(), session).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "readingHistory", "resource": descriptor }),
    );
    Ok(descriptor)
}

#[tauri::command]
pub async fn delete_reading_history_for_book(
    app: tauri::AppHandle,
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::delete_reading_history_for_book(service.resource_store(), &book_id)
            .await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "readingHistory", "resource": descriptor }),
    );
    Ok(descriptor)
}

#[tauri::command]
pub async fn clear_reading_history(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource = crate::reading_tools::clear_reading_history(service.resource_store()).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "readingHistory", "resource": descriptor }),
    );
    Ok(descriptor)
}

#[tauri::command]
pub async fn set_book_groups(
    app: tauri::AppHandle,
    book_id: String,
    groups: Vec<String>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::set_book_groups(service.resource_store(), &book_id, groups).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit("shelf-updated", descriptor.clone());
    Ok(descriptor)
}

#[tauri::command]
pub async fn create_shelf_group(
    app: tauri::AppHandle,
    group_name: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::create_shelf_group(service.resource_store(), &group_name).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit("shelf-updated", descriptor.clone());
    Ok(descriptor)
}

#[tauri::command]
pub async fn rename_shelf_group(
    app: tauri::AppHandle,
    old_name: String,
    new_name: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::rename_shelf_group(service.resource_store(), &old_name, &new_name)
            .await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit("shelf-updated", descriptor.clone());
    Ok(descriptor)
}

#[tauri::command]
pub async fn delete_shelf_group(
    app: tauri::AppHandle,
    group_name: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::delete_shelf_group(service.resource_store(), &group_name).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit("shelf-updated", descriptor.clone());
    Ok(descriptor)
}

#[tauri::command]
pub async fn set_shelf_sort(
    app: tauri::AppHandle,
    key: crate::reading_tools::ShelfSortKey,
    order: crate::reading_tools::ShelfSortOrder,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::set_shelf_sort(service.resource_store(), key, order).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit("shelf-updated", descriptor.clone());
    Ok(descriptor)
}

#[tauri::command]
pub async fn set_shelf_order(
    app: tauri::AppHandle,
    ordered_book_ids: Vec<String>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::set_shelf_order(service.resource_store(), ordered_book_ids).await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit("shelf-updated", descriptor.clone());
    Ok(descriptor)
}
