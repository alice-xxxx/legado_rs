//! 配置文本获取，只负责 URL、大小和编码，不识别任何业务配置。
use std::time::Duration;

pub(crate) fn is_http(input: &str) -> bool {
    input
        .get(..7)
        .is_some_and(|v| v.eq_ignore_ascii_case("http://"))
        || input
            .get(..8)
            .is_some_and(|v| v.eq_ignore_ascii_case("https://"))
}
pub(crate) fn url(input: &str) -> Result<reqwest::Url, String> {
    if input.len() > 4096 {
        return Err("导入链接过长".into());
    }
    let url = reqwest::Url::parse(input).map_err(|_| "导入链接格式错误")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("导入链接必须是无账号密码的 HTTP 或 HTTPS 地址".into());
    }
    Ok(url)
}
pub(crate) async fn text(input: &str, limit: usize) -> Result<String, String> {
    let input = input.trim().trim_start_matches('\u{feff}');
    if !is_http(input) {
        if input.len() > limit {
            return Err("导入内容超过大小限制".into());
        }
        return Ok(input.to_owned());
    }
    let no_ua = input.ends_with("#requestWithoutUA");
    let target = url(input.strip_suffix("#requestWithoutUA").unwrap_or(input))?;
    let https = target.scheme() == "https";
    let client = crate::http_client::builder_with_user_agent(!no_ua)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(request_error)?;
    let mut response = client.get(target).send().await.map_err(request_error)?;
    let final_url = url(response.url().as_str())?;
    if https && final_url.scheme() != "https" {
        return Err("HTTPS 导入链接不能降级为 HTTP".into());
    }
    if !response.status().is_success() {
        return Err(format!("获取导入内容失败：HTTP {}", response.status()));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("导入内容超过大小限制".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("导入内容超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("导入链接没有返回内容".into());
    }
    String::from_utf8(bytes)
        .map(|v| v.trim_start_matches('\u{feff}').to_owned())
        .map_err(|_| "导入内容必须使用 UTF-8 编码".into())
}

fn request_error(error: reqwest::Error) -> String {
    let mut cause: Option<&dyn std::error::Error> = Some(&error);
    let mut messages = Vec::new();
    while let Some(error) = cause {
        messages.push(error.to_string());
        cause = error.source();
    }
    format!("获取导入内容失败：{}", messages.join("："))
}

#[cfg(test)]
mod tests {
    #[test]
    fn request_error_includes_the_underlying_cause() {
        let error = crate::http_client::raw_builder()
            .user_agent("\n")
            .build()
            .unwrap_err();
        let message = super::request_error(error);
        assert!(message.contains("builder error"));
        assert!(message.contains("failed to parse header value"));
    }
}
