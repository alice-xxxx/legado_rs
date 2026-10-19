//! 兼容旧任务历史；不再创建、恢复或执行下载任务。
use super::*;

pub(super) async fn retire_task_history(store: &ResourceStore) -> Result<(), String> {
    let reference = store.document_ref("tasks").map_err(|error| error.to_string())?;
    let mut history = store
        .read_json_ref_or_default(&reference, json!({"schemaVersion":1,"tasks":[]}))
        .await
        .map_err(|error| error.to_string())?;
    let mut changed = false;
    for task in history["tasks"].as_array_mut().into_iter().flatten() {
        if !matches!(
            task["status"].as_str(),
            Some("completed" | "failed" | "cancelled")
        ) {
            task["status"] = json!("cancelled");
            task["updatedAtMs"] = json!(now_ms());
            changed = true;
        }
    }
    if changed {
        store
            .write_json_ref(&reference, &history)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
