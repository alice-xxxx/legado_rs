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
    store
        .update_json_ref_or_default(&reference, empty_replacement_document(), |mut value| {
            replacement_document(&mut value)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Import user-authored display rules without replacing existing rules or
/// accepting their IDs. The complete batch is validated before one atomic JSON
/// update, and repeated imports skip rules with the same behavior.
pub async fn import_replacement_rules_json(
    store: &ResourceStore,
    source_json: &str,
) -> Result<(ResourceRef, usize), String> {
    let imported = parse_replacement_rules_json(source_json)?;
    let imported_ids = imported
        .iter()
        .map(|_| format!("replacement-{}", uuid::Uuid::new_v4().simple()))
        .collect::<Vec<_>>();
    let count_ids = imported_ids.clone();
    let updated = store
        .update_json_ref_or_default(
            &replacements_ref(),
            empty_replacement_document(),
            move |mut value| {
                let mut document = replacement_document(&mut value)?;
                let rules = document
                    .get_mut("rules")
                    .and_then(Value::as_array_mut)
                    .expect("replacement document has an array");
                for (input, id) in imported.into_iter().zip(imported_ids.iter()) {
                    let already_present = rules.iter().any(|candidate| {
                        candidate.get("pattern").and_then(Value::as_str)
                            == Some(input.pattern.as_str())
                            && candidate.get("replacement").and_then(Value::as_str)
                                == Some(input.replacement.as_str())
                            && candidate.get("enabled").and_then(Value::as_bool)
                                == Some(input.enabled)
                            && candidate.get("isRegex").and_then(Value::as_bool)
                                == Some(input.is_regex)
                            && candidate.get("scope").and_then(Value::as_str)
                                == Some(input.scope.as_str())
                    });
                    if already_present {
                        continue;
                    }
                    rules.push(json!({
                        "id": id,
                        "name": input.name,
                        "pattern": input.pattern,
                        "replacement": input.replacement,
                        "enabled": input.enabled,
                        "isRegex": input.is_regex,
                        "scope": input.scope,
                    }));
                }
                document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
                Ok(document)
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    let imported_count = updated
        .get("rules")
        .and_then(Value::as_array)
        .map(|rules| {
            count_ids
                .iter()
                .filter(|id| {
                    rules
                        .iter()
                        .any(|rule| rule.get("id").and_then(Value::as_str) == Some(id.as_str()))
                })
                .count()
        })
        .unwrap_or_default();
    Ok((replacements_ref(), imported_count))
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

/// Save rule configuration only. The frontend applies enabled rules to its
/// rendered, engine-processed chapter; this module does not transform content.
pub async fn upsert_replacement_rule(
    store: &ResourceStore,
    input: DisplayReplacementRuleInput,
) -> Result<(ResourceRef, String), String> {
    let (name, scope) = validate_replacement_rule_input(&input)?;
    let updating = input.id.is_some();
    let id = match input.id {
        Some(id) => {
            validate_id(&id, "replacementRuleId")?;
            id
        }
        None => format!("replacement-{}", uuid::Uuid::new_v4().simple()),
    };
    let saved_id = id.clone();
    store
        .update_json_ref_or_default(
            &replacements_ref(),
            empty_replacement_document(),
            move |mut value| {
                let mut document = replacement_document(&mut value)?;
                let rules = document
                    .get_mut("rules")
                    .and_then(Value::as_array_mut)
                    .expect("default replacement document has an array");
                let rule = json!({
                    "id": id,
                    "name": name,
                    "pattern": input.pattern,
                    "replacement": input.replacement,
                    "enabled": input.enabled,
                    "isRegex": input.is_regex,
                    "scope": scope,
                });
                if let Some(position) = rules.iter().position(|candidate| {
                    candidate.get("id").and_then(Value::as_str) == Some(id.as_str())
                }) {
                    rules[position] = rule;
                } else if updating {
                    return Err("Replacement rule to update was not found".to_owned());
                } else {
                    rules.push(rule);
                }
                document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
                Ok(document)
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok((replacements_ref(), saved_id))
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

pub async fn delete_replacement_rules(
    store: &ResourceStore,
    rule_ids: Vec<String>,
) -> Result<ResourceRef, String> {
    if rule_ids.is_empty() {
        return Err("Replacement rule delete selection is empty".to_owned());
    }
    let targets = rule_ids.into_iter().collect::<HashSet<_>>();
    for id in &targets {
        validate_id(id, "replacementRuleId")?;
    }
    store
        .update_json_ref_or_default(
            &replacements_ref(),
            empty_replacement_document(),
            move |mut value| {
                let mut document = replacement_document(&mut value)?;
                let rules = document
                    .get_mut("rules")
                    .and_then(Value::as_array_mut)
                    .expect("default replacement document has an array");
                let original_len = rules.len();
                rules.retain(|rule| {
                    !rule
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| targets.contains(id))
                });
                if rules.len() == original_len {
                    return Err("显示替换规则不存在".to_owned());
                }
                document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
                Ok(document)
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(replacements_ref())
}

fn replacement_document(value: &mut Value) -> Result<Value, String> {
    let Some(object) = value.as_object_mut() else {
        *value = empty_replacement_document();
        return Ok(value.clone());
    };
    object
        .entry("schemaVersion")
        .or_insert_with(|| json!(CURRENT_SCHEMA_VERSION));
    object.entry("rules").or_insert_with(|| json!([]));
    if value["schemaVersion"].as_u64() != Some(u64::from(CURRENT_SCHEMA_VERSION)) {
        *value = empty_replacement_document();
        return Ok(value.clone());
    }
    let Ok(document) = serde_json::from_value::<ReplacementRuleDocument>(value.clone()) else {
        *value = empty_replacement_document();
        return Ok(value.clone());
    };
    let mut ids = HashSet::with_capacity(document.rules.len());
    for rule in document.rules {
        if !ids.insert(rule.id.clone()) {
            *value = empty_replacement_document();
            return Ok(value.clone());
        }
        let input = DisplayReplacementRuleInput {
            id: Some(rule.id),
            name: rule.name,
            pattern: rule.pattern,
            replacement: rule.replacement,
            enabled: rule.enabled,
            is_regex: rule.is_regex,
            scope: rule.scope,
        };
        if input
            .id
            .as_deref()
            .map_or(true, |id| validate_id(id, "replacementRuleId").is_err())
            || validate_replacement_rule_input(&input).is_err()
        {
            *value = empty_replacement_document();
            return Ok(value.clone());
        }
    }
    Ok(value.clone())
}

fn empty_replacement_document() -> Value {
    serde_json::to_value(ReplacementRuleDocument::default())
        .expect("replacement defaults serialize")
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
