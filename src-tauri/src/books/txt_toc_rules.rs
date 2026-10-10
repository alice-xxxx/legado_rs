//! 定义本地 TXT 目录识别规则的读取与保存。
//! User-managed regular expressions for recognizing local TXT chapter titles.
//!
//! These are local import preferences. They are persisted as an ordinary
//! public JSON resource and are evaluated by Rust; they are not book-source
//! rules and are never handed to the source engine.

use std::collections::{HashMap, HashSet};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::resources::{ResourceRef, ResourceStore};

pub const RESOURCE_NAME: &str = "txt-toc-rules";
pub const SCHEMA_VERSION: u32 = 1;

const MAX_RULES: usize = 64;
const MAX_RULE_ID_BYTES: usize = 128;
const MAX_RULE_NAME_BYTES: usize = 256;
const MAX_RULE_PATTERN_BYTES: usize = 4096;
const MAX_RULE_EXAMPLE_BYTES: usize = 2048;
pub const MAX_IMPORT_BODY_BYTES: usize = 1024 * 1024;
const MAX_REGEX_COMPILED_BYTES: usize = 1024 * 1024;
const MAX_CHAPTER_TITLE_CHARS: usize = 200;
const MAX_TEXT_CHAPTERS: usize = 20_000;
const CHAPTER_HEADING_SCORE_GAP: usize = 1000;

/// A rule document that the UI may read and edit as JSON.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TxtTocRulesDocument {
    pub schema_version: u32,
    pub rules: Vec<TxtTocRule>,
}

impl Default for TxtTocRulesDocument {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            rules: Vec::new(),
        }
    }
}

/// A saved line pattern for local TXT files. Field names preserve the existing
/// TxtTocRule vocabulary while IDs use stable strings in the new app schema.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TxtTocRule {
    pub id: String,
    pub name: String,
    pub rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    pub serial_number: i32,
    pub enable: bool,
}

#[derive(Debug)]
pub enum TxtTocRulesEdit {
    Save { id: Option<String>, rule: TxtTocRule },
    Reorder { ordered_ids: Vec<String> },
    Delete { rule_ids: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocHeading {
    pub title: String,
    /// Byte range for the complete matched heading line, excluding newline.
    pub line_start: usize,
    pub line_end: usize,
    /// Byte offset immediately after the heading line and its newline.
    pub body_start: usize,
}

pub fn resource_ref(store: &ResourceStore) -> Result<ResourceRef, String> {
    store
        .reading_ref(RESOURCE_NAME)
        .map_err(|error| error.to_string())
}

/// Publish a content-addressed rule snapshot so a browser descriptor can never
/// resolve to a later edit of the mutable settings document.
pub async fn publish_snapshot(
    store: &ResourceStore,
    document: &TxtTocRulesDocument,
) -> Result<ResourceRef, String> {
    let bytes = serde_json::to_vec(document)
        .map_err(|error| format!("Cannot encode TXT TOC rules snapshot: {error}"))?;
    let digest = Sha256::digest(bytes);
    let name = format!("txt-toc-rules-snapshot-{digest:x}");
    let reference = store
        .reading_ref(&name)
        .map_err(|error| error.to_string())?;
    store
        .write_serializable(&reference, document)
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Load the user's rules. An absent file is initialized atomically as an empty
/// versioned JSON document so the resource is immediately available to the UI.
pub async fn read_document(store: &ResourceStore) -> Result<TxtTocRulesDocument, String> {
    let reference = resource_ref(store)?;
    let default = serde_json::to_value(TxtTocRulesDocument::default())
        .map_err(|error| format!("Cannot encode TXT TOC rules: {error}"))?;
    let value = store
        .update_json_ref_or_default(&reference, default, |current| {
            let document = decode_current(current).unwrap_or_default();
            serde_json::to_value(document)
                .map_err(|error| format!("Cannot encode TXT TOC rules: {error}"))
        })
        .await
        .map_err(|error| error.to_string())?;
    let document = decode_document(value)?;
    validate_document(&document)?;
    Ok(document)
}

/// Replace the full JSON document with an atomically validated snapshot.
pub async fn replace_document(
    store: &ResourceStore,
    document: &TxtTocRulesDocument,
) -> Result<(), String> {
    validate_document(document)?;
    store
        .write_serializable(&resource_ref(store)?, document)
        .await
        .map_err(|error| error.to_string())
}

/// Create a rule without an ID or update the existing rule named by the ID.
pub async fn save_rule(
    store: &ResourceStore,
    id: Option<String>,
    mut rule: TxtTocRule,
) -> Result<(TxtTocRulesDocument, String), String> {
    let updating = id.is_some();
    let id = id.unwrap_or_else(|| format!("txt-toc-{}", uuid::Uuid::new_v4().simple()));
    rule.id = id.clone();
    validate_rule(&rule)?;
    let reference = resource_ref(store)?;
    let value = store
        .update_json_ref_or_default(&reference, empty_document_value(), move |current| {
            let mut document = decode_current(current)?;
            let position = document.rules.iter().position(|saved| saved.id == rule.id);
            match (updating, position) {
                (true, Some(position)) => document.rules[position] = rule,
                (true, None) => return Err("TXT TOC rule to update was not found".to_owned()),
                (false, Some(_)) => {
                    return Err("Generated TXT TOC rule ID already exists".to_owned());
                }
                (false, None) => document.rules.push(rule),
            }
            validate_document(&document)?;
            serde_json::to_value(document)
                .map_err(|error| format!("Cannot encode TXT TOC rules: {error}"))
        })
        .await
        .map_err(|error| error.to_string())?;
    let document = decode_document(value)?;
    validate_document(&document)?;
    Ok((document, id))
}

struct ImportedTxtTocRule {
    name: String,
    rule: String,
    example: Option<String>,
    enable: bool,
    priority: i64,
    original_index: usize,
}

fn parse_imported_rule(value: &Value, index: usize) -> Result<ImportedTxtTocRule, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("TXT TOC rule {} must be an object", index + 1))?;
    let known = ["id", "name", "rule", "example", "serialNumber", "enable"];
    let unknown = object
        .keys()
        .filter(|key| !known.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(format!(
            "TXT TOC rule {} contains unknown fields: {}",
            index + 1,
            unknown.join(", ")
        ));
    }
    if let Some(id) = object.get("id") {
        if !id.is_string() && safe_json_integer(id).is_none() {
            return Err(format!("TXT TOC rule {} has an invalid ID", index + 1));
        }
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= MAX_RULE_NAME_BYTES)
        .ok_or_else(|| format!("TXT TOC rule {} has an invalid name", index + 1))?
        .to_owned();
    let rule = object
        .get("rule")
        .and_then(Value::as_str)
        .filter(|rule| !rule.trim().is_empty() && rule.len() <= MAX_RULE_PATTERN_BYTES)
        .ok_or_else(|| format!("TXT TOC rule {} has an invalid pattern", index + 1))?
        .to_owned();
    let example = match object.get("example") {
        None | Some(Value::Null) => None,
        Some(Value::String(example)) if example.len() <= MAX_RULE_EXAMPLE_BYTES => {
            Some(example.clone())
        }
        _ => return Err(format!("TXT TOC rule {} has an invalid example", index + 1)),
    };
    let enable = match object.get("enable") {
        None => true,
        Some(Value::Bool(enable)) => *enable,
        _ => {
            return Err(format!(
                "TXT TOC rule {} has an invalid enable flag",
                index + 1
            ))
        }
    };
    let priority = match object.get("serialNumber") {
        None => -1,
        Some(value) => safe_json_integer(value)
            .ok_or_else(|| format!("TXT TOC rule {} has an invalid serialNumber", index + 1))?,
    };
    Ok(ImportedTxtTocRule {
        name,
        rule,
        example,
        enable,
        priority,
        original_index: index,
    })
}

fn safe_json_integer(value: &Value) -> Option<i64> {
    const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
    let integer = value.as_i64().or_else(|| {
        value.as_f64().and_then(|number| {
            (number.is_finite()
                && number.fract() == 0.0
                && number >= -(MAX_SAFE_INTEGER as f64)
                && number <= MAX_SAFE_INTEGER as f64)
                .then_some(number as i64)
        })
    })?;
    (integer >= -MAX_SAFE_INTEGER && integer <= MAX_SAFE_INTEGER).then_some(integer)
}

/// Parse and append Legado rules atomically. Imported IDs are replaced with
/// app-owned IDs, and invalid input leaves the stored document unchanged.
pub async fn import_rules_json(
    store: &ResourceStore,
    source_json: &str,
) -> Result<(TxtTocRulesDocument, usize), String> {
    if source_json.len() > MAX_IMPORT_BODY_BYTES {
        return Err("TXT TOC rule JSON exceeds the 1 MiB limit".to_owned());
    }
    let source: Value = serde_json::from_str(source_json)
        .map_err(|error| format!("Invalid TXT TOC rule JSON: {error}"))?;
    let input = if let Some(object) = source.as_object() {
        let unknown = object
            .keys()
            .filter(|key| !matches!(key.as_str(), "rules" | "schemaVersion"))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(format!(
                "TXT TOC rules document contains unknown fields: {}",
                unknown.join(", ")
            ));
        }
        if object
            .get("schemaVersion")
            .is_some_and(|version| safe_json_integer(version) != Some(SCHEMA_VERSION as i64))
        {
            return Err("Unsupported TXT TOC rules schema version".to_owned());
        }
        object
            .get("rules")
            .and_then(Value::as_array)
            .ok_or_else(|| "TXT TOC rules document must contain a rules array".to_owned())?
    } else {
        source.as_array().ok_or_else(|| {
            "TXT TOC import must be a Legado rule array or versioned document".to_owned()
        })?
    };
    if input.is_empty() {
        return Err("TXT TOC import is empty".to_owned());
    }
    if input.len() > MAX_RULES {
        return Err(format!("TXT TOC import cannot exceed {MAX_RULES} rules"));
    }
    let mut imported = input
        .iter()
        .enumerate()
        .map(|(index, value)| parse_imported_rule(value, index))
        .collect::<Result<Vec<_>, _>>()?;
    imported.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.original_index.cmp(&right.original_index))
    });
    let imported_count = imported.len();
    let reference = resource_ref(store)?;
    let value = store
        .update_json_ref_or_default(&reference, empty_document_value(), move |current| {
            let mut document = decode_current(current)?;
            if document.rules.len() + imported_count > MAX_RULES {
                return Err(format!("TXT TOC rules cannot exceed {MAX_RULES} entries"));
            }
            let first_serial_number = document
                .rules
                .iter()
                .map(|rule| rule.serial_number)
                .max()
                .unwrap_or(0)
                .max(0)
                .checked_add(1)
                .ok_or_else(|| "TXT TOC rule priority has reached the maximum value".to_owned())?;
            if first_serial_number as i64 + imported_count as i64 - 1 > i32::MAX as i64 {
                return Err("TXT TOC rule priority has reached the maximum value".to_owned());
            }
            for (index, imported) in imported.into_iter().enumerate() {
                let rule = TxtTocRule {
                    id: format!("txt-toc-{}-{index}", uuid::Uuid::new_v4().simple()),
                    name: imported.name,
                    rule: imported.rule,
                    example: imported.example.filter(|example| !example.is_empty()),
                    serial_number: first_serial_number + index as i32,
                    enable: imported.enable,
                };
                validate_rule(&rule)?;
                document.rules.push(rule);
            }
            validate_document(&document)?;
            serde_json::to_value(document)
                .map_err(|error| format!("Cannot encode TXT TOC rules: {error}"))
        })
        .await
        .map_err(|error| error.to_string())?;
    let document = decode_document(value)?;
    validate_document(&document)?;
    Ok((document, imported_count))
}

/// Reorder the complete set in one transaction. Reject stale lists and duplicates
/// instead of silently removing or overwriting any newly added rules.
pub async fn reorder_rules(
    store: &ResourceStore,
    ordered_ids: Vec<String>,
) -> Result<TxtTocRulesDocument, String> {
    if ordered_ids.len() > MAX_RULES {
        return Err(format!("TXT TOC rules cannot exceed {MAX_RULES} entries"));
    }
    let reference = resource_ref(store)?;
    let value = store
        .update_json_ref_or_default(&reference, empty_document_value(), move |current| {
            let mut document = decode_current(current)?;
            if document.rules.len() != ordered_ids.len() {
                return Err(
                    "TXT TOC rules changed during reorder; refresh before retrying".to_owned(),
                );
            }
            let mut positions = HashMap::with_capacity(ordered_ids.len());
            for (index, id) in ordered_ids.into_iter().enumerate() {
                validate_rule_id(&id)?;
                if positions.insert(id.clone(), index as i32).is_some() {
                    return Err(format!("Duplicate TXT TOC reorder ID: {id}"));
                }
            }
            for rule in &mut document.rules {
                rule.serial_number = positions
                    .remove(&rule.id)
                    .ok_or_else(|| format!("TXT TOC reorder missing rule: {}", rule.id))?;
            }
            if !positions.is_empty() {
                return Err("TXT TOC reorder includes unknown rules".to_owned());
            }
            validate_document(&document)?;
            serde_json::to_value(document)
                .map_err(|error| format!("Cannot encode TXT TOC rules: {error}"))
        })
        .await
        .map_err(|error| error.to_string())?;
    let document = decode_document(value)?;
    validate_document(&document)?;
    Ok(document)
}

/// Delete multiple rules in one transaction. A failed validation leaves all
/// saved rules untouched; unknown IDs are harmless and deletion is idempotent.
pub async fn delete_rules(
    store: &ResourceStore,
    rule_ids: Vec<String>,
) -> Result<TxtTocRulesDocument, String> {
    if rule_ids.is_empty() {
        return Err("TXT TOC delete selection is empty".to_owned());
    }
    for id in &rule_ids {
        validate_rule_id(id)?;
    }
    let targets = rule_ids.into_iter().collect::<HashSet<_>>();
    let reference = resource_ref(store)?;
    let value = store
        .update_json_ref_or_default(&reference, empty_document_value(), move |current| {
            let mut document = decode_current(current)?;
            document.rules.retain(|rule| !targets.contains(&rule.id));
            validate_document(&document)?;
            serde_json::to_value(document)
                .map_err(|error| format!("Cannot encode TXT TOC rules: {error}"))
        })
        .await
        .map_err(|error| error.to_string())?;
    let document = decode_document(value)?;
    validate_document(&document)?;
    Ok(document)
}

/// Validate a document loaded from disk or a backup. Every stored pattern is
/// compiled with a fixed regex size cap before it can be saved or restored.
pub fn validate_document(document: &TxtTocRulesDocument) -> Result<(), String> {
    if document.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "Unsupported TXT TOC rules schema version: {}",
            document.schema_version
        ));
    }
    if document.rules.len() > MAX_RULES {
        return Err(format!("TXT TOC rules cannot exceed {MAX_RULES} entries"));
    }

    let mut ids = HashSet::with_capacity(document.rules.len());
    for rule in &document.rules {
        validate_rule(rule)?;
        if !ids.insert(rule.id.as_str()) {
            return Err(format!("Duplicate TXT TOC rule ID: {}", rule.id));
        }
    }
    Ok(())
}

pub fn validate_rule(rule: &TxtTocRule) -> Result<(), String> {
    validate_rule_id(&rule.id)?;
    if rule.name.trim().is_empty() || rule.name.len() > MAX_RULE_NAME_BYTES {
        return Err("TXT TOC rule name must contain 1–256 bytes".to_owned());
    }
    if rule.name.chars().any(char::is_control) {
        return Err("TXT TOC rule name cannot contain control characters".to_owned());
    }
    if rule.rule.trim().is_empty() || rule.rule.len() > MAX_RULE_PATTERN_BYTES {
        return Err("TXT TOC rule pattern must contain 1–4096 bytes".to_owned());
    }
    if rule
        .example
        .as_ref()
        .is_some_and(|example| example.len() > MAX_RULE_EXAMPLE_BYTES)
    {
        return Err("TXT TOC rule example exceeds 2048 bytes".to_owned());
    }
    compile_pattern(&rule.rule)?;
    Ok(())
}

pub fn compile_pattern(pattern: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .multi_line(true)
        .size_limit(MAX_REGEX_COMPILED_BYTES)
        .build()
        .map_err(|error| format!("Invalid TXT TOC expression: {error}"))
}

/// Select the best enabled rule. Score counts distinct chapter-like headings
/// separated by at least 1000 characters, matching the old import heuristic. Ties
/// are deterministic: lower serial number first, then lexicographic ID.
pub fn best_rule_headings(
    text: &str,
    rules: &[TxtTocRule],
) -> Result<(bool, Option<Vec<TocHeading>>), String> {
    let mut enabled = rules.iter().filter(|rule| rule.enable).collect::<Vec<_>>();
    enabled.sort_by(|left, right| {
        left.serial_number
            .cmp(&right.serial_number)
            .then_with(|| left.id.cmp(&right.id))
    });
    if enabled.is_empty() {
        return Ok((false, None));
    }

    let mut best: Option<(i32, String, usize, Vec<TocHeading>)> = None;
    for rule in enabled {
        let pattern = compile_pattern(&rule.rule)?;
        let headings = match collect_line_headings(text, &pattern) {
            Ok(headings) => headings,
            Err(error) if error == TOO_MANY_HEADINGS => continue,
            Err(error) => return Err(error),
        };
        if headings.is_empty() {
            continue;
        }
        let score = score_headings(text, &headings);
        let candidate = (rule.serial_number, rule.id.clone(), score, headings);
        let should_replace = best.as_ref().is_none_or(|current| {
            candidate.2 > current.2
                || (candidate.2 == current.2
                    && (candidate.0, candidate.1.as_str()) < (current.0, current.1.as_str()))
        });
        if should_replace {
            best = Some(candidate);
        }
    }
    Ok((true, best.map(|(_, _, _, headings)| headings)))
}

pub fn pattern_headings(text: &str, pattern: &Regex) -> Result<Vec<TocHeading>, String> {
    collect_line_headings(text, pattern)
}

const TOO_MANY_HEADINGS: &str = "TXT TOC pattern recognized too many headings";

fn collect_line_headings(text: &str, pattern: &Regex) -> Result<Vec<TocHeading>, String> {
    let mut headings = Vec::new();
    let mut line_start = 0;
    for raw_line in text.split_inclusive('\n') {
        let line_with_cr_removed = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = line_with_cr_removed
            .strip_suffix('\r')
            .unwrap_or(line_with_cr_removed);
        let line_end = line_start + line.len();
        let body_start = line_start + raw_line.len();
        let Some(captures) = pattern.captures(line) else {
            line_start = body_start;
            continue;
        };
        let matched = captures
            .get(0)
            .ok_or_else(|| "TXT TOC pattern returned an empty match".to_owned())?;
        if !line[..matched.start()].trim().is_empty()
            || !line[matched.end()..].trim().is_empty()
            || matched.as_str().trim().is_empty()
        {
            line_start = body_start;
            continue;
        }

        let title = captures
            .get(1)
            .unwrap_or(matched)
            .as_str()
            .trim()
            .to_owned();
        if title.is_empty() || title.chars().count() > MAX_CHAPTER_TITLE_CHARS {
            line_start = body_start;
            continue;
        }
        if headings.len() == MAX_TEXT_CHAPTERS {
            return Err(TOO_MANY_HEADINGS.to_owned());
        }

        headings.push(TocHeading {
            title,
            line_start,
            line_end,
            body_start,
        });
        line_start = body_start;
    }
    Ok(headings)
}

fn score_headings(text: &str, headings: &[TocHeading]) -> usize {
    let mut score = 0;
    let mut previous_end = None;
    for heading in headings {
        let gap = previous_end.map_or(usize::MAX, |end: usize| {
            text[end..heading.line_start].chars().count()
        });
        if gap > CHAPTER_HEADING_SCORE_GAP {
            score += 1;
            previous_end = Some(heading.line_end);
        }
    }
    score
}

fn validate_rule_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > MAX_RULE_ID_BYTES
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("TXT TOC rule ID must use ASCII letters, digits, '-' or '_'".to_owned());
    }
    Ok(())
}

fn decode_document(value: Value) -> Result<TxtTocRulesDocument, String> {
    serde_json::from_value(value).map_err(|error| format!("Invalid TXT TOC rules JSON: {error}"))
}

fn decode_current(value: Value) -> Result<TxtTocRulesDocument, String> {
    if value.is_null() {
        return Ok(TxtTocRulesDocument::default());
    }
    Ok(decode_document(value)
        .and_then(|document| {
            validate_document(&document)?;
            Ok(document)
        })
        .unwrap_or_default())
}

fn empty_document_value() -> Value {
    serde_json::to_value(TxtTocRulesDocument::default()).expect("TXT TOC defaults serialize")
}
