//! 展示替换规则文档操作。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

use crate::{
    models::CURRENT_SCHEMA_VERSION,
    resources::{ResourceRef, ResourceStore},
};

use super::common::{current_schema_version, validate_id};

const REPLACEMENTS_PATH: &str = "resource://replacement-rules.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementRuleDocument {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub rules: Vec<DisplayReplacementRule>,
}

impl Default for ReplacementRuleDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            rules: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisplayReplacementRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub replacement: String,
    pub enabled: bool,
    pub is_regex: bool,
    /// `all` or `book:<book-id>`; interpretation and replacement happen in JS.
    pub scope: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayReplacementRuleInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub pattern: String,
    #[serde(default)]
    pub replacement: String,
    #[serde(default = "default_replacement_enabled")]
    pub enabled: bool,
    pub is_regex: bool,
    #[serde(default = "default_replacement_scope")]
    pub scope: String,
}

pub async fn replacement_rules_resource(store: &ResourceStore) -> Result<ResourceRef, String> {
    let reference = replacements_ref();
    let value=store.read_json_ref(&reference).await.map_err(|e|e.to_string())?;
    validate_replacement_document(&value)?;
    Ok(reference)
}

/// Parse import data without changing the saved configuration.
pub fn parse_replacement_import(source_json: &str) -> Result<Value, String> {
    let rules = parse_replacement_rules_json(source_json)?.into_iter().map(|input| {
        json!({
            "id": format!("replacement-{}", uuid::Uuid::new_v4().simple()),
            "name": input.name, "pattern": input.pattern, "replacement": input.replacement,
            "enabled": input.enabled, "isRegex": input.is_regex, "scope": input.scope,
        })
    }).collect::<Vec<_>>();
    Ok(json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "rules": rules }))
}

/// Parse and validate a complete display-rule document before importing it.
fn parse_replacement_rules_json(source_json: &str) -> Result<Vec<DisplayReplacementRuleInput>, String> {
    if source_json.len() > 2 * 1024 * 1024 {
        return Err("显示替换规则 JSON 超过 2 MiB 限制".to_owned());
    }
    let value: Value = serde_json::from_str(source_json)
        .map_err(|error| format!("显示替换规则 JSON 格式无效：{error}"))?;
    let rules = match &value {
        Value::Array(rules) => rules,
        Value::Object(object) => {
            if object
                .get("schemaVersion")
                .is_some_and(|version| version.as_u64() != Some(u64::from(CURRENT_SCHEMA_VERSION)))
            {
                return Err("显示替换规则 JSON 版本不受支持".to_owned());
            }
            object
                .get("rules")
                .and_then(Value::as_array)
                .ok_or_else(|| "显示替换规则 JSON 缺少 rules 数组".to_owned())?
        }
        _ => return Err("显示替换规则 JSON 必须是规则数组或包含 rules 数组的对象".to_owned()),
    };
    if rules.is_empty() {
        return Err("链接中的 JSON 没有显示替换规则".to_owned());
    }
    if rules.len() > 1000 {
        return Err("显示替换规则导入每次最多支持 1000 条".to_owned());
    }

    let mut imported = Vec::with_capacity(rules.len());
    for (index, rule) in rules.iter().enumerate() {
        let mut normalized_rule = rule.clone();
        if let Some(object) = normalized_rule.as_object_mut() {
            // 导入时会生成本地 ID，不接受上游 ID；旧规则把文本数字字段写成了 JSON 数字时按原值文本化。
            object.remove("id");
            for field in ["name", "pattern", "replacement", "scope"] {
                if let Some(number) = object.get(field).and_then(Value::as_number) {
                    object.insert(field.to_owned(), Value::String(number.to_string()));
                }
            }
        }
        let mut input: DisplayReplacementRuleInput = serde_json::from_value(normalized_rule)
            .map_err(|error| format!("第 {} 条显示替换规则格式无效：{error}", index + 1))?;
        input.id = None;
        let (name, scope) = validate_replacement_rule_input(&input)
            .map_err(|error| format!("第 {} 条显示替换规则无效：{error}", index + 1))?;
        input.name = name;
        input.scope = scope;
        imported.push(input);
    }
    Ok(imported)
}

fn validate_replacement_rule_input(
    input: &DisplayReplacementRuleInput,
) -> Result<(String, String), String> {
    let name = input.name.trim().to_owned();
    if name.is_empty() || name.len() > 512 {
        return Err("Replacement rule name must contain 1 to 512 bytes".to_owned());
    }
    if input.enabled && input.pattern.is_empty() {
        return Err("Enabled replacement pattern cannot be empty".to_owned());
    }
    if input.pattern.len() > 16_384 {
        return Err("Replacement pattern exceeds 16384 bytes".to_owned());
    }
    if input.replacement.len() > 16_384 {
        return Err("Replacement value exceeds 16 KiB".to_owned());
    }
    let scope = input.scope.trim().to_owned();
    if scope != "all" {
        let book_id = scope
            .strip_prefix("book:")
            .ok_or_else(|| "Replacement scope must be 'all' or 'book:<bookId>'".to_owned())?;
        validate_id(book_id, "bookId")?;
    }
    Ok((name, scope))
}

pub fn validate_replacement_document(value: &Value) -> Result<(), String> {
    let document: ReplacementRuleDocument = serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid replacement rules JSON: {error}"))?;
    if document.schema_version != CURRENT_SCHEMA_VERSION {
        return Err("Unsupported replacement rules schema".into());
    }
    let mut ids = HashSet::new();
    for rule in document.rules {
        validate_id(&rule.id, "replacementRuleId")?;
        if !ids.insert(rule.id.clone()) { return Err("Duplicate replacement rule ID".into()); }
        validate_replacement_rule_input(&DisplayReplacementRuleInput {
            id: Some(rule.id), name: rule.name, pattern: rule.pattern,
            replacement: rule.replacement, enabled: rule.enabled,
            is_regex: rule.is_regex, scope: rule.scope,
        })?;
    }
    Ok(())
}

fn replacements_ref() -> ResourceRef {
    ResourceRef::new(REPLACEMENTS_PATH).expect("static replacement resource reference is valid")
}

fn default_replacement_scope() -> String {
    "all".to_owned()
}

fn default_replacement_enabled() -> bool {
    true
}
