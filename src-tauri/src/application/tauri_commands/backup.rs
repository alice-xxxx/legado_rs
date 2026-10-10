//! 备份与恢复 Tauri 命令。

use super::*;

#[tauri::command]
pub async fn create_backup_from_picker(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let Some(destination) = pick_save_file(&app, "ZIP backup", &["zip"]).await? else {
        return Ok(json!({ "cancelled": true }));
    };
    let _backup = service.webdav_backup_upload_gate.clone().lock_owned().await;
    if let Ok(path) = destination.clone().into_path() {
        service.create_backup(&path).await?;
    } else {
        let directory = std::env::temp_dir().join("legado-rs-backup-staging");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| error.to_string())?;
        let staged = PickerFile::temporary(
            directory.join(format!("{}.zip", uuid::Uuid::new_v4().simple())),
            None,
        );
        service.create_backup(&staged.path).await?;
        let bytes = tokio::fs::read(&staged.path)
            .await
            .map_err(|error| error.to_string())?;
        write_selected_file(&app, destination, &bytes)?;
    }
    // The archive contains the previous successful-export time. Update the
    // live settings only after the user-selected destination is complete.
    let mut response = json!({ "backup": true, "created": true });
    match service.record_backup_success().await {
        Ok(settings) => {
            let _ = app.emit("settings-updated", settings.clone());
            response["settings"] = settings;
        }
        Err(error) => {
            response["warning"] = json!(format!("备份文件已保存，但最近备份时间更新失败：{error}"));
        }
    }
    Ok(response)
}

#[tauri::command]
pub async fn get_webdav_backup_config(
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.get_webdav_backup_config().await
}

#[tauri::command]
pub async fn save_webdav_backup_config(
    url: String,
    username: String,
    password: String,
    clear_password: bool,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .save_webdav_backup_config(url, username, password, clear_password)
        .await
}

#[tauri::command]
pub async fn save_webdav_auto_backup_interval(
    interval_hours: Option<u32>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .save_webdav_auto_backup_interval(interval_hours)
        .await
}

#[tauri::command]
pub async fn upload_webdav_backup(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.upload_webdav_backup().await?;
    if let Some(settings) = result.get("settings") {
        let _ = app.emit("settings-updated", settings.clone());
    }
    Ok(result)
}

#[tauri::command]
pub async fn restore_webdav_backup(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.restore_webdav_backup().await?;
    if result["commitState"] == "committed" && !result["recoveryRequired"].as_bool().unwrap_or(true)
    {
        let bootstrap = result["bootstrap"].clone();
        let _ = app.emit("app-state-updated", bootstrap.clone());
        let _ = app.emit("shelf-updated", bootstrap["shelf"].clone());
        let _ = app.emit("settings-updated", bootstrap["settings"].clone());
        let _ = app.emit("sources-updated", bootstrap["sources"].clone());
    }
    Ok(result)
}

#[tauri::command]
pub async fn restore_backup_from_picker(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let selected = match pick_file(&app, "Legado backup", &["zip"]).await {
        Ok(selected) => selected,
        Err(error) => {
            return Ok(restore_error_response(
                crate::resource_transactions::TransactionError::new(error),
            ));
        }
    };
    let Some(path) = selected else {
        return Ok(json!({ "cancelled": true }));
    };
    let mut bootstrap = match service.restore_backup(&path.path).await {
        Ok(bootstrap) => bootstrap,
        Err(error) => return Ok(restore_error_response(error)),
    };
    let warning = bootstrap
        .as_object_mut()
        .and_then(|object| object.remove("webdavBackupWarning"));
    let _ = app.emit("app-state-updated", bootstrap.clone());
    let _ = app.emit("shelf-updated", bootstrap["shelf"].clone());
    let _ = app.emit("settings-updated", bootstrap["settings"].clone());
    let _ = app.emit("sources-updated", bootstrap["sources"].clone());
    Ok(json!({
        "restored": true,
        "commitState": "committed",
        "recoveryRequired": false,
        "warning": warning,
        "bootstrap": bootstrap,
    }))
}
