//! HTTP TTS 配置和音频资源准备。

use super::*;
use crate::resources::{TtsStreamBoundary, TtsStreamProgress};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpTtsStreamChunk {
    text: String,
    highlight_index: usize,
}

impl ApplicationService {
    async fn prepared_http_tts_bytes(&self, result: &Value) -> Result<Vec<u8>, String> {
        let id = result["audioId"]
            .as_str()
            .ok_or("HTTP TTS audio ID is missing")?;
        let extension = result["resource"]["resourceId"]
            .as_str()
            .and_then(|reference| reference.rsplit('.').next())
            .ok_or("HTTP TTS audio format is missing")?;
        let bytes = self
            .store
            .read_tts_audio(id, extension)
            .await
            .map_err(|error| error.to_string());
        let _ = self.release_http_tts_audio(id).await;
        bytes
    }

    pub async fn start_http_tts_stream(
        &self,
        config_id: String,
        chunks: Vec<HttpTtsStreamChunk>,
        speech_rate: u32,
    ) -> Result<Value, String> {
        if chunks.is_empty() || chunks.len() > 2048 {
            return Err("HTTP TTS chapter must contain 1–2048 segments".to_owned());
        }
        if chunks.iter().any(|chunk| {
            chunk.text.trim().is_empty()
                || chunk.text.chars().count() > MAX_HTTP_TTS_TEXT_CHARS
                || chunk.highlight_index >= chunks.len()
        }) {
            return Err("HTTP TTS chapter contains an invalid segment".to_owned());
        }
        if chunks.iter().map(|chunk| chunk.text.len()).sum::<usize>() > 8 * 1024 * 1024 {
            return Err("HTTP TTS chapter text exceeds 8 MiB".to_owned());
        }
        let mut first_index = 0;
        let first = loop {
            let result = self
                .request_http_tts_audio(&config_id, &chunks[first_index].text, speech_rate)
                .await?;
            if result["status"] != "empty" {
                break result;
            }
            first_index += 1;
            if first_index == chunks.len() {
                return Ok(json!({ "status": "empty", "skipped": first_index }));
            }
        };
        if first["contentType"] != "audio/mpeg" {
            return Ok(json!({ "status": "fallback", "firstIndex": first_index, "first": first }));
        }
        let first_audio_id = first["audioId"]
            .as_str()
            .ok_or("HTTP TTS audio ID is missing")?;
        let bytes = match self.store.read_tts_audio(first_audio_id, "mp3").await {
            Ok(bytes) => bytes,
            Err(_) => {
                return Ok(
                    json!({ "status": "fallback", "firstIndex": first_index, "first": first }),
                );
            }
        };
        let Ok((payload, duration, profile)) = mp3_stream_payload(&bytes) else {
            // This response claimed to be MP3 but cannot be safely joined to a stream.
            return Ok(json!({ "status": "fallback", "firstIndex": first_index, "first": first }));
        };
        let initial = TtsStreamProgress {
            boundaries: vec![TtsStreamBoundary {
                index: first_index,
                highlight_index: chunks[first_index].highlight_index,
                end_seconds: duration,
            }],
            state: "running",
            next_index: first_index + 1,
            skipped: first_index,
            error: None,
        };
        let writer = match self.server.create_tts_stream(initial.clone()).await {
            Ok(writer) => writer,
            Err(_) => {
                return Ok(
                    json!({ "status": "fallback", "firstIndex": first_index, "first": first }),
                );
            }
        };
        let stream_id = writer.id.clone();
        let producer_id = stream_id.clone();
        let stream_url = writer.url.clone();
        if writer.sender.send(Ok(payload.to_vec())).await.is_err() {
            self.server.release_tts_stream(&stream_id).await;
            return Ok(json!({ "status": "fallback", "firstIndex": first_index, "first": first }));
        }
        let _ = self.release_http_tts_audio(first_audio_id).await;
        let service = self.clone();
        tokio::spawn(async move {
            let mut progress = initial;
            for (index, chunk) in chunks.into_iter().enumerate().skip(first_index + 1) {
                if *writer.cancellation.borrow() {
                    break;
                }
                let result = tokio::select! {
                    result = service.request_http_tts_audio(&config_id, &chunk.text, speech_rate) => result,
                    _ = writer.sender.closed() => break,
                    _ = wait_tts_cancel(writer.cancellation.clone()) => break,
                };
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        progress.state = "error";
                        progress.error = Some(error.clone());
                        service
                            .server
                            .update_tts_stream(&producer_id, progress)
                            .await;
                        let _ = writer.sender.send(Err(std::io::Error::other(error))).await;
                        return;
                    }
                };
                if result["status"] == "empty" {
                    progress.skipped += 1;
                    progress.next_index = index + 1;
                    service
                        .server
                        .update_tts_stream(&producer_id, progress.clone())
                        .await;
                    continue;
                }
                if result["contentType"] != "audio/mpeg" {
                    if let Some(id) = result["audioId"].as_str() {
                        let _ = service.release_http_tts_audio(id).await;
                    }
                    progress.state = "fallback";
                    progress.next_index = index;
                    service
                        .server
                        .update_tts_stream(&producer_id, progress)
                        .await;
                    return;
                }
                let bytes = match service.prepared_http_tts_bytes(&result).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        progress.state = "error";
                        progress.error = Some(error.clone());
                        service
                            .server
                            .update_tts_stream(&producer_id, progress)
                            .await;
                        let _ = writer.sender.send(Err(std::io::Error::other(error))).await;
                        return;
                    }
                };
                let Ok((payload, duration, next_profile)) = mp3_stream_payload(&bytes) else {
                    progress.state = "fallback";
                    progress.next_index = index;
                    service
                        .server
                        .update_tts_stream(&producer_id, progress)
                        .await;
                    return;
                };
                if next_profile != profile {
                    progress.state = "fallback";
                    progress.next_index = index;
                    service
                        .server
                        .update_tts_stream(&producer_id, progress)
                        .await;
                    return;
                }
                let previous = progress
                    .boundaries
                    .last()
                    .map_or(0.0, |item| item.end_seconds);
                progress.boundaries.push(TtsStreamBoundary {
                    index,
                    highlight_index: chunk.highlight_index,
                    end_seconds: previous + duration,
                });
                progress.next_index = index + 1;
                service
                    .server
                    .update_tts_stream(&producer_id, progress.clone())
                    .await;
                if writer.sender.send(Ok(payload.to_vec())).await.is_err() {
                    return;
                }
            }
            if !*writer.cancellation.borrow() {
                progress.state = "complete";
                service
                    .server
                    .update_tts_stream(&producer_id, progress)
                    .await;
            }
        });
        Ok(
            json!({ "status": "stream", "streamId": stream_id, "src": stream_url,
            "firstIndex": first_index, "skipped": first_index }),
        )
    }

    pub async fn get_http_tts_stream_status(&self, id: &str) -> Result<Value, String> {
        let progress = self
            .server
            .tts_stream_progress(id)
            .await
            .ok_or("HTTP TTS stream was released")?;
        serde_json::to_value(progress).map_err(|error| error.to_string())
    }

    pub async fn release_http_tts_stream(&self, id: &str) -> bool {
        self.server.release_tts_stream(id).await
    }
    async fn read_http_tts_configs(&self) -> Result<Value, String> {
        self.store
            .read_json_ref(&self.store.http_tts_configs_ref())
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn request_http_tts_audio(
        &self,
        config_id: &str,
        text: &str,
        speech_rate: u32,
    ) -> Result<Value, String> {
        if text.trim().is_empty()
            || text.chars().count() > MAX_HTTP_TTS_TEXT_CHARS
            || text.chars().any(|character| {
                character == '\0'
                    || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
            })
        {
            return Err(
                "HTTP TTS text must contain 1–20000 characters without invalid control characters"
                    .to_owned(),
            );
        }
        if speech_rate > 45 {
            return Err("HTTP TTS speech rate must be between 0 and 45".to_owned());
        }
        let _operation = self.operation_read().await;
        let mut restore_barrier = self.restore_barrier.subscribe();
        let restore_epoch = self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst);
        if *restore_barrier.borrow() {
            return Err("HTTP TTS request cancelled because app data is being restored".to_owned());
        }
        let config = {
            let _lock = self.sources_lock.lock().await;
            let document = self.read_http_tts_configs().await?;
            crate::http_tts_config::find_config(&document, config_id)?
        };
        let execution = self.executor.execute(engine_request(
            "httpTtsAudio",
            &config,
            Some(text.to_owned()),
            Some(speech_rate as i32),
            None,
            None,
            None,
        ));
        let result = tokio::select! {
            result = execution => match result {
                Ok(result) => result,
                Err(error) if error.contains("HTTP TTS returned an empty audio response") => {
                    return Ok(json!({ "status": "empty" }));
                }
                Err(error) => return Err(error),
            },
            _ = restore_barrier.changed() => {
                return Err("HTTP TTS request cancelled because app data is being restored".to_owned());
            }
        };
        if self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst) != restore_epoch
            || *restore_barrier.borrow()
        {
            return Err("HTTP TTS request cancelled because app data is being restored".to_owned());
        }
        let encoded = result
            .get("bytesBase64")
            .and_then(Value::as_str)
            .ok_or_else(|| "Source engine returned no HTTP TTS audio".to_owned())?;
        if encoded.len() > MAX_HTTP_TTS_AUDIO_BASE64_BYTES {
            return Err("HTTP TTS audio exceeds the 16 MiB limit".to_owned());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "Source engine returned invalid HTTP TTS audio".to_owned())?;
        if bytes.is_empty() {
            return Ok(json!({ "status": "empty" }));
        }
        if bytes.len() > MAX_HTTP_TTS_AUDIO_BYTES {
            return Err("HTTP TTS audio exceeds the 16 MiB limit".to_owned());
        }
        let reported_content_type = result
            .get("contentType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (extension, content_type) = http_tts_audio_type(reported_content_type, &bytes)?;
        let audio_id = uuid::Uuid::new_v4().simple().to_string();
        let _config_lock = self.sources_lock.lock().await;
        let current_document = self.read_http_tts_configs().await?;
        if crate::http_tts_config::find_config(&current_document, config_id)? != config {
            return Err(
                "HTTP TTS configuration was removed while audio was being generated".to_owned(),
            );
        }
        if self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst) != restore_epoch
            || *restore_barrier.borrow()
        {
            return Err("HTTP TTS request cancelled because app data is being restored".to_owned());
        }
        let resource = self
            .store
            .write_tts_audio(&audio_id, extension, &bytes)
            .await
            .map_err(|error| error.to_string())?;
        if self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst) != restore_epoch
            || *restore_barrier.borrow()
        {
            let _ = self.store.remove_tts_audio(&audio_id).await;
            return Err("HTTP TTS request cancelled because app data is being restored".to_owned());
        }
        Ok(json!({
            "status": "ready",
            "audioId": audio_id,
            "resource": self.resource_descriptor(&resource),
            "contentType": content_type,
        }))
    }

    pub async fn release_http_tts_audio(&self, audio_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let released = self
            .store
            .remove_tts_audio(audio_id)
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({ "released": released }))
    }
}

async fn wait_tts_cancel(mut cancellation: tokio::sync::watch::Receiver<bool>) {
    while !*cancellation.borrow() && cancellation.changed().await.is_ok() {}
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Mp3StreamProfile {
    version: u32,
    sample_rate: usize,
    mono: bool,
}

/// Locate MPEG Layer III frames and their decoded time; remove per-file tags before joining.
fn mp3_stream_payload(bytes: &[u8]) -> Result<(&[u8], f64, Mp3StreamProfile), ()> {
    let mut cursor = 0usize;
    if bytes.starts_with(b"ID3") {
        if bytes.len() < 10 || bytes[6..10].iter().any(|byte| byte & 0x80 != 0) {
            return Err(());
        }
        let size = ((bytes[6] as usize) << 21)
            | ((bytes[7] as usize) << 14)
            | ((bytes[8] as usize) << 7)
            | bytes[9] as usize;
        cursor = 10 + size + if bytes[5] & 0x10 != 0 { 10 } else { 0 };
        if cursor > bytes.len() {
            return Err(());
        }
    }
    let start = cursor;
    let mut duration = 0.0;
    let mut frames = 0;
    let mut profile = None;
    while cursor + 4 <= bytes.len() {
        let header = u32::from_be_bytes(bytes[cursor..cursor + 4].try_into().map_err(|_| ())?);
        if header >> 21 != 0x7ff {
            break;
        }
        let version = (header >> 19) & 3;
        let layer = (header >> 17) & 3;
        let bitrate_index = ((header >> 12) & 15) as usize;
        let sample_index = ((header >> 10) & 3) as usize;
        if version == 1
            || layer != 1
            || bitrate_index == 0
            || bitrate_index == 15
            || sample_index == 3
        {
            break;
        }
        let bitrate = if version == 3 {
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ][bitrate_index]
        } else {
            [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][bitrate_index]
        };
        let sample_rate = [44100, 48000, 32000][sample_index]
            / match version {
                3 => 1,
                2 => 2,
                _ => 4,
            };
        let frame_profile = Mp3StreamProfile {
            version,
            sample_rate,
            mono: (header >> 6) & 3 == 3,
        };
        if profile.is_some_and(|previous| previous != frame_profile) {
            return Err(());
        }
        profile = Some(frame_profile);
        let frame_size = (if version == 3 { 144_000 } else { 72_000 }) * bitrate / sample_rate
            + ((header >> 9) & 1) as usize;
        if cursor + frame_size > bytes.len() {
            break;
        }
        cursor += frame_size;
        duration += (if version == 3 { 1152.0 } else { 576.0 }) / sample_rate as f64;
        frames += 1;
    }
    if frames < 2 || bytes.len() - cursor > 256 {
        return Err(());
    }
    Ok((&bytes[start..cursor], duration, profile.ok_or(())?))
}

#[cfg(test)]
mod tts_stream_tests {
    use super::mp3_stream_payload;

    #[test]
    fn strips_id3_and_counts_mpeg_frames_for_stream_boundaries() {
        // MPEG-1 Layer III, 128 kb/s, 44.1 kHz: 417 bytes per frame.
        let mut bytes = b"ID3\x04\0\0\0\0\0\0".to_vec();
        for _ in 0..3 {
            let mut frame = vec![0; 417];
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
            bytes.extend(frame);
        }
        let (payload, duration, _) = mp3_stream_payload(&bytes).unwrap();
        assert_eq!(payload.len(), 3 * 417);
        assert!((duration - 3.0 * 1152.0 / 44100.0).abs() < 0.0001);
    }

    #[test]
    fn rejects_container_bytes_that_cannot_be_joined_as_mp3() {
        assert!(mp3_stream_payload(b"RIFF\0\0\0\0WAVE").is_err());
        assert!(mp3_stream_payload(b"ID3\x04\0\0\xff\0\0\0").is_err());
        assert!(mp3_stream_payload(b"ID3\x04\0\0\x7f\x7f\x7f\x7f").is_err());
    }
}
