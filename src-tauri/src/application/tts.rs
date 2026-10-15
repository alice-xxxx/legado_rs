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
        let epoch = self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst);
        let config = {
            let _lock = self.sources_lock.lock().await;
            crate::http_tts_config::find_config(&self.read_http_tts_configs().await?, &config_id)?
        };
        let mut first_index = 0;
        let first = loop {
            let result = self
                .request_http_tts_audio_for_config(
                    &config_id,
                    &chunks[first_index].text,
                    speech_rate,
                    Some(&config),
                )
                .await?;
            if result["status"] != "empty" {
                break result;
            }
            first_index += 1;
            if first_index == chunks.len() {
                return Ok(json!({ "status": "empty", "skipped": first_index }));
            }
        };
        let fallback =
            || json!({ "status": "fallback", "firstIndex": first_index, "first": first });
        if first["contentType"] != "audio/mpeg" {
            return Ok(fallback());
        }
        let first_audio_id = first["audioId"]
            .as_str()
            .ok_or("HTTP TTS audio ID is missing")?;
        let bytes = match self.store.read_tts_audio(first_audio_id, "mp3").await {
            Ok(bytes) => bytes,
            Err(_) => return Ok(fallback()),
        };
        let Ok((payload, duration, profile)) = mp3_stream_payload(&bytes) else {
            return Ok(fallback());
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
        let mut writer = match self.server.create_tts_stream(initial.clone()).await {
            Ok(writer) => writer,
            Err(_) => return Ok(fallback()),
        };
        let stream_id = writer.id.clone();
        if writer.append(payload).await.is_err() {
            self.server.release_tts_stream(&stream_id).await;
            return Ok(fallback());
        }
        let _ = self.release_http_tts_audio(first_audio_id).await;
        if self.restore_epoch.load(std::sync::atomic::Ordering::SeqCst) != epoch {
            self.server.release_tts_stream(&stream_id).await;
            return Err("HTTP TTS chapter cancelled because app data was restored".into());
        }
        let stream_url = writer.url.clone();
        let service = self.clone();
        tokio::spawn(async move {
            let mut progress = initial;
            for (index, chunk) in chunks.into_iter().enumerate().skip(first_index + 1) {
                if *writer.cancellation.borrow()
                    || service
                        .restore_epoch
                        .load(std::sync::atomic::Ordering::SeqCst)
                        != epoch
                {
                    service.server.release_tts_stream(&writer.id).await;
                    return;
                }
                let result = tokio::select! {
                    result = service.request_http_tts_audio_for_config(&config_id, &chunk.text, speech_rate, Some(&config)) => result,
                    _ = wait_tts_cancel(writer.cancellation.clone()) => return,
                };
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        progress.state = "error";
                        progress.error = Some(error);
                        break;
                    }
                };
                if result["status"] == "empty" {
                    progress.skipped += 1;
                    progress.next_index = index + 1;
                    service
                        .server
                        .update_tts_stream(&writer.id, progress.clone())
                        .await;
                    continue;
                }
                if result["contentType"] != "audio/mpeg" {
                    if let Some(id) = result["audioId"].as_str() {
                        let _ = service.release_http_tts_audio(id).await;
                    }
                    progress.state = "fallback";
                    progress.next_index = index;
                    break;
                }
                let bytes = match service.prepared_http_tts_bytes(&result).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        progress.state = "error";
                        progress.error = Some(error);
                        break;
                    }
                };
                let Ok((payload, duration, next_profile)) = mp3_stream_payload(&bytes) else {
                    progress.state = "fallback";
                    progress.next_index = index;
                    break;
                };
                if next_profile != profile {
                    progress.state = "fallback";
                    progress.next_index = index;
                    break;
                }
                if let Err(error) = writer.append(payload).await {
                    progress.state = "error";
                    progress.error = Some(error.to_string());
                    break;
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
                    .update_tts_stream(&writer.id, progress.clone())
                    .await;
            }
            if progress.state == "running" {
                progress.state = "complete";
            }
            service.server.update_tts_stream(&writer.id, progress).await;
            // Dropping the writer closes every HTTP reader after its buffered tail.
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
        self.request_http_tts_audio_for_config(config_id, text, speech_rate, None)
            .await
    }

    async fn request_http_tts_audio_for_config(
        &self,
        config_id: &str,
        text: &str,
        speech_rate: u32,
        expected: Option<&Value>,
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
        if expected.is_some_and(|expected| expected != &config) {
            return Err("HTTP TTS configuration changed during chapter playback".into());
        }
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
    let mut start = cursor;
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
        // Per-file Xing/Info/VBRI metadata advertises only that file's length.
        // Keeping it makes native players stop at the first generated segment.
        let side_info = if version == 3 {
            if frame_profile.mono {
                17
            } else {
                32
            }
        } else if frame_profile.mono {
            9
        } else {
            17
        };
        let crc = if (header >> 16) & 1 == 0 { 2 } else { 0 };
        let xing = cursor + 4 + crc + side_info;
        let vbri = cursor + 36;
        let metadata = frames == 0
            && (matches!(bytes.get(xing..xing + 4), Some(b"Xing" | b"Info"))
                || bytes.get(vbri..vbri + 4) == Some(b"VBRI"));
        cursor += frame_size;
        if metadata {
            start = cursor;
            continue;
        }
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
    use super::{
        ApplicationService, EngineFuture, HttpTtsStreamChunk, SourceEngineRequest, SourceExecutor,
    };
    use base64::Engine;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct ChapterExecutor(Arc<Mutex<Vec<String>>>);
    impl SourceExecutor for ChapterExecutor {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                assert_eq!(request.operation, "httpTtsAudio");
                let text = request.keyword.unwrap();
                self.0.lock().unwrap().push(text.clone());
                if text == "skip" {
                    return Err("HTTP TTS returned an empty audio response".into());
                }
                let mut bytes = Vec::new();
                for _ in 0..3 {
                    let mut frame = vec![0; 417];
                    frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
                    bytes.extend(frame);
                }
                Ok(
                    json!({"bytesBase64": base64::engine::general_purpose::STANDARD.encode(bytes), "contentType": "audio/mpeg"}),
                )
            })
        }
    }

    #[tokio::test]
    async fn rust_generates_one_chapter_stream_with_ordered_highlight_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let service = ApplicationService::open_with_executor(
            directory.path(),
            Arc::new(ChapterExecutor(calls.clone())),
        )
        .await
        .unwrap();
        let (_, entry) = crate::http_tts_config::import_configs(
            r#"{"name":"Test","url":"https://example.invalid/tts"}"#,
        )
        .unwrap()
        .remove(0);
        let config_id = entry["id"].as_str().unwrap().to_owned();
        service
            .store
            .write_json_ref(
                &service.store.http_tts_configs_ref(),
                &json!({"schemaVersion":1,"configs":[entry]}),
            )
            .await
            .unwrap();
        let result = service
            .start_http_tts_stream(
                config_id,
                vec![
                    HttpTtsStreamChunk {
                        text: "skip".into(),
                        highlight_index: 1,
                    },
                    HttpTtsStreamChunk {
                        text: "second".into(),
                        highlight_index: 2,
                    },
                    HttpTtsStreamChunk {
                        text: "third".into(),
                        highlight_index: 0,
                    },
                ],
                5,
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "stream");
        assert_eq!(result["firstIndex"], 1);
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let src = result["src"].as_str().unwrap();
        let unauthorized = format!(
            "{}/r/invalid/tts-stream/{}.mp3",
            src.split("/r/").next().unwrap(),
            result["streamId"].as_str().unwrap()
        );
        assert_eq!(
            client.get(unauthorized).send().await.unwrap().status(),
            reqwest::StatusCode::NOT_FOUND
        );
        let response = client
            .get(result["src"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let bytes = tokio::time::timeout(std::time::Duration::from_secs(2), response.bytes())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bytes.len(), 6 * 417);
        let replay = client
            .get(src)
            .header(reqwest::header::RANGE, "bytes=0-3")
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            replay.bytes().await.unwrap().as_ref(),
            &[0xff, 0xfb, 0x90, 0x64]
        );
        let status = service
            .get_http_tts_stream_status(result["streamId"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(status["state"], "complete");
        assert_eq!(status["skipped"], 1);
        assert_eq!(status["boundaries"][0]["highlightIndex"], 2);
        assert_eq!(status["boundaries"][1]["highlightIndex"], 0);
        assert!(
            status["boundaries"][1]["endSeconds"].as_f64().unwrap()
                > status["boundaries"][0]["endSeconds"].as_f64().unwrap()
        );
        assert_eq!(*calls.lock().unwrap(), ["skip", "second", "third"]);
        service
            .release_http_tts_stream(result["streamId"].as_str().unwrap())
            .await;
    }

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

    #[test]
    fn removes_per_file_duration_frames_before_joining_chapter_audio() {
        let mut bytes = Vec::new();
        for index in 0..3 {
            let mut frame = vec![0; 417];
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
            if index == 0 {
                frame[36..40].copy_from_slice(b"Info");
            }
            bytes.extend(frame);
        }
        let (payload, duration, _) = mp3_stream_payload(&bytes).unwrap();
        assert_eq!(payload.len(), 2 * 417);
        assert!((duration - 2.0 * 1152.0 / 44100.0).abs() < 0.0001);
    }
}
