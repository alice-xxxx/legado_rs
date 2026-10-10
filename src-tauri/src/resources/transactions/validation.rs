//! 校验事务目标、数据内容和安全路径规则。

use super::*;
use std::collections::HashSet;
use std::path::{Component, Path};

pub(super) fn validate_unique_entries(
    replacements: &[Replacement],
    deletes: &[PostCommitDelete],
) -> Result<(), TransactionError> {
    let mut targets = HashSet::new();
    for replacement in replacements {
        let path = replacement.target.relative_path();
        if !targets.insert(path) {
            return Err(TransactionError::new("Duplicate transaction target"));
        }
    }
    let mut delete_paths = HashSet::<&str>::new();
    for delete in deletes {
        if !is_chapter_resource_path(&delete.path) && !is_book_directory_path(&delete.path) {
            return Err(TransactionError::new(
                "Invalid typed post-commit cleanup path",
            ));
        }
        if !delete_paths.insert(delete.path.as_str()) {
            return Err(TransactionError::new("Duplicate post-commit cleanup path"));
        }
    }
    for delete in deletes {
        if has_cleanup_ancestor(&delete.path, &delete_paths) {
            return Err(TransactionError::new("Overlapping post-commit cleanup paths"));
        }
    }
    for replacement in replacements {
        if replacement.bytes.is_none() {
            // Deleting book.json before removing the book directory is valid.
            continue;
        }
        let target = replacement.target.relative_path();
        if delete_paths.contains(target.as_str())
            || has_cleanup_ancestor(&target, &delete_paths)
        {
            return Err(TransactionError::new(
                "Post-commit cleanup would delete a newly written transaction target",
            ));
        }
    }
    Ok(())
}

/// Only path components can be ancestors. This avoids pairwise comparisons
/// over every chapter in large cache-clearing transactions.
pub(super) fn has_cleanup_ancestor(path: &str, cleanup_paths: &HashSet<&str>) -> bool {
    path.bytes().enumerate().any(|(index, byte)| {
        byte == b'/' && cleanup_paths.contains(&path[..index])
    })
}

pub(super) fn validate_json_bytes(bytes: &[u8]) -> Result<(), TransactionError> {
    validate_json_size(bytes)?;
    serde_json::from_slice::<serde_json::Value>(bytes)
        .map(|_| ())
        .map_err(|error| {
            TransactionError::new(format!("Transaction target is not valid JSON: {error}"))
        })
}

pub(super) fn validate_public_json_bytes(bytes: &[u8]) -> Result<(), TransactionError> {
    validate_json_size(bytes)?;
    crate::resources::validate_persistable_json_bytes(bytes)
        .map_err(|error| TransactionError::new(format!("Invalid public transaction JSON: {error}")))
}

pub(super) fn validate_json_size(bytes: &[u8]) -> Result<(), TransactionError> {
    if bytes.len() as u64 > MAX_TARGET_BYTES {
        return Err(TransactionError::new(
            "A transaction JSON document exceeds the 64 MiB safety limit",
        ));
    }
    Ok(())
}

pub(super) fn validate_target_payload(
    target: &TransactionTarget,
    bytes: &[u8],
) -> Result<(), TransactionError> {
    match target {
        TransactionTarget::PublicJson(_) => validate_public_json_bytes(bytes),
        TransactionTarget::PrivateBookJson(_) => validate_json_bytes(bytes),
    }
}

pub(super) fn is_deletable_target(target: &TransactionTarget) -> bool {
    match target {
        TransactionTarget::PublicJson(path) => {
            is_book_json_path(path) || is_progress_json_path(path)
        }
        TransactionTarget::PrivateBookJson(_) => true,
    }
}

pub(super) fn read_validated_payload(
    stage_dir: &Path,
    target: &TransactionTarget,
    snapshot: &Snapshot,
) -> Result<Vec<u8>, TransactionError> {
    let bytes = read_payload(stage_dir, snapshot)?;
    validate_target_payload(target, &bytes)?;
    Ok(bytes)
}

pub(super) fn validate_operation(operation: &str) -> Result<(), TransactionError> {
    if operation.is_empty()
        || operation.len() > MAX_OPERATION_BYTES
        || !operation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(TransactionError::new("Invalid transaction operation name"));
    }
    Ok(())
}

pub(super) fn validate_transaction_id(transaction_id: &str) -> Result<(), TransactionError> {
    let Some(uuid) = transaction_id.strip_prefix("txn-") else {
        return Err(TransactionError::new("Invalid transaction identifier"));
    };
    if uuid.len() != 32 || !uuid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TransactionError::new("Invalid transaction identifier"));
    }
    Ok(())
}

pub(super) fn validate_identifier(id: &str) -> Result<(), TransactionError> {
    if id.is_empty()
        || id.len() > 255
        || id == "."
        || id == ".."
        || id.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err(TransactionError::new("Invalid transaction book identifier"));
    }
    Ok(())
}

pub(super) fn is_chapter_resource_path(path: &str) -> bool {
    chapter_resource_id(path, ".json").is_some()
}

fn chapter_resource_id<'a>(path: &'a str, suffix: &str) -> Option<&'a str> {
    let components = path.split('/').collect::<Vec<_>>();
    if components.len() != 4
        || components[0] != "books"
        || !valid_identifier(components[1])
        || components[2] != "chapters"
    {
        return None;
    }
    components[3]
        .strip_suffix(suffix)
        .filter(|id| valid_identifier(id))
}

pub(super) fn is_book_json_path(path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    components.len() == 3
        && components[0] == "books"
        && valid_identifier(components[1])
        && components[2] == "book.json"
}

pub(super) fn is_progress_json_path(path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    components.len() == 2
        && components[0] == "progress"
        && components[1]
            .strip_suffix(".json")
            .is_some_and(valid_identifier)
}

pub(super) fn is_book_directory_path(path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    components.len() == 2
        && components[0] == "books"
        && valid_identifier(components[1])
        && safe_components(path).is_ok()
}

pub(super) fn valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id != "."
        && id != ".."
        && !id.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

pub(super) fn safe_components(path: &str) -> Result<Vec<&str>, TransactionError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(TransactionError::new("Unsafe transaction path"));
    }
    let components: Vec<_> = path.split('/').collect();
    if components.iter().any(|component| {
        component.is_empty()
            || *component == "."
            || *component == ".."
            || component.ends_with(['.', ' '])
            || is_windows_reserved_component(component)
    }) {
        return Err(TransactionError::new("Unsafe transaction path component"));
    }
    let parsed = Path::new(path);
    if parsed
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(TransactionError::new("Transaction path must be relative"));
    }
    Ok(components)
}

pub(super) fn is_windows_reserved_component(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit()
        && stem.as_bytes()[3] != b'0')
}

pub(super) fn add_snapshot_size(total: &mut u64, next: u64) -> Result<(), TransactionError> {
    *total = total
        .checked_add(next)
        .ok_or_else(|| TransactionError::new("Transaction snapshot size overflow"))?;
    if *total > MAX_TRANSACTION_BYTES {
        return Err(TransactionError::new(
            "Transaction snapshots exceed the 128 MiB safety limit",
        ));
    }
    Ok(())
}
