//! 配置导入预览与 JSON 获取；提交通过 fe_db，设置单独保存。

use crate::file_access::pick_file_with_limit;
use crate::source_catalog::MAX_SOURCE_IMPORT_BODY_BYTES;
use crate::support::text_at;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

pub(crate) fn parse_import(kind: &str, source_json: &str) -> Result<Value, String> {
    let source_json = source_json.strip_prefix('\u{feff}').unwrap_or(source_json);
    let detected_kind;
    let kind = if kind == "auto" {
        let value: Value = serde_json::from_str(source_json).map_err(|error| error.to_string())?;
        let entry = value
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(&value);
        detected_kind = if entry.get("bookSourceUrl").is_some()
            || entry.get("sourceUrl").is_some()
            || entry.get("sources").is_some()
            || entry.get("bookSource").is_some()
        {
            "sources"
        } else if entry.get("pattern").is_some() {
            "replacementRules"
        } else if entry.get("themeName").is_some() {
            return Err("本应用尚未提供原版主题导入".into());
        } else if entry.get("showRule").is_some() {
            return Err("本应用尚未提供原版词典规则导入".into());
        } else if entry.get("name").is_some() && entry.get("rule").is_some() {
            "txtTocRules"
        } else if entry.get("name").is_some() && entry.get("url").is_some() {
            "httpTtsConfigs"
        } else if entry.get("configs").is_some() {
            "httpTtsConfigs"
        } else if entry
            .get("rules")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .is_some_and(|rule| rule.get("pattern").is_some())
        {
            "replacementRules"
        } else if entry
            .get("rules")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .is_some_and(|rule| rule.get("rule").is_some())
        {
            "txtTocRules"
        } else {
            return Err("无法识别导入内容的类型".into());
        };
        detected_kind
    } else {
        kind
    };
    let document = match kind {
        "sources" => {
            let value: Value =
                serde_json::from_str(source_json).map_err(|error| error.to_string())?;
            let sources = extract_sources(value)?;
            for source in &sources {
                if !source.is_object()
                    || text_at(source, &["bookSourceUrl", "sourceUrl", "url"]).is_none()
                {
                    return Err("书源缺少来源地址".into());
                }
            }
            // 源定义按文本交给 Vue 选择和提交，避免 JS 舍入其中的 64 位整数。
            json!({ "sources": sources.into_iter().map(|source| source.to_string()).collect::<Vec<_>>() })
        }
        "replacementRules" => crate::reading_tools::parse_replacement_import(source_json)?,
        "txtTocRules" => serde_json::to_value(crate::local_books::txt_toc_rules::parse_import(
            source_json,
        )?)
        .map_err(|error| error.to_string())?,
        "httpTtsConfigs" => {
            let configs = crate::http_tts_config::import_configs(source_json)?
                .into_iter()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>();
            json!({ "schemaVersion": 1, "configs": configs })
        }
        _ => return Err("Unknown configuration kind".into()),
    };
    Ok(json!({ "kind": kind, "document": document }))
}

pub(crate) fn extract_sources(value: Value) -> Result<Vec<Value>, String> {
    if let Some(array) = value.as_array() {
        return Ok(array.clone());
    }
    let Some(object) = value.as_object() else {
        return Err("Source JSON must be an object or array".into());
    };
    for key in ["bookSource", "sources", "data"] {
        if let Some(array) = object.get(key).and_then(Value::as_array) {
            return Ok(array.clone());
        }
    }
    Ok(vec![value])
}

pub(crate) fn parse_http_json_import_url(
    input: &str,
    purpose: &str,
) -> Result<reqwest::Url, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(format!("请输入{purpose}的 HTTP 或 HTTPS 链接"));
    }
    if input.len() > 4096 {
        return Err("链接超过 4096 字节限制".to_owned());
    }
    let url = reqwest::Url::parse(input).map_err(|_| "链接格式无效".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("链接必须是未包含用户名和密码的 HTTP 或 HTTPS URL".to_owned());
    }
    Ok(url)
}

pub(crate) async fn download_json_url(
    input: &str,
    max_bytes: usize,
    timeout: Duration,
    purpose: &str,
) -> Result<String, String> {
    let input = input.trim();
    let without_user_agent = input.ends_with("#requestWithoutUA");
    let input = input.strip_suffix("#requestWithoutUA").unwrap_or(input);
    let url = parse_http_json_import_url(input, purpose)?;
    let requested_https = url.scheme() == "https";
    let client = crate::http_client::builder_with_user_agent(!without_user_agent)
        .timeout(timeout)
        .build()
        .map_err(|_| format!("无法准备{purpose}下载"))?;
    let mut response = client.get(url).send().await.map_err(|error| {
        if error.is_timeout() {
            format!("下载{purpose}超时")
        } else {
            format!("无法下载{purpose}，请检查链接和网络")
        }
    })?;
    let final_url = parse_http_json_import_url(response.url().as_str(), purpose)?;
    if requested_https && final_url.scheme() != "https" {
        return Err(format!("HTTPS {purpose}链接不能重定向到不安全的 HTTP 地址"));
    }
    if !response.status().is_success() {
        return Err(format!("下载{purpose}失败（HTTP {}）", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(format!(
            "{purpose}超过 {} MiB 大小限制",
            max_bytes / (1024 * 1024)
        ));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        if error.is_timeout() {
            format!("下载{purpose}超时")
        } else {
            format!("读取下载的{purpose}失败")
        }
    })? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!(
                "{purpose}超过 {} MiB 大小限制",
                max_bytes / (1024 * 1024)
            ));
        }
        body.extend_from_slice(&chunk);
    }
    if body.is_empty() {
        return Err(format!("{purpose}链接返回了空文件"));
    }
    let text = String::from_utf8(body).map_err(|_| format!("{purpose}必须是 UTF-8 文本"))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}

/// Resolve the original URL / JSON / sourceUrls input contract before preparing an import.
pub(crate) async fn resolve_import_json(
    input: &str,
    max_bytes: usize,
    purpose: &str,
    expand_source_urls: bool,
) -> Result<String, String> {
    let mut pending = vec![input.to_owned()];
    let mut values = Vec::new();
    let mut downloads = HashSet::new();
    let mut bytes = 0usize;
    while let Some(input) = pending.pop() {
        let input = input.trim().trim_start_matches('\u{feff}');
        if input
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
            || input
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
        {
            if downloads.contains(input) {
                continue;
            }
            if downloads.len() >= 32 {
                return Err("导入链接数量过多".into());
            }
            downloads.insert(input.to_owned());
            pending
                .push(download_json_url(input, max_bytes, Duration::from_secs(30), purpose).await?);
            continue;
        }
        bytes = bytes.saturating_add(input.len());
        if bytes > max_bytes {
            return Err(format!("{purpose}超过大小限制"));
        }
        let value: Value =
            serde_json::from_str(input).map_err(|error| format!("{purpose}格式不正确：{error}"))?;
        if expand_source_urls && value.get("sourceUrls").is_some() {
            let urls = value["sourceUrls"]
                .as_array()
                .ok_or_else(|| "sourceUrls 必须是链接数组".to_owned())?;
            if urls.len() > 32 {
                return Err("导入链接数量过多".into());
            }
            for url in urls.iter().rev() {
                let url = url
                    .as_str()
                    .ok_or_else(|| "sourceUrls 必须是链接数组".to_owned())?;
                parse_http_json_import_url(url, purpose)?;
                pending.push(url.to_owned());
            }
        } else if let Value::Array(items) = value {
            values.extend(items);
        } else {
            values.push(value);
        }
    }
    if values.is_empty() {
        return Err("没有可导入的内容".into());
    }
    let value = if values.len() == 1 {
        values.remove(0)
    } else {
        Value::Array(values)
    };
    serde_json::to_string(&value).map_err(|error| error.to_string())
}
#[tauri::command]
pub async fn prepare_configuration_import(
    app: tauri::AppHandle,
    kind: String,
    source: ConfigurationImportSource,
) -> Result<Value, String> {
    let (limit, label) = match kind.as_str() {
        "sources" | "auto" => (MAX_SOURCE_IMPORT_BODY_BYTES, "导入 JSON"),
        "replacementRules" => (MAX_REPLACEMENT_RULE_IMPORT_BODY_BYTES, "显示替换规则 JSON"),
        "txtTocRules" => (
            crate::local_books::txt_toc_rules::MAX_IMPORT_BODY_BYTES,
            "TXT 目录规则 JSON",
        ),
        "httpTtsConfigs" => (crate::http_tts_config::MAX_CONFIG_BYTES, "HTTP TTS JSON"),
        _ => return Err("Unknown configuration kind".into()),
    };
    let source_json = match source {
        ConfigurationImportSource::Local => {
            let Some(file) = pick_file_with_limit(&app, label, &["json"], limit as u64).await?
            else {
                return Ok(json!({ "cancelled": true }));
            };
            tokio::fs::read_to_string(&file.path)
                .await
                .map_err(|error| error.to_string())?
        }
        ConfigurationImportSource::Url { url } => {
            resolve_import_json(
                &url,
                limit,
                label,
                matches!(kind.as_str(), "sources" | "auto"),
            )
            .await?
        }
        ConfigurationImportSource::Text { text } => {
            resolve_import_json(
                &text,
                limit,
                label,
                matches!(kind.as_str(), "sources" | "auto"),
            )
            .await?
        }
    };
    parse_import(&kind, &source_json)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConfigurationImportSource {
    Local,
    Url { url: String },
    Text { text: String },
}

pub(crate) const MAX_REPLACEMENT_RULE_IMPORT_BODY_BYTES: usize = 2 * 1024 * 1024;
