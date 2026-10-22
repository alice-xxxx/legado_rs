//! 词典查询与清洗后的词条缓存。

use crate::{application::ApplicationService, settings::reader_defaults};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::State;

pub(crate) async fn fetch_wiktionary_page(
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

    let client = crate::http_client::builder()
        .timeout(Duration::from_secs(15))
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
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn lookup_dictionary(
    word: String,
    language: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let word = word.trim();
    if word.is_empty()
        || word.chars().count() > MAX_DICTIONARY_WORD_CHARS
        || word.chars().any(char::is_control)
    {
        return Err("请输入不超过 128 个字符的词语。".to_owned());
    }

    let (wiki_root, provider) = match language.as_str() {
        "zh" => ("https://zh.wiktionary.org/", "中文维基词典"),
        "en" => ("https://en.wiktionary.org/", "英文维基词典"),
        _ => return Err("请选择中文或英文维基词典。".to_owned()),
    };
    let (title, html, source_url) = fetch_wiktionary_page(wiki_root, word).await?;
    let settings = service
        .store
        .read_json_ref(&service.store.settings_ref())
        .await
        .ok();
    let defaults = reader_defaults(settings);
    let dictionary_id = format!("{:x}", Sha256::digest(format!("{language}\0{word}")));
    let resource = service
        .store
        .write_dictionary_html(&dictionary_id, &html, &source_url, &defaults)
        .await
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "title": title,
        "provider": provider,
        "sourceUrl": source_url,
        "resource": service.server.descriptor(&resource),
    }))
}

pub(crate) const MAX_DICTIONARY_WORD_CHARS: usize = 128;
pub(crate) const MAX_DICTIONARY_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
