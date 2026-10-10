//! 备份配置、WebDAV 传输和恢复流程的应用层编排。

use super::*;

const WEBDAV_BACKUP_RETRY_COOLDOWN: Duration = Duration::from_secs(60 * 60);
const WEBDAV_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_WEBDAV_BACKUP_BYTES: u64 = 512 * 1024 * 1024;
const WEBDAV_BACKUP_FILE_NAME: &str = "legado-rs-backup.zip";
const WEBDAV_BACKUP_CONFIG_PATH: &str = "webdav-backup.json";
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WebDavBackupConfig {
    #[serde(default)]
    url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    auto_interval_hours: Option<u32>,
    #[serde(default)]
    last_webdav_backup_at_ms: Option<u64>,
    #[serde(default)]
    auto_backup_scheduled_at_ms: Option<u64>,
    #[serde(default)]
    last_auto_attempt_at_ms: Option<u64>,
}

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
    reqwest::Client::builder()
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

pub(super) fn restore_error_response(
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

/// Resource-oriented application service for commands and resource delivery.
impl ApplicationService {
    async fn read_webdav_backup_config(&self) -> Result<WebDavBackupConfig, String> {
        let path = self.private_root.join(WEBDAV_BACKUP_CONFIG_PATH);
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(WebDavBackupConfig::default());
            }
            Err(error) => return Err(format!("Cannot read WebDAV backup settings: {error}")),
        };
        serde_json::from_slice(&bytes)
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
        self.write_private_json(Path::new(WEBDAV_BACKUP_CONFIG_PATH), &value)
            .await
    }

    fn webdav_backup_config_metadata(config: &WebDavBackupConfig) -> Value {
        json!({
            "url": config.url,
            "username": config.username,
            "hasPassword": config.password.as_ref().is_some_and(|password| !password.is_empty()),
            "autoIntervalHours": config.auto_interval_hours,
        })
    }

    pub async fn get_webdav_backup_config(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let config = self.read_webdav_backup_config().await?;
        Ok(Self::webdav_backup_config_metadata(&config))
    }

    pub async fn save_webdav_backup_config(
        &self,
        url: String,
        username: String,
        password: String,
        clear_password: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let url = url.trim().to_owned();
        let username = username.trim().to_owned();
        if username.len() > 512 || username.contains(':') || username.chars().any(char::is_control)
        {
            return Err("WebDAV 用户名无效或超过 512 字节".to_owned());
        }
        if password.len() > 4096 {
            return Err("WebDAV 密码不能超过 4096 字节".to_owned());
        }

        let _config = self.webdav_backup_config_gate.lock().await;
        let previous = self.read_webdav_backup_config().await?;
        let config = if url.is_empty() {
            WebDavBackupConfig::default()
        } else {
            let normalized_url = webdav_directory_url(&url)?.to_string();
            let same_remote = previous.url == normalized_url && previous.username == username;
            let saved_password = if clear_password {
                None
            } else if password.is_empty() && same_remote {
                previous.password
            } else if password.is_empty() {
                None
            } else {
                Some(password)
            };
            WebDavBackupConfig {
                url: normalized_url,
                username,
                password: saved_password,
                auto_interval_hours: previous.auto_interval_hours,
                last_webdav_backup_at_ms: if same_remote {
                    previous.last_webdav_backup_at_ms
                } else {
                    None
                },
                auto_backup_scheduled_at_ms: if same_remote {
                    previous.auto_backup_scheduled_at_ms
                } else {
                    previous.auto_interval_hours.map(|_| now_ms())
                },
                last_auto_attempt_at_ms: if same_remote {
                    previous.last_auto_attempt_at_ms
                } else {
                    None
                },
            }
        };
        self.write_webdav_backup_config_locked(&config).await?;
        Ok(Self::webdav_backup_config_metadata(&config))
    }

    pub async fn save_webdav_auto_backup_interval(
        &self,
        interval_hours: Option<u32>,
    ) -> Result<Value, String> {
        if interval_hours.is_some_and(|hours| !(1..=720).contains(&hours)) {
            return Err("自动备份间隔必须为 1 到 720 小时".to_owned());
        }
        let _operation = self.operation_read().await;
        let _config = self.webdav_backup_config_gate.lock().await;
        let mut config = self.read_webdav_backup_config().await?;
        if interval_hours.is_some() && config.url.is_empty() {
            return Err("请先保存 WebDAV 连接设置".to_owned());
        }
        if interval_hours != config.auto_interval_hours {
            config.auto_interval_hours = interval_hours;
            config.auto_backup_scheduled_at_ms = interval_hours.map(|_| now_ms());
            config.last_auto_attempt_at_ms = None;
            self.write_webdav_backup_config_locked(&config).await?;
        }
        Ok(Self::webdav_backup_config_metadata(&config))
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

    pub async fn restore_webdav_backup(&self) -> Result<Value, String> {
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

    pub(super) async fn record_backup_success(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let settings_ref = self.store.settings_ref();
        let timestamp = now_ms();
        self.store
            .update_json_ref(&settings_ref, move |mut settings| {
                if !settings.is_object() {
                    return Err("Settings must be a JSON object".to_owned());
                }
                settings["lastBackupAtMs"] = json!(timestamp);
                Ok(settings)
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(self.resource_descriptor(&settings_ref))
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
        self.cancel_active_tasks()
            .await
            .map_err(crate::resource_transactions::TransactionError::new)?;
        let _exclusive = self.operation_gate.clone().write_owned().await;
        if let Err(error) = crate::backup::restore_backup(&self.store, archive).await {
            if error.recovery_required_flag() {
                admission.release_on_drop = false;
            }
            return Err(error);
        }
        if let Err(error) = self.pending_pdf_imports.clear() {
            admission.release_on_drop = false;
            return Err(
                crate::resource_transactions::TransactionError::recovery_required(
                    format!(
                        "Restored data is active, but pending imports could not be cleared: {error}"
                    ),
                    crate::resource_transactions::CommitState::Committed,
                ),
            );
        }
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
        self.reload_tasks_after_restore().await.map_err(|error| {
            admission.release_on_drop = false;
            crate::resource_transactions::TransactionError::recovery_required(
                format!("Restored data is active, but task history could not be reloaded: {error}"),
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

    async fn reload_tasks_after_restore(&self) -> Result<(), String> {
        let reference = self
            .store
            .reading_ref("tasks")
            .map_err(|error| error.to_string())?;
        let document = if self.store.regular_file_exists(&reference).await
            .map_err(|error| format!("Cannot inspect restored task history: {error}"))?
        {
            self.store.read_json_ref(&reference).await
                .map_err(|error| format!("Cannot read restored task history: {error}"))?
        } else {
            // A backup may not contain optional task history. Missing is
            // different from corrupt: never overwrite a damaged history
            // with an empty record set and silently claim restore succeeded.
            json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "tasks": [] })
        };
        let restored_tasks = document.get("tasks").cloned()
            .ok_or_else(|| "Restored task history is missing its tasks array".to_owned())?;
        let mut records = serde_json::from_value::<Vec<AppTask>>(restored_tasks)
            .map_err(|error| format!("Cannot load restored task history: {error}"))?;
        for task in &mut records {
            if matches!(task.kind.as_str(), "search" | "bookSourceCandidates") {
                // Search display versions are not included in backups.
                task.result = None;
            }
            // A persisted user stop request remains effective after process loss.
            let recovered_status = match task.status.as_str() {
                "cancelling" => Some("cancelled"),
                "pausing" => Some("paused"),
                "queued" | "running" => Some("interrupted"),
                _ => None,
            };
            if let Some(status) = recovered_status {
                task.status = status.to_owned();
                task.updated_at_ms = now_ms();
            }
            if task.kind == "bookSourceCandidates"
                && !matches!(
                    task.status.as_str(),
                    "completed" | "failed" | "cancelled" | "recoveryRequired"
                )
            {
                task.status = "failed".into();
                task.error = Some(
                    "Candidate search results are session-only and cannot resume after restore; retry the search.".into(),
                );
                task.updated_at_ms = now_ms();
            }
            if task.kind == "search"
                && !matches!(task.status.as_str(), "completed" | "failed" | "cancelled" | "recoveryRequired")
            {
                // Discovery results are session memory. Preserve pause intent,
                // but restart resumable work from the first source.
                if !matches!(task.status.as_str(), "paused" | "interrupted") {
                    task.status = "interrupted".into();
                }
                task.completed = 0;
                task.error = None;
                task.updated_at_ms = now_ms();
            }
        }
        self.store
            .write_json_ref(
                &reference,
                &json!({
                    "schemaVersion": CURRENT_SCHEMA_VERSION,
                    "tasks": records,
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
        let mut registry = self.tasks.lock().await;
        registry.records = records;
        registry.signals.clear();
        registry.search_documents = registry
            .records
            .iter()
            .filter(|task| {
                task.kind == "search" && matches!(task.status.as_str(), "paused" | "interrupted")
            })
            .filter_map(|task| {
                task.search_id
                    .as_ref()
                    .map(|search_id| (search_id.clone(), empty_search_task_document(task)))
            })
            .collect();
        registry.search_result_documents.clear();
        registry.search_replacement_contexts.clear();
        registry.search_book_group_roots.clear();
        // Restored tasks have no running workers. Search publication
        // ownership never survives a backup restore.
        registry.active_search_task_id = None;
        Ok(())
    }

    async fn cancel_active_tasks(&self) -> Result<(), String> {
        let changed = {
            let mut registry = self.tasks.lock().await;
            // Reject recovery-needed history before touching any records.
            // This state cannot safely be turned into a normal cancellation.
            if registry.records.iter().any(|task| task.status == "recoveryRequired") {
                return Err("A task requires crash recovery before restoring a backup".into());
            }
            let previous = registry.records.clone();
            let signals = registry.signals.clone();
            let mut changed = Vec::new();
            for task in &mut registry.records {
                if matches!(task.status.as_str(), "completed" | "failed" | "cancelled") {
                    continue;
                }
                task.status = if signals.contains_key(&task.id) {
                    "cancelling".into()
                } else {
                    "cancelled".into()
                };
                task.updated_at_ms = now_ms();
                changed.push(task.clone());
            }
            if !changed.is_empty() {
                if let Err(error) = self.persist_tasks_locked(&registry).await {
                    registry.records = previous;
                    return Err(error);
                }
                // Never signal a running task until its new stop state is
                // durably committed. A failed restore admission leaves the
                // in-memory and on-disk records unchanged.
                for task in &changed {
                    if let Some(sender) = signals.get(&task.id) {
                        sender.send_modify(|signal| signal.cancelled = true);
                    }
                }
            }
            changed
        };
        for task in &changed {
            self.notify_task(task).await;
        }
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if self.tasks.lock().await.signals.is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| {
            "Timed out waiting for active tasks to stop; backup restore was not applied".to_owned()
        })
    }
}
