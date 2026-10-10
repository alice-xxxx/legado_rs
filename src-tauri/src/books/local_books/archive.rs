//! 校验 ZIP 成员路径、条目数量和解压体积。

use std::{collections::HashMap, io::Read};

use zip::ZipArchive;
pub(super) fn index_zip_members<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    max_files: usize,
    max_expanded_bytes: u64,
    format: &str,
) -> Result<HashMap<String, usize>, String> {
    if archive.len() > max_files {
        return Err(format!("{format} contains too many ZIP entries"));
    }
    let mut names = HashMap::new();
    let mut expanded_total = 0u64;
    for index in 0..archive.len() {
        let member = archive
            .by_index(index)
            .map_err(|_| format!("Cannot inspect {format} ZIP entry"))?;
        let name = member.name().to_owned();
        if member
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!(
                "{format} must not contain symbolic-link ZIP entries"
            ));
        }
        let normalized = validate_zip_member_name(&name)
            .ok_or_else(|| format!("{format} contains an unsafe ZIP entry path"))?;
        if member.is_dir() {
            continue;
        }
        if names.insert(normalized, index).is_some() {
            return Err(format!("{format} contains duplicate ZIP entry names"));
        }
        expanded_total = expanded_total.saturating_add(member.size());
        if expanded_total > max_expanded_bytes {
            return Err(format!("{format} expands beyond the supported size limit"));
        }
    }
    Ok(names)
}

fn validate_zip_member_name(name: &str) -> Option<String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.contains(':')
    {
        return None;
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return None;
    }
    let parts = trimmed.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return None;
    }
    Some(trimmed.to_owned())
}

pub(super) fn read_archive_member<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    members: &HashMap<String, usize>,
    name: &str,
    max_size: u64,
    format: &str,
) -> Result<Vec<u8>, String> {
    let index = *members
        .get(name)
        .ok_or_else(|| format!("{format} entry is missing: {name}"))?;
    let member = archive
        .by_index(index)
        .map_err(|_| format!("Cannot open {format} ZIP entry"))?;
    if member.size() > max_size {
        return Err(format!("{format} entry exceeds the supported size limit"));
    }
    let mut bytes = Vec::with_capacity(member.size() as usize);
    member
        .take(max_size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("Cannot decompress {format} ZIP entry"))?;
    if bytes.len() as u64 > max_size {
        return Err(format!(
            "{format} entry expands beyond the supported size limit"
        ));
    }
    Ok(bytes)
}
