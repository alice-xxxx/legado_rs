//! 书源登录、元数据和 JSON 导入共用处理。

use super::*;

pub(super) fn engine_request(
    operation: &str,
    source: &Value,
    keyword: Option<String>,
    page: Option<i32>,
    book: Option<Value>,
    chapter: Option<Value>,
    next_chapter_url: Option<String>,
) -> SourceEngineRequest {
    SourceEngineRequest {
        operation: operation.to_owned(),
        source: source.clone(),
        keyword,
        credentials: None,
        action_id: None,
        media_url: None,
        media_headers: None,
        page,
        book,
        chapter,
        next_chapter_url,
    }
}

pub(super) fn sanitize_source_login_form(form: &Value) -> Result<Value, String> {
    let can_login = form
        .get("canLogin")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Source engine returned an invalid login form".to_owned())?;
    let raw_fields = form["fields"]
        .as_array()
        .ok_or_else(|| "Source engine returned an invalid login form".to_owned())?;
    let raw_actions = form["actions"]
        .as_array()
        .ok_or_else(|| "Source engine returned an invalid login form".to_owned())?;
    if raw_fields.len() > 64 || raw_actions.len() > 64 {
        return Err("Source login form exceeds the supported field limit".to_owned());
    }

    let mut names = HashSet::new();
    let mut fields = Vec::with_capacity(raw_fields.len());
    for field in raw_fields {
        let name = field
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty() && name.len() <= 128)
            .ok_or_else(|| "Source engine returned an invalid login field".to_owned())?;
        if !names.insert(name.to_owned()) {
            return Err("Source login form has duplicate field names".to_owned());
        }
        let field_type = field
            .get("type")
            .and_then(Value::as_str)
            .filter(|field_type| matches!(*field_type, "text" | "password" | "select" | "toggle"))
            .ok_or_else(|| "Source engine returned an unsupported login field".to_owned())?;
        let mut safe_field = json!({
            "name": name,
            "label": name,
            "type": field_type,
            "password": field_type == "password",
        });
        if field_type == "select" {
            let choices = field
                .get("choices")
                .and_then(Value::as_array)
                .ok_or_else(|| "Source engine returned invalid login choices".to_owned())?;
            if choices.len() > 64 {
                return Err("Source login form has too many choices".to_owned());
            }
            let choices = choices
                .iter()
                .map(|choice| {
                    choice
                        .as_str()
                        .filter(|choice| choice.len() <= 256)
                        .map(str::to_owned)
                        .ok_or_else(|| "Source engine returned invalid login choices".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            safe_field["choices"] = json!(choices);
        }
        fields.push(safe_field);
    }

    let mut action_ids = HashSet::new();
    let mut actions = Vec::with_capacity(raw_actions.len());
    for action in raw_actions {
        let id = action
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id <= i32::MAX as u64)
            .ok_or_else(|| "Source engine returned an invalid login action".to_owned())?;
        if !action_ids.insert(id) {
            return Err("Source engine returned duplicate login actions".to_owned());
        }
        let label = action
            .get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty() && label.len() <= 128)
            .ok_or_else(|| "Source engine returned an invalid login action".to_owned())?;
        actions.push(json!({ "id": id, "label": label }));
    }

    Ok(json!({
        "mode": "form",
        "canLogin": can_login,
        "fields": fields,
        "actions": actions,
    }))
}

pub(super) fn validate_source_login_credentials(
    credentials: &HashMap<String, String>,
) -> Result<(), String> {
    if credentials.len() > 64 {
        return Err("Source login input has too many fields".to_owned());
    }
    let mut total_bytes = 0usize;
    for (name, value) in credentials {
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err("Source login input contains an invalid field".to_owned());
        }
        if value.len() > 16 * 1024 {
            return Err("A source login field exceeds the supported size".to_owned());
        }
        total_bytes = total_bytes
            .saturating_add(name.len())
            .saturating_add(value.len());
    }
    if total_bytes > 64 * 1024 {
        return Err("Source login input exceeds the supported size".to_owned());
    }
    Ok(())
}

pub(super) fn sanitize_source_login_result(result: Value) -> Result<Value, String> {
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "Source login returned an invalid result".to_owned())?;
    let code = result
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| "Source login returned an invalid result".to_owned())?;
    let message = match (status, code) {
        ("executed", "login_executed") => "登录操作已执行。此书源没有提供独立的认证验证。",
        ("executed", "login_action_executed") => "登录操作已执行。",
        ("failed", "login_unavailable") => "此书源没有可执行的表单登录规则。",
        ("failed", "invalid_credentials") => "登录输入与此书源表单不匹配。",
        ("failed", "storage_unavailable" | "storage_failed") => "登录信息无法保存，请稍后重试。",
        ("failed", "login_failed") => "登录请求执行失败。请检查凭据、网络或书源登录规则后重试。",
        ("failed", "login_action_unavailable") => "此书源登录操作不可用。",
        ("failed", "login_action_failed") => "登录操作执行失败。请检查网络或书源登录规则后重试。",
        _ => return Err("Source login returned an invalid result".to_owned()),
    };
    Ok(json!({
        "status": status,
        "code": code,
        "message": message,
        "authenticated": null,
    }))
}
pub(super) fn extract_sources(value: Value) -> Result<Vec<Value>, String> {
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

pub(super) fn parse_http_json_import_url(
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

pub(super) async fn download_json_url(
    input: &str,
    max_bytes: usize,
    timeout: Duration,
    purpose: &str,
) -> Result<String, String> {
    let url = parse_http_json_import_url(input, purpose)?;
    let requested_https = url.scheme() == "https";
    let client = reqwest::Client::builder()
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

pub(super) fn metadata(records: &[SourceRecord]) -> Vec<SourceMetadata> {
    records
        .iter()
        .map(|record| SourceMetadata {
            id: record.id.clone(),
            name: record.name.clone(),
            group: record.group.clone(),
            enabled: record.enabled,
            is_rss: crate::source_metadata::is_rss_source_metadata(&record.source),
            media_type: crate::source_metadata::media_type(&record.source).map(str::to_owned),
            capabilities: source_capabilities(&record.source),
            user_agent_override: valid_user_agent_override(&record.source),
        })
        .collect()
}

/// Expose only rule presence to the management UI, never private scripts.
pub(super) fn source_capabilities(source: &Value) -> SourceCapabilities {
    fn configured(source: &Value, keys: &[&str]) -> bool {
        keys.iter().any(|key| match source.get(*key) {
            Some(Value::String(text)) => !text.trim().is_empty(),
            Some(Value::Object(values)) => !values.is_empty(),
            Some(Value::Array(values)) => !values.is_empty(),
            None | Some(Value::Null) => false,
            Some(_) => true,
        })
    }
    SourceCapabilities {
        search: configured(source, &["searchUrl", "ruleSearch"]),
        detail: configured(source, &["ruleBookInfo"]),
        toc: configured(source, &["ruleToc"]),
        content: configured(source, &["ruleContent"]),
    }
}

pub(super) fn valid_user_agent_override(source: &Value) -> Option<String> {
    let value = source.get("legadoRsUserAgentOverride")?.as_str()?.trim();
    (!value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}

pub(super) fn can_change_source_from_private(private: &Value) -> bool {
    private
        .get("sourceId")
        .and_then(Value::as_str)
        .is_some_and(|source_id| !source_id.trim().is_empty())
        && private.get("book").is_some_and(Value::is_object)
        && private.get("chapters").and_then(Value::as_array).is_some()
}

pub(super) fn set_optional_public_field(document: &mut Value, key: &str, value: Option<Value>) {
    if let Some(fields) = document.as_object_mut() {
        if let Some(value) = value {
            fields.insert(key.to_owned(), value);
        } else {
            fields.remove(key);
        }
    }
}
