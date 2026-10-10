//! HTTP TTS 配置和音频资源准备。

use super::*;

impl ApplicationService {
    async fn read_http_tts_configs(&self) -> Result<Value, String> {
        self.store
            .read_json_ref(&self.store.http_tts_configs_ref())
            .await
            .map_err(|error| error.to_string())
    }

    async fn write_http_tts_configs(&self, document: &Value) -> Result<(), String> {
        crate::http_tts_config::list_configs(document)?;
        self.store
            .write_json_ref(&self.store.http_tts_configs_ref(), document)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn fetch_http_tts_config_json(&self, input: &str) -> Result<String, String> {
        let config_json = download_json_url(
            input,
            crate::http_tts_config::MAX_CONFIG_BYTES,
            Duration::from_secs(30),
            "HTTP TTS JSON",
        )
        .await?;
        Ok(config_json)
    }

    pub async fn mutate_http_tts_configs(
        &self,
        mutation: crate::http_tts_config::HttpTtsConfigMutation,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let _lock = self.sources_lock.lock().await;
        let document = self.read_http_tts_configs().await?;
        let (updated, mut result) = crate::http_tts_config::apply_mutation(&document, mutation)?;
        if result.get("deleted").and_then(Value::as_bool) != Some(false) {
            self.write_http_tts_configs(&updated).await?;
        }
        result["resource"] = self.resource_descriptor(&self.store.http_tts_configs_ref());
        Ok(result)
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
