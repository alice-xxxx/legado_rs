//! HTTP TTS Tauri 命令。

use super::*;

#[tauri::command]
pub async fn mutate_http_tts_configs(
    mutation: crate::http_tts_config::HttpTtsConfigOperation,
    app: tauri::AppHandle,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let mutation = match mutation {
        crate::http_tts_config::HttpTtsConfigOperation::Import(
            crate::http_tts_config::HttpTtsConfigImportSource::Local,
        ) => {
            let Some(file) = pick_file_with_limit(
                &app,
                "HTTP TTS JSON",
                &["json"],
                crate::http_tts_config::MAX_CONFIG_BYTES as u64,
            )
            .await?
            else {
                return Ok(json!({ "cancelled": true }));
            };
            let config_json = tokio::fs::read_to_string(&file.path)
                .await
                .map_err(|error| format!("Cannot read selected HttpTTS file: {error}"))?;
            crate::http_tts_config::HttpTtsConfigMutation::Save {
                id: None,
                json: config_json
                    .strip_prefix('\u{feff}')
                    .unwrap_or(&config_json)
                    .to_owned(),
            }
        }
        crate::http_tts_config::HttpTtsConfigOperation::Import(
            crate::http_tts_config::HttpTtsConfigImportSource::Url { url },
        ) => {
            let config_json = service.fetch_http_tts_config_json(&url).await?;
            crate::http_tts_config::HttpTtsConfigMutation::Save {
                id: None,
                json: config_json,
            }
        }
        crate::http_tts_config::HttpTtsConfigOperation::Mutation(mutation) => mutation,
    };
    service.mutate_http_tts_configs(mutation).await
}

#[tauri::command]
pub async fn request_http_tts_audio(
    config_id: String,
    text: String,
    speech_rate: u32,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .request_http_tts_audio(&config_id, &text, speech_rate)
        .await
}

#[tauri::command]
pub async fn release_http_tts_audio(
    audio_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.release_http_tts_audio(&audio_id).await
}
