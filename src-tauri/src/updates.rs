//! One published manifest drives desktop signed updates and Android APK installation.
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
#[cfg(target_os = "android")]
use tauri::Manager;
use tauri::{ipc::Channel, AppHandle, State};
use tokio::sync::Mutex;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: String,
    page: String,
    kind: &'static str,
}

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    system: &'static str,
    arch: &'static str,
}

#[tauri::command]
pub fn get_app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        system: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

struct Pending {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    info: UpdateInfo,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    desktop: tauri_plugin_updater::Update,
    #[cfg(target_os = "android")]
    package: Value,
}

#[derive(Default)]
pub struct Updates(Mutex<Option<Pending>>);

#[derive(Clone, Serialize)]
pub struct Progress {
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

fn configuration(app: &AppHandle) -> Result<&Value, String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .ok_or("未配置应用更新".into())
}

#[tauri::command]
pub async fn check_app_update(
    app: AppHandle,
    state: State<'_, Updates>,
) -> Result<Option<UpdateInfo>, String> {
    let mut pending = state.0.try_lock().map_err(|_| "更新任务正在进行")?;
    let endpoint = configuration(&app)?
        .pointer("/endpoints/0")
        .and_then(Value::as_str)
        .ok_or("未配置更新地址")?;
    let response = crate::http_client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?
        .get(endpoint)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        *pending = None;
        return Ok(None);
    }
    let bytes = response
        .error_for_status()
        .map_err(|error| error.to_string())?
        .bytes()
        .await
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("更新说明超过大小限制".into());
    }
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let version: semver::Version = manifest["version"]
        .as_str()
        .ok_or("更新版本缺失")?
        .parse()
        .map_err(|error: semver::Error| error.to_string())?;
    if version.patch != 0 || !version.pre.is_empty() || !version.build.is_empty() {
        return Err("更新来源不是正式发版版本".into());
    }
    if version <= app.package_info().version {
        *pending = None;
        return Ok(None);
    }
    let info = UpdateInfo {
        page: format!(
            "{}/releases/tag/v{version}",
            endpoint.split("/releases/").next().ok_or("更新地址无效")?
        ),
        version: version.to_string(),
        notes: manifest["notes"].as_str().unwrap_or_default().to_owned(),
        kind: if cfg!(target_os = "android") {
            "android"
        } else if cfg!(target_os = "ios") {
            "ios"
        } else {
            "desktop"
        },
    };
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let next = {
        use tauri_plugin_updater::UpdaterExt;
        let desktop = app
            .updater_builder()
            .timeout(Duration::from_secs(1800))
            .build()
            .map_err(|error| error.to_string())?
            .restore_update(manifest)
            .map_err(|error| error.to_string())?
            .ok_or("没有适用的更新包")?;
        Pending { desktop }
    };
    #[cfg(target_os = "android")]
    let next = {
        let arch = match std::env::consts::ARCH {
            "arm" => "armv7",
            "x86" => "i686",
            arch => arch,
        };
        let package = manifest["platforms"][format!("android-{arch}")].clone();
        if !package.is_object() {
            return Err("此架构尚无更新包".into());
        }
        Pending {
            info: info.clone(),
            package,
        }
    };
    #[cfg(target_os = "ios")]
    let next = Pending { info: info.clone() };
    *pending = Some(next);
    Ok(Some(info))
}

#[tauri::command]
pub async fn install_app_update(
    _app: AppHandle,
    state: State<'_, Updates>,
    progress: Channel<Progress>,
) -> Result<String, String> {
    let pending = state.0.try_lock().map_err(|_| "更新任务正在进行")?;
    let update = pending.as_ref().ok_or("请先检查更新")?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let mut downloaded = 0;
        update
            .desktop
            .download_and_install(
                |count, total| {
                    downloaded += count as u64;
                    let _ = progress.send(Progress {
                        phase: "downloading",
                        downloaded,
                        total,
                    });
                },
                || {
                    let _ = progress.send(Progress {
                        phase: "installing",
                        downloaded: 0,
                        total: None,
                    });
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok("restart".into())
    }
    #[cfg(target_os = "android")]
    {
        download_apk(&_app, update, &progress).await?;
        let permission = tauri_plugin_update_installer::install(&_app).await?;
        Ok(if permission {
            "permission"
        } else {
            "installer"
        }
        .into())
    }
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_opener::OpenerExt;
        let _ = progress;
        _app.opener()
            .open_url(update.info.page.clone(), None::<&str>)
            .map_err(|error| error.to_string())?;
        Ok("browser".into())
    }
}

#[tauri::command]
pub fn restart_after_update(app: AppHandle) {
    app.restart();
}

#[cfg(target_os = "android")]
fn verify_apk(app: &AppHandle, update: &Pending, bytes: &[u8]) -> Result<(), String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sha2::{Digest, Sha256};
    let decode = |encoded: &str| -> Result<String, String> {
        String::from_utf8(
            STANDARD
                .decode(encoded)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    };
    let public = decode(
        configuration(app)?["pubkey"]
            .as_str()
            .ok_or("更新公钥缺失")?,
    )?;
    let signature = decode(update.package["signature"].as_str().ok_or("更新签名缺失")?)?;
    let signature =
        minisign_verify::Signature::decode(&signature).map_err(|error| error.to_string())?;
    minisign_verify::PublicKey::decode(&public)
        .map_err(|error| error.to_string())?
        .verify(bytes, &signature, true)
        .map_err(|error| error.to_string())?;
    let signed_version = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"));
    if signed_version != Some(update.info.version.as_str()) {
        return Err("安装包签名与更新版本不一致".into());
    }
    if update.package["size"].as_u64() != Some(bytes.len() as u64)
        || update.package["sha256"].as_str()
            != Some(format!("{:x}", Sha256::digest(bytes)).as_str())
    {
        return Err("安装包完整性校验失败，请重新下载".into());
    }
    Ok(())
}

#[cfg(target_os = "android")]
async fn download_apk(
    app: &AppHandle,
    update: &Pending,
    progress: &Channel<Progress>,
) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let directory = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("foliver-updates");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let destination = directory.join("update.apk");
    if let Ok(bytes) = tokio::fs::read(&destination).await {
        if verify_apk(app, update, &bytes).is_ok() {
            return Ok(());
        }
    }
    let size = update.package["size"]
        .as_u64()
        .filter(|size| *size > 0 && *size <= 512 * 1024 * 1024)
        .ok_or("安装包大小无效")?;
    let url = update.package["url"].as_str().ok_or("更新包地址缺失")?;
    if !url.starts_with("https://github.com/") {
        return Err("更新包地址无效".into());
    }
    let response = crate::http_client::builder()
        .timeout(Duration::from_secs(1800))
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let partial = directory.join("update.part");
    let mut file = tokio::fs::File::create(&partial)
        .await
        .map_err(|error| error.to_string())?;
    let mut stream = response.bytes_stream();
    let mut downloaded = 0;
    while let Some(bytes) = stream.next().await {
        let bytes = bytes.map_err(|error| error.to_string())?;
        downloaded += bytes.len() as u64;
        if downloaded > size {
            return Err("下载内容超过安装包大小".into());
        }
        file.write_all(&bytes)
            .await
            .map_err(|error| error.to_string())?;
        let _ = progress.send(Progress {
            phase: "downloading",
            downloaded,
            total: Some(size),
        });
    }
    file.flush().await.map_err(|error| error.to_string())?;
    drop(file);
    let bytes = tokio::fs::read(&partial)
        .await
        .map_err(|error| error.to_string())?;
    verify_apk(app, update, &bytes)?;
    tokio::fs::rename(partial, destination)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}
