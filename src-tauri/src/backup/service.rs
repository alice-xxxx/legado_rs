//! 备份配置、WebDAV 传输和恢复流程的应用层编排。

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use crate::file_access::{pick_file, pick_save_file, write_selected_file};
use crate::{
    application::{ApplicationService, RestoreAdmissionGuard},
    file_access::PickerFile,
    support::now_ms,
};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tokio::io::AsyncWriteExt;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::{Emitter, State};

const WEBDAV_BACKUP_RETRY_COOLDOWN: Duration = Duration::from_secs(60 * 60);
const WEBDAV_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_WEBDAV_BACKUP_BYTES: u64 = 512 * 1024 * 1024;
const WEBDAV_BACKUP_FILE_NAME: &str = "legado-rs-backup.zip";
use crate::models::WebDavBackupConfig;

async fn temporary_webdav_backup_file() -> Result<PickerFile, String> {
    let directory = std::env::temp_dir()
        .join("legado-rs-webdav-backups")
        .join(uuid::Uuid::new_v4().simple().to_string());
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("Cannot create WebDAV backup staging folder: {error}"))?;
    let staged = PickerFile::temporary(directory.join("backup.zip"), Some(directory.clone()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|error| format!("Cannot protect WebDAV backup staging folder: {error}"))?;
    }
    Ok(staged)
}

fn webdav_directory_url(value: &str) -> Result<reqwest::Url, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("请先填写 WebDAV 目录地址".to_owned());
    }
    if value.len() > 2048 {
        return Err("WebDAV 目录地址不能超过 2048 字节".to_owned());
    }
    let url = reqwest::Url::parse(value).map_err(|_| "WebDAV 目录地址无效".to_owned())?;
    let authority_has_userinfo = value
        .split_once("://")
        .map(|(_, rest)| {
            rest.split(|character| matches!(character, '/' | '?' | '#'))
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
        .unwrap_or(false);
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || authority_has_userinfo
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("WebDAV 目录必须是没有用户名、密码、查询参数或片段的 HTTP(S) URL".to_owned());
    }
    Ok(url)
}

fn webdav_file_url(directory: &reqwest::Url, file_name: &str) -> Result<reqwest::Url, String> {
    let mut url = directory.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "WebDAV 地址不能用于目录路径".to_owned())?;
        segments.pop_if_empty().push(file_name);
    }
    Ok(url)
}

fn webdav_client() -> Result<reqwest::Client, String> {
    crate::http_client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(WEBDAV_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Cannot prepare WebDAV request: {error}"))
}

fn webdav_auth(
    request: reqwest::RequestBuilder,
    config: &WebDavBackupConfig,
) -> reqwest::RequestBuilder {
    if !config.username.is_empty() || config.password.is_some() {
        request.basic_auth(&config.username, config.password.as_deref())
    } else {
        request
    }
}

pub(crate) fn restore_error_response(
    error: crate::resource_transactions::TransactionError,
) -> Value {
    let commit_state = match error.commit_state() {
        crate::resource_transactions::CommitState::NotCommitted => "notCommitted",
        crate::resource_transactions::CommitState::Committed => "committed",
        crate::resource_transactions::CommitState::Indeterminate => "indeterminate",
    };
    json!({
        "restored": false,
        "commitState": commit_state,
        "recoveryRequired": error.recovery_required_flag(),
        "warning": error.to_string(),
    })
}

pub(crate) fn normalize_webdav_settings(
    current: &Value,
    document: &mut Value,
) -> Result<(), String> {
    let previous: WebDavBackupConfig = serde_json::from_value(
        current
            .get("webdavBackup")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|error| error.to_string())?;
    let mut config: WebDavBackupConfig = serde_json::from_value(
        document
            .get("webdavBackup")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|error| format!("Invalid WebDAV configuration: {error}"))?;
    config.url = config.url.trim().to_owned();
    config.username = config.username.trim().to_owned();
    if config.username.len() > 512
        || config.username.contains(':')
        || config.username.chars().any(char::is_control)
    {
        return Err("WebDAV 用户名无效或超过 512 字节".into());
    }
    if config
        .password
        .as_ref()
        .is_some_and(|password| password.len() > 4096)
    {
        return Err("WebDAV 密码不能超过 4096 字节".into());
    }
    if config
        .auto_interval_hours
        .is_some_and(|hours| !(1..=720).contains(&hours))
    {
        return Err("自动备份间隔必须为 1 到 720 小时".into());
    }
    if config.url.is_empty() {
        if config.auto_interval_hours.is_some() {
            return Err("请先设置 WebDAV 连接".into());
        }
        config = WebDavBackupConfig::default();
    } else {
        config.url = webdav_directory_url(&config.url)?.to_string();
        let same_remote = config.url == previous.url && config.username == previous.username;
        let same_schedule =
            same_remote && config.auto_interval_hours == previous.auto_interval_hours;
        config.last_webdav_backup_at_ms = if same_remote {
            previous.last_webdav_backup_at_ms
        } else {
            None
        };
        config.auto_backup_scheduled_at_ms = if same_schedule {
            previous.auto_backup_scheduled_at_ms
        } else {
            config.auto_interval_hours.map(|_| now_ms())
        };
        config.last_auto_attempt_at_ms = if same_schedule {
            previous.last_auto_attempt_at_ms
        } else {
            None
        };
    }
    document["webdavBackup"] = serde_json::to_value(config).map_err(|error| error.to_string())?;
    Ok(())
}

/// Resource-oriented application service for commands and resource delivery.
impl ApplicationService {
    async fn read_webdav_backup_config(&self) -> Result<WebDavBackupConfig, String> {
        let settings = self
            .store
            .read_json_ref(&self.store.settings_ref())
            .await
            .map_err(|error| format!("Cannot read WebDAV backup settings: {error}"))?;
        serde_json::from_value(
            settings
                .get("webdavBackup")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .map_err(|error| format!("Cannot parse WebDAV backup settings: {error}"))
    }

    async fn write_webdav_backup_config(&self, config: &WebDavBackupConfig) -> Result<(), String> {
        let _config = self.webdav_backup_config_gate.lock().await;
        self.write_webdav_backup_config_locked(config).await
    }

    async fn write_webdav_backup_config_locked(
        &self,
        config: &WebDavBackupConfig,
    ) -> Result<(), String> {
        let value = serde_json::to_value(config)
            .map_err(|error| format!("Cannot encode WebDAV backup settings: {error}"))?;
        self.store
            .update_json_ref(&self.store.settings_ref(), move |mut settings| {
                settings["webdavBackup"] = value;
                Ok(settings)
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn record_webdav_backup_success(
        &self,
        exported_config: &WebDavBackupConfig,
    ) -> Result<(), String> {
        let _operation = self.operation_read().await;
        let _config = self.webdav_backup_config_gate.lock().await;
        let mut current = self.read_webdav_backup_config().await?;
        if current.url == exported_config.url && current.username == exported_config.username {
            current.last_webdav_backup_at_ms = Some(now_ms());
            current.last_auto_attempt_at_ms = None;
            self.write_webdav_backup_config_locked(&current).await?;
        }
        Ok(())
    }

    pub async fn upload_webdav_backup(&self) -> Result<Value, String> {
        let _upload = self.webdav_backup_upload_gate.clone().lock_owned().await;
        let config = {
            let _operation = self.operation_read().await;
            self.read_webdav_backup_config().await?
        };
        self.upload_webdav_backup_locked(config).await
    }

    pub async fn maybe_upload_webdav_backup_due(&self) -> Result<Option<Value>, String> {
        let Ok(_upload) = self.webdav_backup_upload_gate.clone().try_lock_owned() else {
            return Ok(None);
        };
        let config = {
            let _operation = self.operation_read().await;
            let _config = self.webdav_backup_config_gate.lock().await;
            let mut config = self.read_webdav_backup_config().await?;
            let Some(interval_hours) = config.auto_interval_hours else {
                return Ok(None);
            };
            if config.url.is_empty() {
                return Ok(None);
            }

            let now = now_ms();
            let interval_ms = u64::from(interval_hours).saturating_mul(60 * 60 * 1000);
            let last_success = config.last_webdav_backup_at_ms.unwrap_or_default();
            let scheduled_at = config.auto_backup_scheduled_at_ms.unwrap_or(now);
            let cadence_start = last_success.max(scheduled_at);
            if now.saturating_sub(cadence_start) < interval_ms {
                return Ok(None);
            }
            let retry_cooldown_ms = WEBDAV_BACKUP_RETRY_COOLDOWN.as_millis() as u64;
            if config
                .last_auto_attempt_at_ms
                .is_some_and(|last_attempt| now.saturating_sub(last_attempt) < retry_cooldown_ms)
            {
                return Ok(None);
            }

            config.last_auto_attempt_at_ms = Some(now);
            self.write_webdav_backup_config_locked(&config).await?;
            config
        };

        self.upload_webdav_backup_locked(config).await.map(Some)
    }

    async fn upload_webdav_backup_locked(
        &self,
        config: WebDavBackupConfig,
    ) -> Result<Value, String> {
        let directory = webdav_directory_url(&config.url)?;
        let destination_url = webdav_file_url(&directory, WEBDAV_BACKUP_FILE_NAME)?;
        let temporary_name = format!(".legado-rs-backup-{}.upload", uuid::Uuid::new_v4().simple());
        let temporary_url = webdav_file_url(&directory, &temporary_name)?;
        let staged = temporary_webdav_backup_file().await?;
        self.create_backup(&staged.path).await?;
        let archive_size = tokio::fs::metadata(&staged.path)
            .await
            .map_err(|error| format!("Cannot inspect WebDAV backup archive: {error}"))?
            .len();
        if archive_size > MAX_WEBDAV_BACKUP_BYTES {
            return Err("WebDAV backup exceeds the 512 MiB size limit".to_owned());
        }

        let client = webdav_client()?;
        let archive = tokio::fs::File::open(&staged.path)
            .await
            .map_err(|error| format!("Cannot open WebDAV backup archive: {error}"))?;
        let move_method = reqwest::Method::from_bytes(b"MOVE")
            .map_err(|error| format!("Cannot prepare WebDAV MOVE request: {error}"))?;
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(archive));
        let upload = webdav_auth(client.put(temporary_url.clone()), &config)
            .header(reqwest::header::CONTENT_LENGTH, archive_size.to_string())
            .body(body)
            .send()
            .await
            .map_err(|error| format!("WebDAV temporary upload failed: {error}"))
            .and_then(|response| {
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(format!(
                        "WebDAV temporary upload failed (HTTP {})",
                        response.status()
                    ))
                }
            });
        let upload = match upload {
            Ok(()) => webdav_auth(client.request(move_method, temporary_url.clone()), &config)
                .header("Destination", destination_url.as_str())
                .header("Overwrite", "T")
                .send()
                .await
                .map_err(|error| format!("WebDAV backup replacement failed: {error}"))
                .and_then(|response| {
                    if response.status().is_success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "WebDAV backup replacement failed (HTTP {})",
                            response.status()
                        ))
                    }
                }),
            Err(error) => Err(error),
        };
        if let Err(error) = upload {
            let _ = webdav_auth(client.delete(temporary_url), &config)
                .send()
                .await;
            return Err(error);
        }

        let mut result = json!({ "uploaded": true });
        let mut warnings = Vec::new();
        if let Err(error) = self.record_webdav_backup_success(&config).await {
            warnings.push(format!("远端备份时间未保存：{error}"));
        }
        match self.record_backup_success().await {
            Ok(settings) => result["settings"] = settings,
            Err(error) => warnings.push(format!("最近备份时间更新失败：{error}")),
        }
        if !warnings.is_empty() {
            result["warning"] = json!(format!("WebDAV 备份已上传，但{}", warnings.join("；")));
        }
        Ok(result)
    }

    async fn download_webdav_backup_archive(&self) -> Result<PickerFile, String> {
        let config = self.read_webdav_backup_config().await?;
        let directory = webdav_directory_url(&config.url)?;
        let source_url = webdav_file_url(&directory, WEBDAV_BACKUP_FILE_NAME)?;
        let client = webdav_client()?;
        let mut response = webdav_auth(client.get(source_url), &config)
            .send()
            .await
            .map_err(|error| format!("WebDAV backup download failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "WebDAV backup download failed (HTTP {})",
                response.status()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_WEBDAV_BACKUP_BYTES)
        {
            return Err("WebDAV backup exceeds the 512 MiB size limit".to_owned());
        }

        let staged = temporary_webdav_backup_file().await?;
        let mut archive = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged.path)
            .await
            .map_err(|error| format!("Cannot create WebDAV restore staging file: {error}"))?;
        let mut downloaded = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("WebDAV backup download failed: {error}"))?
        {
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if downloaded > MAX_WEBDAV_BACKUP_BYTES {
                return Err("WebDAV backup exceeds the 512 MiB size limit".to_owned());
            }
            archive
                .write_all(&chunk)
                .await
                .map_err(|error| format!("Cannot write WebDAV restore staging file: {error}"))?;
        }
        archive
            .sync_all()
            .await
            .map_err(|error| format!("Cannot flush WebDAV restore staging file: {error}"))?;
        drop(archive);
        Ok(staged)
    }

    pub async fn restore_webdav_backup(
        &self,
        close_readers: impl std::future::Future<Output = ()>,
    ) -> Result<Value, String> {
        // Hold a read admission only while retrieving the remote file. Drop it
        // before calling restore_backup, which acquires the exclusive gate.
        let staged = match async {
            let _operation = self.operation_read().await;
            self.download_webdav_backup_archive().await
        }
        .await
        {
            Ok(staged) => staged,
            Err(error) => {
                return Ok(restore_error_response(
                    crate::resource_transactions::TransactionError::new(error),
                ));
            }
        };
        close_readers.await;
        let mut bootstrap = match self.restore_backup(&staged.path).await {
            Ok(bootstrap) => bootstrap,
            Err(error) => return Ok(restore_error_response(error)),
        };
        let warning = bootstrap
            .as_object_mut()
            .and_then(|object| object.remove("webdavBackupWarning"));
        Ok(json!({
            "restored": true,
            "commitState": "committed",
            "recoveryRequired": false,
            "warning": warning,
            "bootstrap": bootstrap,
        }))
    }

    pub async fn create_backup(&self, destination: &Path) -> Result<(), String> {
        let mut barrier = self.restore_barrier.subscribe();
        loop {
            if *barrier.borrow() {
                barrier
                    .changed()
                    .await
                    .map_err(|_| "Restore admission barrier closed".to_owned())?;
                continue;
            }
            let admission = self.admission_gate.clone().read_owned().await;
            if *barrier.borrow() {
                drop(admission);
                continue;
            }
            let exclusive = self.operation_gate.clone().write_owned().await;
            if *barrier.borrow() {
                drop(exclusive);
                drop(admission);
                continue;
            }
            let result = crate::backup::create_backup(&self.store, destination).await;
            drop(exclusive);
            drop(admission);
            return result;
        }
    }

    pub(crate) async fn record_backup_success(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let settings_ref = self.store.settings_ref();
        let timestamp = now_ms();
        let settings = self
            .store
            .update_json_ref(&settings_ref, move |mut settings| {
                if !settings.is_object() {
                    return Err("Settings must be a JSON object".to_owned());
                }
                settings["lastBackupAtMs"] = json!(timestamp);
                Ok(settings)
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(settings)
    }

    pub(crate) async fn restore_backup(
        &self,
        archive: &Path,
    ) -> Result<Value, crate::resource_transactions::TransactionError> {
        self.restore_epoch
            .try_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |epoch| epoch.checked_add(1),
            )
            .map_err(|_| {
                crate::resource_transactions::TransactionError::new("Restore epoch exhausted")
            })?;
        let _restore_serial = self.restore_serial.clone().lock_owned().await;
        self.restore_barrier.send_replace(true);
        self.end_tts().await;
        self.cancel_login()
            .await
            .map_err(crate::resource_transactions::TransactionError::new)?;
        self.server.clear_tts_streams().await;
        let mut admission = RestoreAdmissionGuard {
            barrier: self.restore_barrier.clone(),
            release_on_drop: true,
        };
        let _admission_gate = self.admission_gate.clone().write_owned().await;
        let (webdav_config, mut webdav_config_warning) =
            match self.read_webdav_backup_config().await {
                Ok(config)
                    if config.url.is_empty()
                        && config.username.is_empty()
                        && config.password.is_none() =>
                {
                    (None, None)
                }
                Ok(config) => (Some(config), None),
                Err(error) => (
                    None,
                    Some(format!("无法读取并保留本机 WebDAV 备份设置：{error}")),
                ),
            };
        let _ = self.information_cancel.send(None);
        let _ = self.text_cancel.send(None);
        self.clear_searches().await;
        let _exclusive = self.operation_gate.clone().write_owned().await;
        if let Err(error) = crate::backup::restore_backup(&self.store, archive).await {
            if error.recovery_required_flag() {
                admission.release_on_drop = false;
            }
            return Err(error);
        }
        self.sources.invalidate().await;
        self.server
            .reload_private_media_mappings()
            .await
            .map_err(|error| {
                admission.release_on_drop = false;
                crate::resource_transactions::TransactionError::recovery_required(
                    format!("Restored data is active, but private media mappings could not be reloaded: {error}"),
                    crate::resource_transactions::CommitState::Committed,
                )
            })?;
        if let Some(config) = webdav_config {
            if let Err(error) = self.write_webdav_backup_config(&config).await {
                webdav_config_warning =
                    Some(format!("备份已恢复，但无法保留本机 WebDAV 设置：{error}"));
            }
        }
        let mut bootstrap = self.bootstrap().await.map_err(|error| {
            admission.release_on_drop = false;
            crate::resource_transactions::TransactionError::recovery_required(
                format!("Restored data is active, but app state could not be loaded: {error}"),
                crate::resource_transactions::CommitState::Committed,
            )
        })?;
        if let Some(warning) = webdav_config_warning {
            bootstrap["webdavBackupWarning"] = json!(warning);
        }
        Ok(bootstrap)
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
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
            let _ = app.emit(
                "settings-updated",
                json!({ "requestId": 0, "resource": settings.clone() }),
            );
            response["settings"] = settings;
        }
        Err(error) => {
            response["warning"] = json!(format!("备份文件已保存，但最近备份时间更新失败：{error}"));
        }
    }
    Ok(response)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn upload_webdav_backup(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service.upload_webdav_backup().await?;
    if let Some(settings) = result.get("settings") {
        let _ = app.emit(
            "settings-updated",
            json!({ "requestId": 0, "resource": settings.clone() }),
        );
    }
    Ok(result)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn restore_webdav_backup(
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .restore_webdav_backup(crate::database::reader::close(&app))
        .await;
    crate::database::reader::open(&app).await?;
    let result = result?;
    if result["commitState"] == "committed" && !result["recoveryRequired"].as_bool().unwrap_or(true)
    {
        let bootstrap = result["bootstrap"].clone();
        let _ = app.emit("app-state-updated", bootstrap.clone());
    }
    Ok(result)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
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
    crate::database::reader::close(&app).await;
    let restored = service.restore_backup(&path.path).await;
    crate::database::reader::open(&app).await?;
    let mut bootstrap = match restored {
        Ok(bootstrap) => bootstrap,
        Err(error) => return Ok(restore_error_response(error)),
    };
    let warning = bootstrap
        .as_object_mut()
        .and_then(|object| object.remove("webdavBackupWarning"));
    let _ = app.emit("app-state-updated", bootstrap.clone());
    Ok(json!({
        "restored": true,
        "commitState": "committed",
        "recoveryRequired": false,
        "warning": warning,
        "bootstrap": bootstrap,
    }))
}

