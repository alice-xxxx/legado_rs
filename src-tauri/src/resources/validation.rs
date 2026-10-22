//! 路径边界和媒体类型，文件操作共用。
use super::ResourceError;
use std::path::{Component, Path};

pub(crate) fn validate_id(id: &str) -> Result<(), ResourceError> {
    if id.is_empty()
        || id.len() > 255
        || id == "."
        || id == ".."
        || id
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return Err(ResourceError::new("无效资源标识"));
    }
    Ok(())
}
pub(crate) fn validate_public_path(path: &str) -> Result<(), ResourceError> {
    let parts: Vec<_> = path.split('/').collect();
    for part in &parts {
        validate_id(part)?;
    }
    match parts.as_slice() {
        ["settings.json"] | ["proxy", _] => Ok(()),
        ["cache", kind, _, _] if super::CONTENT_TYPES.contains(kind) => Ok(()),
        _ => Err(ResourceError::new("资源不属于公开分发范围")),
    }
}
pub(crate) fn check_path_no_symlink(
    root: &Path,
    path: &Path,
    missing: bool,
) -> Result<(), ResourceError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ResourceError::new("路径超出根目录"))?;
    let mut current = root.to_owned();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err(ResourceError::new("路径包含非法分段"));
        };
        if part.to_string_lossy().contains(':') {
            return Err(ResourceError::new("路径含设备或数据流名称"));
        }
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ResourceError::new("不允许符号链接"))
            }
            Ok(_) => {}
            Err(error) if missing && error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
pub(crate) fn mime_type(path: &str) -> &'static str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "xhtml" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "m3u8" => "application/vnd.apple.mpegurl",
        "ts" => "video/mp2t",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "aac" => "audio/aac",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}
pub(super) fn percent_encode_path(path: &str) -> String {
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~/".contains(&byte) {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}
