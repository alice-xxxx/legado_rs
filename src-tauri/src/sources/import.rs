//! 源协议解析与订阅链接展开，不访问数据库连接或表。
use crate::{
    database::SourceDefinition,
    support::{imports, text_at},
};
use serde_json::{json, Value};
use std::collections::HashSet;
pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;

pub(crate) fn is_document(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.first().is_some_and(is_document),
        Value::Object(map) => {
            [
                "bookSourceUrl",
                "sourceUrl",
                "feedUrl",
                "sourceUrls",
                "sources",
                "bookSource",
            ]
            .iter()
            .any(|key| map.contains_key(*key))
                || map.get("data").is_some_and(is_document)
        }
        _ => false,
    }
}
/// 同时接受单源、源数组和常用导出容器。
fn collect(value: Value, output: &mut Vec<Value>) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            for value in values {
                collect(value, output)?
            }
        }
        Value::String(text) => collect(
            serde_json::from_str(&text).map_err(|e| format!("源 JSON 格式错误：{e}"))?,
            output,
        )?,
        Value::Object(mut map) => {
            if !map.contains_key("bookSourceUrl")
                && !map.contains_key("sourceUrl")
                && !map.contains_key("feedUrl")
            {
                if let Some(key) = ["bookSource", "sources", "data"]
                    .into_iter()
                    .find(|key| map.contains_key(*key))
                {
                    return collect(map.remove(key).unwrap(), output);
                }
            }
            output.push(normalize(Value::Object(map))?);
        }
        _ => return Err("源内容必须是对象或数组".into()),
    }
    Ok(())
}
pub(crate) fn preview(text: &str) -> Result<Value, String> {
    if text.len() > MAX_BYTES {
        return Err("源导入内容超过 32 MiB".into());
    }
    let value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("源 JSON 格式错误：{e}"))?;
    let mut records = Vec::new();
    collect(value, &mut records)?;
    if records.is_empty() {
        return Err("没有可导入的源".into());
    }
    for record in &records {
        serde_json::from_value::<SourceDefinition>(record.clone())
            .map_err(|e| format!("源定义字段无效：{e}"))?;
    }
    Ok(json!({"sources":records.into_iter().map(|v|v.to_string()).collect::<Vec<_>>()}))
}
pub(crate) async fn resolve(input: &str) -> Result<String, String> {
    let mut pending = vec![input.to_owned()];
    let mut documents = Vec::new();
    let mut visited = HashSet::new();
    let mut bytes = 0usize;
    while let Some(input) = pending.pop() {
        let input = input.trim().trim_start_matches('\u{feff}');
        let text = if imports::is_http(input) {
            let canonical =
                imports::url(input.strip_suffix("#requestWithoutUA").unwrap_or(input))?.to_string();
            if !visited.insert(canonical) {
                continue;
            }
            if visited.len() > 32 {
                return Err("源导入链接不能超过 32 个".into());
            }
            imports::text(input, MAX_BYTES).await?
        } else {
            input.to_owned()
        };
        bytes = bytes.saturating_add(text.len());
        if bytes > MAX_BYTES {
            return Err("源导入内容超过 32 MiB".into());
        }
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("源 JSON 格式错误：{e}"))?;
        if let Some(urls) = value.get("sourceUrls") {
            let urls = urls
                .as_array()
                .filter(|v| v.len() <= 32)
                .ok_or("sourceUrls 必须是不超过 32 项的链接数组")?;
            for url in urls.iter().rev() {
                let url = url.as_str().ok_or("sourceUrls 中存在非链接内容")?;
                imports::url(url)?;
                pending.push(url.to_owned());
            }
        } else {
            documents.push(value)
        }
    }
    if documents.is_empty() {
        return Err("没有可导入的源".into());
    }
    serde_json::to_string(&if documents.len() == 1 {
        documents.remove(0)
    } else {
        Value::Array(documents)
    })
    .map_err(|e| e.to_string())
}
/// 旧 RSS 源转换为引擎的标准源定义，规则和注入脚本保留。
pub(crate) fn normalize(mut value: Value) -> Result<Value, String> {
    use tauri_plugin_source_engine::source::{is_legacy_rss_source, is_standard_feed_source};
    if !value.is_object() {
        return Err("源定义必须是 JSON 对象".into());
    }
    if is_legacy_rss_source(&value) {
        let original = value.clone();
        let map = value.as_object_mut().unwrap();
        for (old, new) in [
            ("sourceUrl", "bookSourceUrl"),
            ("sourceName", "bookSourceName"),
            ("sourceGroup", "bookSourceGroup"),
            ("sourceComment", "bookSourceComment"),
            ("sortUrl", "exploreUrl"),
            ("articleStyle", "exploreStyle"),
        ] {
            if let Some(value) = original.get(old) {
                map.insert(new.into(), value.clone());
            }
        }
        map.insert("bookSourceType".into(), json!(5));
        map.insert("enabledExplore".into(), json!(true));
        let explore = [
            ("ruleArticles", "bookList"),
            ("ruleTitle", "name"),
            ("rulePubDate", "author"),
            ("ruleDescription", "intro"),
            ("ruleImage", "coverUrl"),
            ("ruleLink", "bookUrl"),
        ]
        .into_iter()
        .filter_map(|(old, new)| {
            original
                .get(old)
                .filter(|v| !v.is_null())
                .map(|v| (new.to_owned(), v.clone()))
        })
        .collect::<serde_json::Map<_, _>>();
        map.insert("ruleExplore".into(), Value::Object(explore));
        let mut content = json!({});
        if let Some(rule) = original.get("ruleContent").and_then(Value::as_str) {
            content["content"] = json!(rule);
        }
        let style = original["style"].as_str().unwrap_or("");
        let mut script = if style.is_empty() {
            String::new()
        } else {
            format!("var style=document.createElement('style');style.innerHTML={};document.head.appendChild(style);",json!(style))
        };
        script.push_str(original["injectJs"].as_str().unwrap_or(""));
        if !script.is_empty() {
            content["webJs"] = json!(script);
        }
        if let Some(rule) = original.get("shouldOverrideUrlLoading") {
            content["shouldOverrideUrlLoading"] = rule.clone();
        }
        map.insert("ruleContent".into(), content);
    } else if value.get("bookSourceUrl").is_none() && is_standard_feed_source(&value) {
        let url = text_at(&value, &["feedUrl", "sourceUrl", "url"]).ok_or("订阅源缺少地址")?;
        let name = text_at(&value, &["bookSourceName", "sourceName", "name", "title"])
            .unwrap_or_else(|| url.clone());
        value["bookSourceUrl"] = json!(url);
        value["bookSourceName"] = json!(name);
        value["bookSourceType"] = json!(5);
    }
    let url = text_at(&value, &["bookSourceUrl"]).ok_or("源定义缺少 bookSourceUrl")?;
    if url.len() > 8192 || url.chars().any(char::is_control) {
        return Err("源地址无效".into());
    }
    let name = text_at(&value, &["bookSourceName"]).unwrap_or_else(|| url.clone());
    value["bookSourceUrl"] = json!(url);
    value["bookSourceName"] = json!(name);
    Ok(value)
}
