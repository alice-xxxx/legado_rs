//! User-managed regular expressions for recognizing local TXT chapter titles.
//!
//! These are local import preferences. They are persisted as an ordinary
//! public JSON resource and are evaluated by Rust; they are not book-source
//! rules and are never handed to the source engine.

use std::collections::HashSet;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::resources::{ResourceRef, ResourceStore};

pub const RESOURCE_NAME: &str = "txt-toc-rules";
pub const SCHEMA_VERSION: u32 = 1;

const MAX_RULES: usize = 64;
const MAX_RULE_ID_BYTES: usize = 128;
const MAX_RULE_NAME_BYTES: usize = 256;
const MAX_RULE_PATTERN_BYTES: usize = 4096;
const MAX_RULE_EXAMPLE_BYTES: usize = 2048;
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

/// Load the user's rules. An absent file is initialized atomically as an empty
/// versioned JSON document so the resource is immediately available to the UI.
pub async fn read_document(store: &ResourceStore) -> Result<TxtTocRulesDocument, String> {
    let reference = resource_ref(store)?;
    let path = store.root().join(reference.path());
    match tokio::fs::symlink_metadata(&path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("TXT TOC rule resource cannot be a symbolic link".to_owned());
        }
        Ok(_) => {
            let value = store
                .read_json_ref(&reference)
                .await
                .map_err(|error| error.to_string())?;
            let document = decode_document(value)?;
            validate_document(&document)?;
            Ok(document)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let value = store
                .update_json_ref(&reference, |current| {
                    let document = if current.is_null() {
                        TxtTocRulesDocument::default()
                    } else {
                        let document = decode_document(current)?;
                        validate_document(&document)?;
                        document
                    };
                    serde_json::to_value(document)
                        .map_err(|error| format!("Cannot encode TXT TOC rules: {error}"))
                })
                .await
                .map_err(|error| error.to_string())?;
            let document = decode_document(value)?;
            validate_document(&document)?;
            Ok(document)
        }
        Err(error) => Err(format!("Cannot inspect TXT TOC rules: {error}")),
    }
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

/// Insert or replace one rule under ResourceStore's read/modify/write lock.
pub async fn upsert_rule(
    store: &ResourceStore,
    rule: TxtTocRule,
) -> Result<TxtTocRulesDocument, String> {
    validate_rule(&rule)?;
    let reference = resource_ref(store)?;
    let value = store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_current(current)?;
            match document.rules.iter_mut().find(|saved| saved.id == rule.id) {
                Some(saved) => *saved = rule,
                None => document.rules.push(rule),
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

/// Delete one saved rule atomically. Deleting an unknown ID is an idempotent
/// no-op, which makes repeated UI retries safe.
pub async fn delete_rule(
    store: &ResourceStore,
    rule_id: &str,
) -> Result<TxtTocRulesDocument, String> {
    validate_rule_id(rule_id)?;
    let reference = resource_ref(store)?;
    let rule_id = rule_id.to_owned();
    let value = store
        .update_json_ref(&reference, move |current| {
            let mut document = decode_current(current)?;
            document.rules.retain(|rule| rule.id != rule_id);
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
    let document = decode_document(value)?;
    validate_document(&document)?;
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::{
        best_rule_headings, delete_rule, read_document, resource_ref, upsert_rule,
        validate_document, TxtTocRule, TxtTocRulesDocument,
    };
    use crate::resources::ResourceStore;
    use tempfile::tempdir;

    fn rule(id: &str, pattern: &str, serial_number: i32) -> TxtTocRule {
        TxtTocRule {
            id: id.to_owned(),
            name: id.to_owned(),
            rule: pattern.to_owned(),
            example: None,
            serial_number,
            enable: true,
        }
    }

    #[tokio::test]
    async fn rule_service_persists_serves_updates_and_deletes_json() {
        let root = tempdir().unwrap();
        let store = ResourceStore::open(root.path()).unwrap();
        let empty = read_document(&store).await.unwrap();
        assert!(empty.rules.is_empty());
        assert!(root.path().join("reading/txt-toc-rules.json").is_file());

        let first = rule("chapter-lines", r"^(Chapter\s+\d+\s+.+)$", 20);
        let saved = upsert_rule(&store, first.clone()).await.unwrap();
        assert_eq!(saved.rules.len(), 1);
        assert_eq!(saved.rules[0], first);
        let updated = TxtTocRule {
            name: "Book chapter lines".to_owned(),
            ..first.clone()
        };
        let saved = upsert_rule(&store, updated.clone()).await.unwrap();
        assert_eq!(saved.rules.len(), 1);
        assert_eq!(saved.rules[0].name, "Book chapter lines");

        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let response = reqwest::get(server.url_for(&resource_ref(&store).unwrap()))
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let public: TxtTocRulesDocument =
            serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
        assert_eq!(public, saved);

        let reloaded_store = ResourceStore::open(root.path()).unwrap();
        assert_eq!(read_document(&reloaded_store).await.unwrap(), saved);
        let deleted = delete_rule(&store, "chapter-lines").await.unwrap();
        assert!(deleted.rules.is_empty());
        server.shutdown().await.unwrap();
    }

    #[test]
    fn best_rule_uses_chapter_spacing_then_stable_serial_and_id_ties() {
        let text = "Opening prose.\n\nZeta chapter\n\nAlpha chapter\n";
        let rules = vec![
            rule("z-rule", r"^(Zeta chapter)$", 10),
            rule("a-rule", r"^(Alpha chapter)$", 10),
        ];
        let (had_enabled, headings) = best_rule_headings(text, &rules).unwrap();
        assert!(had_enabled);
        let headings = headings.unwrap();
        assert_eq!(
            headings
                .iter()
                .map(|heading| heading.title.as_str())
                .collect::<Vec<_>>(),
            ["Alpha chapter"]
        );

        let body = "body line\n".repeat(120);
        let spaced_text = format!("Alpha chapter\n{body}Alpha chapter\n{body}Zeta chapter\n");
        let (had_enabled, headings) = best_rule_headings(&spaced_text, &rules).unwrap();
        assert!(had_enabled);
        assert_eq!(headings.unwrap().len(), 2);

        let disabled_rule = TxtTocRule {
            enable: false,
            ..rule("disabled", "x", 0)
        };
        let (had_enabled, headings) =
            best_rule_headings("No headings here.", &[disabled_rule]).unwrap();
        assert!(!had_enabled);
        assert!(headings.is_none());
    }

    #[test]
    fn invalid_or_duplicate_rules_are_rejected_before_persisting() {
        let invalid = TxtTocRulesDocument {
            schema_version: 1,
            rules: vec![rule("bad", "(", 0)],
        };
        assert!(validate_document(&invalid).is_err());
        let duplicate = TxtTocRulesDocument {
            schema_version: 1,
            rules: vec![rule("same", r"^One$", 0), rule("same", r"^Two$", 1)],
        };
        assert!(validate_document(&duplicate)
            .unwrap_err()
            .contains("Duplicate"));
    }
}
