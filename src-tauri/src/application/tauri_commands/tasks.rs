//! 后台任务控制 Tauri 命令。

use super::*;

// Commands acknowledge mutations; Vue reads the task JSON through its resource.
pub(super) fn task_resource_response(mut response: Value) -> Value {
    if let Some(object) = response.as_object_mut() {
        if let Some(task) = object.remove("task") {
            if let Some(id) = task.get("id") {
                object.entry("taskId").or_insert_with(|| id.clone());
            }
        }
    }
    response
}

#[tauri::command]
pub async fn clear_finished_tasks(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.clear_finished_tasks().await?;
    let _ = app.emit(
        "resource-updated",
        json!({ "kind": "tasks", "resource": result["resource"] }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn retry_task(
    task_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.retry_task(&task_id).await.map(task_resource_response)
}

#[tauri::command]
pub async fn start_book_download(
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.start_book_download(&book_id).await.map(task_resource_response)
}

#[tauri::command]
pub async fn start_chapter_download(
    book_id: String,
    from_index: usize,
    count: usize,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.start_chapter_download(&book_id, from_index, count).await.map(task_resource_response)
}

#[tauri::command]
pub async fn refresh_chapters(
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.refresh_chapters(&book_id).await.map(task_resource_response)
}

#[tauri::command]
pub async fn check_new_chapters(
    book_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.check_new_chapters(&book_id).await.map(task_resource_response)
}

#[tauri::command]
pub async fn pause_task(
    task_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.pause_task(&task_id).await.map(task_resource_response)
}

#[tauri::command]
pub async fn resume_task(
    task_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.resume_task(&task_id).await.map(task_resource_response)
}

#[tauri::command]
pub async fn cancel_task(
    task_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.cancel_task(&task_id).await.map(task_resource_response)
}
