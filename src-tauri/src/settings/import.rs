//! 配置导入入口只选择所属业务模块，源协议由 sources 处理。
use crate::{file_access::pick_file_with_limit, support::imports};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConfigurationImportSource {
    Local,
    Url { url: String },
    Text { text: String },
}
const MAX_RULE_BYTES: usize = 2 * 1024 * 1024;

fn detect(value: &Value) -> Result<&'static str, String> {
    if crate::sources::import::is_document(value) {
        return Ok("sources");
    }
    let first = value.as_array().and_then(|v| v.first()).unwrap_or(value);
    let rule = first
        .get("rules")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .unwrap_or(first);
    if rule.get("pattern").is_some() {
        Ok("replacementRules")
    } else if rule.get("name").is_some() && rule.get("rule").is_some() {
        Ok("txtTocRules")
    } else if first.get("configs").is_some()
        || first.get("name").is_some() && first.get("url").is_some()
    {
        Ok("httpTtsConfigs")
    } else {
        Err("无法识别导入内容的类型".into())
    }
}
fn limit(kind: &str) -> Result<usize, String> {
    Ok(match kind {
        "sources" | "auto" => crate::sources::import::MAX_BYTES,
        "replacementRules" => MAX_RULE_BYTES,
        "txtTocRules" => crate::local_books::txt_toc_rules::MAX_IMPORT_BODY_BYTES,
        "httpTtsConfigs" => crate::http_tts_config::MAX_CONFIG_BYTES,
        _ => return Err("不支持的导入类型".into()),
    })
}
pub(crate) async fn prepare(kind: &str, input: &str) -> Result<Value, String> {
    let limit = limit(kind)?;
    let mut text = if kind == "sources" {
        crate::sources::import::resolve(input).await?
    } else {
        imports::text(input, limit).await?
    };
    let value: Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let kind = if kind == "auto" {
        detect(&value)?
    } else {
        kind
    };
    let document = match kind {
        "sources" => {
            if value.get("sourceUrls").is_some() {
                text = crate::sources::import::resolve(&text).await?;
            }
            crate::sources::import::preview(&text)?
        }
        "replacementRules" => crate::reading_tools::parse_replacement_import(&text)?,
        "txtTocRules" => {
            serde_json::to_value(crate::local_books::txt_toc_rules::parse_import(&text)?)
                .map_err(|e| e.to_string())?
        }
        "httpTtsConfigs" => {
            json!({"schemaVersion":1,"configs":crate::http_tts_config::import_configs(&text)?.into_iter().map(|(_,v)|v).collect::<Vec<_>>()})
        }
        _ => unreachable!(),
    };
    Ok(json!({"kind":kind,"document":document}))
}
#[tauri::command]
pub async fn prepare_configuration_import(
    app: tauri::AppHandle,
    kind: String,
    source: ConfigurationImportSource,
) -> Result<Value, String> {
    let limit = limit(&kind)?;
    let input = match source {
        ConfigurationImportSource::Local => {
            let Some(file) =
                pick_file_with_limit(&app, "配置 JSON", &["json"], limit as u64).await?
            else {
                return Ok(json!({"cancelled":true}));
            };
            tokio::fs::read_to_string(file.path)
                .await
                .map_err(|e| e.to_string())?
        }
        ConfigurationImportSource::Url { url } => url,
        ConfigurationImportSource::Text { text } => text,
    };
    prepare(&kind, &input).await
}
