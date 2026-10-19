//! 前端只提交写入参数，与 Rust 共用数据库写入实现。
use super::Change;
use crate::application::ApplicationService;

#[tauri::command]
pub async fn fe_db(
    changes: Vec<Change>,
    service: tauri::State<'_, ApplicationService>,
) -> Result<(), String> {
    let _operation = service.operation_read().await;
    let sources_changed = changes.iter().any(|change| match change {
        Change::Insert { table, .. }
        | Change::Upsert { table, .. }
        | Change::Update { table, .. }
        | Change::Delete { table, .. } => table == "sources",
        _ => false,
    });
    let database = service.resource_store().database().clone();
    let result = tokio::task::spawn_blocking(move || database.modify(changes))
        .await
        .map_err(|error| error.to_string())?;
    if sources_changed {
        service.invalidate_sources().await;
    }
    result.map_err(|error| error.to_string())
}
