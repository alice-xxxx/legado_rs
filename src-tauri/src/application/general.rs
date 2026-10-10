//! 应用启动状态、首页配置、RSS、阅读统计和轻量设置服务。

use super::*;

impl ApplicationService {
    /// Return the browser-readable, processed home configuration resource.
    pub async fn get_home_config(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::discovery::home_config::home_config_resource(&self.store).await?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn get_reading_statistics(&self, from_ms: u64, to_ms: u64) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        crate::reading_tools::reading_statistics(&self.store, from_ms, to_ms).await
    }

    /// Validate and save the home layout. Source/category IDs are resolved by
    /// Rust; the public document contains no source URLs or source rules.
    pub async fn save_home_config(
        &self,
        config: crate::discovery::home_config::HomeConfigDocument,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let input = serde_json::to_value(config)
            .map_err(|error| format!("Cannot encode home configuration: {error}"))?;
        let resource = crate::discovery::home_config::save_home_config(self, input).await?;
        Ok(self.resource_descriptor(&resource))
    }

    /// Return the RSS read/favorite/filter state resource.
    pub async fn get_rss_state(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::rss::rss_state_resource(&self.store).await?;
        Ok(self.resource_descriptor(&resource))
    }

    /// Look up one word in the selected Wiktionary and publish only sanitized
    /// HTML plus its reader metadata to the WebView.
    pub async fn lookup_dictionary(&self, word: &str, language: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let word = word.trim();
        if word.is_empty()
            || word.chars().count() > MAX_DICTIONARY_WORD_CHARS
            || word.chars().any(char::is_control)
        {
            return Err("请输入不超过 128 个字符的词语。".to_owned());
        }

        let (wiki_root, provider) = match language {
            "zh" => ("https://zh.wiktionary.org/", "中文维基词典"),
            "en" => ("https://en.wiktionary.org/", "英文维基词典"),
            _ => return Err("请选择中文或英文维基词典。".to_owned()),
        };
        let (title, html, source_url) = fetch_wiktionary_page(wiki_root, word).await?;
        let settings = self
            .store
            .read_json_ref(&self.store.settings_ref())
            .await
            .ok();
        let defaults = reader_defaults(settings);
        let dictionary_id = format!("{:x}", Sha256::digest(format!("{language}\0{word}")));
        let resource = self
            .store
            .write_dictionary_html(&dictionary_id, &html, &source_url, &defaults)
            .await
            .map_err(|error| error.to_string())?;

        Ok(json!({
            "title": title,
            "provider": provider,
            "sourceUrl": source_url,
            "resource": self.resource_descriptor(&resource),
        }))
    }

    pub async fn set_rss_filter(&self, source_id: &str, filter: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::rss::set_subscription_filter(self, source_id, filter).await?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn set_rss_article_state(
        &self,
        source_id: &str,
        article_id: &str,
        is_read: Option<bool>,
        is_favorite: Option<bool>,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource =
            crate::rss::set_article_state(self, source_id, article_id, is_read, is_favorite)
                .await?;
        Ok(self.resource_descriptor(&resource))
    }

    /// Remove an RSS subscription and its private category mapping, cached
    /// article HTML, and read/favorite/filter state under one restore guard.
    pub async fn unsubscribe_rss(&self, source_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let _sources = self.sources_lock.lock().await;
        let source = self.source_record(source_id).await?;
        if !crate::rss::is_rss_source(&source.source) {
            return Err("Selected source is not an RSS subscription".to_owned());
        }
        let resource = crate::rss::remove_subscription_data(self, source_id).await?;
        let mut records = self.read_sources().await?;
        records.retain(|source| source.id != source_id);
        self.write_sources(&records).await?;
        Ok(json!({
            "sources": metadata(&records),
            "resource": self.resource_descriptor(&resource),
        }))
    }

    /// Import a TXT rule JSON document and return the published JSON resource.
    pub async fn import_txt_toc_rules_json(&self, source_json: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let source_json = source_json.strip_prefix('\u{feff}').unwrap_or(source_json);
        let (document, imported_count) =
            crate::local_books::txt_toc_rules::import_rules_json(&self.store, source_json).await?;
        let resource =
            crate::local_books::txt_toc_rules::publish_snapshot(&self.store, &document).await?;
        Ok(json!({
            "resource": self.resource_descriptor(&resource),
            "importedCount": imported_count,
        }))
    }

    /// Download and import remote TXT rules as one user operation.
    pub async fn import_txt_toc_rules_from_url(&self, url: &str) -> Result<Value, String> {
        let source_json = self.fetch_txt_toc_rule_json(url).await?;
        self.import_txt_toc_rules_json(&source_json).await
    }

    /// Apply one validated TXT rule edit and return its committed JSON resource.
    pub async fn mutate_txt_toc_rules(
        &self,
        mutation: crate::local_books::txt_toc_rules::TxtTocRulesEdit,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let (document, saved_id) = match mutation {
            crate::local_books::txt_toc_rules::TxtTocRulesEdit::Save { id, rule } => {
                let (document, saved_id) =
                    crate::local_books::txt_toc_rules::save_rule(&self.store, id, rule).await?;
                (document, Some(saved_id))
            }
            crate::local_books::txt_toc_rules::TxtTocRulesEdit::Reorder { ordered_ids } => {
                (
                    crate::local_books::txt_toc_rules::reorder_rules(&self.store, ordered_ids)
                        .await?,
                    None,
                )
            }
            crate::local_books::txt_toc_rules::TxtTocRulesEdit::Delete { rule_ids } => {
                (
                    crate::local_books::txt_toc_rules::delete_rules(&self.store, rule_ids).await?,
                    None,
                )
            }
        };
        let resource =
            crate::local_books::txt_toc_rules::publish_snapshot(&self.store, &document).await?;
        let mut response = json!({ "resource": self.resource_descriptor(&resource) });
        if let Some(id) = saved_id {
            response["savedId"] = json!(id);
        }
        Ok(response)
    }

    pub async fn bootstrap(&self) -> Result<Value, String> {
        crate::search_history::load(&self.store).await?;
        let settings = self
            .store
            .read_json_ref(&self.store.settings_ref())
            .await
            .map_err(|error| error.to_string())?;
        crate::source_http::set_network_timeout_seconds(configured_source_http_timeout_seconds(
            Some(&settings),
        ));
        let search_history = crate::search_history::resource_ref(&self.store)?;
        let replacement_rules =
            crate::reading_tools::replacement_rules_resource(&self.store).await?;
        let txt_toc_document =
            crate::local_books::txt_toc_rules::read_document(&self.store).await?;
        let txt_toc_rules =
            crate::local_books::txt_toc_rules::publish_snapshot(&self.store, &txt_toc_document)
                .await?;
        let sources = self.source_definitions_resource().await?;
        Ok(json!({
            "shelf": self.resource_descriptor(&self.store.shelf_ref()),
            "settings": self.resource_descriptor(&self.store.settings_ref()),
            "httpTtsConfigs": self.resource_descriptor(&self.store.http_tts_configs_ref()),
            "sources": self.resource_descriptor(&sources),
            "txtTocRules": self.resource_descriptor(&txt_toc_rules),
            "replacementRules": self.resource_descriptor(&replacement_rules),
            "searchHistory": self.resource_descriptor(&search_history),
        }))
    }

    pub async fn get_search_history(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        crate::search_history::load(&self.store).await?;
        let resource = crate::search_history::resource_ref(&self.store)?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn delete_search_history(&self, query: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::search_history::delete_query(&self.store, query).await?;
        Ok(self.resource_descriptor(&resource))
    }

    pub async fn clear_search_history(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let resource = crate::search_history::clear(&self.store).await?;
        Ok(self.resource_descriptor(&resource))
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) async fn fetch_wiktionary_page(
    wiki_root: &str,
    word: &str,
) -> Result<(String, String, String), String> {
    let wiki_url =
        reqwest::Url::parse(wiki_root).map_err(|_| "维基词典地址无效，请稍后重试。".to_owned())?;
    let mut endpoint = wiki_url
        .join("w/api.php")
        .map_err(|_| "无法准备维基词典查询，请稍后重试。".to_owned())?;
    {
        let mut query = endpoint.query_pairs_mut();
        query
            .append_pair("action", "parse")
            .append_pair("page", word)
            .append_pair("prop", "text")
            .append_pair("format", "json")
            .append_pair("formatversion", "2")
            .append_pair("redirects", "1");
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("legado_rs/0.1 (Wiktionary dictionary lookup)")
        .build()
        .map_err(|_| "无法准备维基词典查询，请稍后重试。".to_owned())?;
    let mut response = client
        .get(endpoint)
        .send()
        .await
        .map_err(|_| "无法连接维基词典，请检查网络后重试。".to_owned())?;
    if !response.status().is_success() {
        return Err("维基词典暂时无法查询，请稍后重试。".to_owned());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DICTIONARY_RESPONSE_BYTES as u64)
    {
        return Err("维基词典返回内容超过大小限制，请稍后重试。".to_owned());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取维基词典结果失败，请重试。".to_owned())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_DICTIONARY_RESPONSE_BYTES {
            return Err("维基词典返回内容超过大小限制，请稍后重试。".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "维基词典返回了无法读取的结果，请重试。".to_owned())?;
    if let Some(error) = payload.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(code, "missingtitle" | "nosuchpageid") {
            return Err(format!("维基词典中找不到“{word}”这个词条。"));
        }
        return Err("维基词典暂时无法查询，请稍后重试。".to_owned());
    }
    let Some(parsed) = payload.get("parse") else {
        return Err(format!("维基词典中找不到“{word}”这个词条。"));
    };
    let title = parsed
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| {
            !title.is_empty()
                && title.chars().count() <= 512
                && !title.chars().any(char::is_control)
        })
        .ok_or_else(|| "维基词典没有返回有效词条标题，请重试。".to_owned())?;
    let html = parsed
        .get("text")
        .and_then(Value::as_str)
        .filter(|html| !html.trim().is_empty())
        .ok_or_else(|| format!("维基词典中找不到“{word}”这个词条。"))?;

    let mut source_url = wiki_url;
    let encoded_title = title.replace(' ', "_");
    source_url
        .path_segments_mut()
        .map_err(|_| "无法准备维基词典词条链接，请重试。".to_owned())?
        .push("wiki")
        .push(&encoded_title);
    Ok((title.to_owned(), html.to_owned(), source_url.to_string()))
}

pub(super) fn configured_source_http_timeout_seconds(settings: Option<&Value>) -> u64 {
    settings
        .and_then(|settings| settings.get("sourceHttpTimeoutSeconds"))
        .and_then(Value::as_u64)
        .filter(|seconds| [15, 30, 60, 120].contains(seconds))
        .unwrap_or(15)
}

pub(super) fn reader_defaults(settings: Option<Value>) -> ReaderDefaults {
    let Some(mut value) = settings else {
        return ReaderDefaults::default();
    };
    if value.get("reader").is_some() {
        value = value["reader"].clone();
    }
    let mut defaults = ReaderDefaults::default();
    if let Some(size) = value
        .get("fontSizePx")
        .or_else(|| value.get("fontSize"))
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
    {
        defaults.font_size_px = (size as f32).clamp(12.0, 36.0);
    }
    if let Some(line_height) = value
        .get("lineHeight")
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
    {
        defaults.line_height = (line_height as f32).clamp(1.2, 2.8);
    }
    if let Some(family) = value.get("fontFamily").and_then(Value::as_str) {
        defaults.font_family = match family {
            "serif" => "serif",
            "sans" => "sans-serif",
            "system" => "system-ui, sans-serif",
            "mono" => "monospace",
            _ => defaults.font_family.as_str(),
        }
        .to_owned();
    }
    if let Some(color) = value.get("textColor").and_then(Value::as_str) {
        defaults.text_color = reader_color(color, &defaults.text_color);
    }
    if let Some(color) = value.get("backgroundColor").and_then(Value::as_str) {
        defaults.background_color = reader_color(color, &defaults.background_color);
    }
    if let Some(align) = value.get("textAlign").and_then(Value::as_str) {
        if matches!(align, "left" | "right" | "center" | "justify") {
            defaults.text_align = align.to_owned();
        }
    }
    if let Some(preload_count) = value.get("preloadCount").and_then(Value::as_u64) {
        if (1..=20).contains(&preload_count) {
            defaults.preload_count = preload_count as usize;
        }
    }
    if let Some(theme) = value.get("theme").and_then(Value::as_str) {
        defaults.theme = match theme {
            "sepia" => ReaderTheme::Sepia,
            "dark" => ReaderTheme::Dark,
            "system" | "light" => ReaderTheme::System,
            _ => ReaderTheme::Paper,
        };
    }
    defaults
}

pub(super) fn reader_color(value: &str, fallback: &str) -> String {
    let hex = value.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 6) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return format!("#{hex}");
    }
    match value.to_ascii_lowercase().as_str() {
        "black" | "white" | "red" | "green" | "blue" | "gray" | "grey" | "transparent" => {
            value.to_ascii_lowercase()
        }
        _ => fallback.to_owned(),
    }
}

pub(super) fn text_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    })
}

pub(super) fn bool_at(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

pub(super) fn strict_unsigned_field(value: &Value, field: &str) -> Result<Option<u64>, String> {
    match value.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("Progress {field} must be a non-negative integer")),
    }
}

pub(super) fn validate_number_range(
    object: &Map<String, Value>,
    field: &str,
    min: f64,
    max: f64,
) -> Result<(), String> {
    if let Some(value) = object.get(field) {
        let Some(number) = value.as_f64() else {
            return Err(format!("Settings {field} must be a number"));
        };
        if !number.is_finite() || number < min || number > max {
            return Err(format!("Settings {field} must be between {min} and {max}"));
        }
    }
    Ok(())
}

pub(super) fn validate_integer_range(
    object: &Map<String, Value>,
    field: &str,
    min: u64,
    max: u64,
) -> Result<(), String> {
    if let Some(value) = object.get(field) {
        let Some(number) = value.as_u64() else {
            return Err(format!("Settings {field} must be a non-negative integer"));
        };
        if number < min || number > max {
            return Err(format!("Settings {field} must be between {min} and {max}"));
        }
    }
    Ok(())
}

pub(super) fn looks_like_html(content: &str) -> bool {
    let trimmed = content.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<body")
        || trimmed.starts_with("<div")
        || trimmed.starts_with("<p")
        || trimmed.starts_with("<section")
}

pub(super) fn validate_id(id: &str, field: &str) -> Result<(), String> {
    if id.len() > 128
        || id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("Invalid {field}"));
    }
    Ok(())
}

pub(super) fn stable_hash(text: &str) -> u64 {
    text.as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub(super) fn content_type(resource: &str) -> &'static str {
    match resource_extension(resource).as_str() {
        "json" => "application/json; charset=utf-8",
        "html" | "xhtml" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "bmp" => "image/bmp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

pub(super) fn resource_format(resource: &str) -> &'static str {
    match resource_extension(resource).as_str() {
        "json" => "json",
        "html" | "xhtml" => "html",
        "txt" => "text",
        "css" => "stylesheet",
        "bmp" | "png" | "jpg" | "jpeg" | "svg" | "webp" | "gif" => "image",
        "woff" | "woff2" | "ttf" | "otf" => "font",
        "mp3" | "m4a" | "aac" | "wav" | "ogg" | "opus" | "flac" => "audio",
        "mp4" | "webm" => "video",
        "pdf" => "pdf",
        _ => "binary",
    }
}

fn resource_extension(resource: &str) -> String {
    let resource = resource.split(['?', '#']).next().unwrap_or(resource);
    Path::new(resource)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub(super) fn http_tts_audio_type(
    content_type: &str,
    bytes: &[u8],
) -> Result<(&'static str, &'static str), String> {
    let content_type = content_type.trim().to_ascii_lowercase();
    match content_type.as_str() {
        "audio/mpeg" | "audio/mp3" | "audio/x-mpeg" | "audio/x-mp3" => Ok(("mp3", "audio/mpeg")),
        "audio/mp4" | "audio/x-m4a" => Ok(("m4a", "audio/mp4")),
        "audio/aac" | "audio/aacp" => Ok(("aac", "audio/aac")),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Ok(("wav", "audio/wav")),
        "audio/ogg" => Ok(("ogg", "audio/ogg")),
        "audio/opus" => Ok(("opus", "audio/ogg")),
        "audio/flac" | "audio/x-flac" => Ok(("flac", "audio/flac")),
        "" | "application/octet-stream" => http_tts_audio_type_from_signature(bytes),
        _ => Err(format!(
            "Unsupported HTTP TTS audio content type: {content_type}"
        )),
    }
}

pub(super) fn http_tts_audio_type_from_signature(
    bytes: &[u8],
) -> Result<(&'static str, &'static str), String> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Ok(("wav", "audio/wav"))
    } else if bytes.starts_with(b"OggS") {
        Ok(("ogg", "audio/ogg"))
    } else if bytes.starts_with(b"fLaC") {
        Ok(("flac", "audio/flac"))
    } else if bytes.starts_with(b"ID3") {
        Ok(("mp3", "audio/mpeg"))
    } else if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        Ok(("m4a", "audio/mp4"))
    } else if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xf6 == 0xf0 {
        Ok(("aac", "audio/aac"))
    } else if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0 {
        Ok(("mp3", "audio/mpeg"))
    } else {
        Err("HTTP TTS returned an unsupported audio signature".to_owned())
    }
}
