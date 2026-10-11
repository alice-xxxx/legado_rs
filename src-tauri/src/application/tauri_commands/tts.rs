//! HTTP TTS Tauri 命令。

use super::*;

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
