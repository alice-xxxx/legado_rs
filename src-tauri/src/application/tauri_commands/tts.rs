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

#[tauri::command]
pub async fn start_http_tts_stream(
    config_id: String,
    chunks: Vec<crate::application::tts::HttpTtsStreamChunk>,
    speech_rate: u32,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service
        .start_http_tts_stream(config_id, chunks, speech_rate)
        .await
}

#[tauri::command]
pub async fn get_http_tts_stream_status(
    stream_id: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    service.get_http_tts_stream_status(&stream_id).await
}

#[tauri::command]
pub async fn release_http_tts_stream(
    stream_id: String,
    service: State<'_, ApplicationService>,
) -> Result<bool, String> {
    Ok(service.release_http_tts_stream(&stream_id).await)
}
