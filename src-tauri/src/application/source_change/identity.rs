//! 书源切换使用的身份匹配和指纹。

use super::*;

pub(in crate::application) fn chapter_id_for_generation(
    book_id: &str,
    generation: &str,
    stable_url: &str,
) -> String {
    format!(
        "chapter-{:016x}",
        stable_hash(&format!("{book_id}\0{generation}\0{stable_url}"))
    )
}

pub(in crate::application) fn normalize_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(in crate::application) fn candidate_identity_match(
    context: &Value,
    candidate: &Value,
) -> Option<bool> {
    let expected_title = context.get("targetTitle")?.as_str()?;
    let candidate_title = text_at(candidate, &["name", "title"])?;
    let expected_title = normalize_identity(expected_title);
    if expected_title.is_empty() || normalize_identity(&candidate_title) != expected_title {
        return None;
    }
    let expected_author = context
        .get("targetAuthor")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let candidate_author = text_at(candidate, &["author"]).unwrap_or_default();
    if !expected_author.trim().is_empty()
        && !candidate_author.trim().is_empty()
        && normalize_identity(expected_author) != normalize_identity(&candidate_author)
    {
        return None;
    }
    Some(expected_author.trim().is_empty() || candidate_author.trim().is_empty())
}

pub(in crate::application) fn source_definition_fingerprint(
    source: &Value,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(source)
        .map_err(|error| format!("Cannot fingerprint source definition: {error}"))?;
    Ok(sha256_hex(&bytes))
}

pub(in crate::application) async fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot remove partial book resource: {error}")),
    }
}
