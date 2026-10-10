//! 校验稳定资源路径、标识符和本地文件访问。

use std::path::{Component, Path};

use super::{ResourceError, ResourceRef};

pub(super) fn validate_public_path(path: &str) -> Result<(), ResourceError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('%') {
        return Err(ResourceError::new("Invalid resource path"));
    }
    let path_components: Vec<&str> = path.split('/').collect();
    if path_components
        .iter()
        .any(|component| component.is_empty() || *component == "." || *component == "..")
    {
        return Err(ResourceError::new("Invalid resource path component"));
    }
    let path_obj = Path::new(path);
    if path_obj
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ResourceError::new("Resource path must be relative"));
    }

    let valid = match path_components.as_slice() {
        ["shelf.json"]
        | ["sources.json"]
        | ["settings.json"]
        | ["http-tts.json"]
        | ["bookmarks.json"]
        | ["reading-history.json"]
        | ["replacement-rules.json"]
        | ["discovery-favorites.json"] => true,
        ["progress", name] => filename_id(name, ".json").is_some(),
        ["search", name] => filename_id(name, ".json").is_some(),
        ["reading", name] => filename_id(name, ".json").is_some(),
        ["discovery", name] => filename_id(name, ".json").is_some(),
        ["dictionary", name] => filename_id(name, ".html").is_some(),
        ["media", id] => valid_id(id),
        ["books", book_id, "book.json"] => valid_id(book_id),
        ["books", book_id, "chapters", chapter_name] => {
            valid_id(book_id)
                && filename_id(chapter_name, ".json").is_some()
        }
        ["books", book_id, "assets", asset_id] => valid_id(book_id) && valid_asset_id(asset_id),
        ["books", "tts-audio", "media", file_name] => valid_tts_audio_file(file_name),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ResourceError::new(
            "Resource path is outside the public JSON/HTML resource set",
        ))
    }
}

pub(super) fn tts_audio_ref(audio_id: &str, extension: &str) -> Result<ResourceRef, ResourceError> {
    if audio_id.len() != 32
        || !audio_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !matches!(
            extension,
            "mp3" | "m4a" | "aac" | "wav" | "ogg" | "opus" | "flac"
        )
    {
        return Err(ResourceError::new(
            "Invalid temporary HTTP TTS audio identifier",
        ));
    }
    ResourceRef::new(format!(
        "resource://books/tts-audio/media/{audio_id}.{extension}"
    ))
}

pub(super) fn valid_tts_audio_file(file_name: &str) -> bool {
    let Some((audio_id, extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    audio_id.len() == 32
        && audio_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && matches!(
            extension,
            "mp3" | "m4a" | "aac" | "wav" | "ogg" | "opus" | "flac"
        )
}

fn filename_id<'a>(filename: &'a str, extension: &str) -> Option<&'a str> {
    let id = filename.strip_suffix(extension)?;
    valid_id(id).then_some(id)
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id != "."
        && id != ".."
        && !id.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

pub(super) fn validate_id(id: &str) -> Result<(), ResourceError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ResourceError::new("Invalid resource identifier"))
    }
}

pub(super) fn validate_asset_id(id: &str) -> Result<(), ResourceError> {
    if valid_asset_id(id) {
        Ok(())
    } else {
        Err(ResourceError::new("Invalid or unsupported asset filename"))
    }
}

fn valid_asset_id(id: &str) -> bool {
    let extension = id.rsplit_once('.').map(|(_, extension)| extension);
    valid_id(id)
        && extension.is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "css"
                    | "bmp"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "svg"
                    | "webp"
                    | "gif"
                    | "woff"
                    | "woff2"
                    | "ttf"
                    | "otf"
                    | "mp3"
                    | "m4a"
                    | "aac"
                    | "wav"
                    | "ogg"
                    | "opus"
                    | "mp4"
                    | "pdf"
                    | "webm"
            )
        })
}

pub(super) fn mime_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "xhtml" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "bmp" => "image/bmp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "pdf" => "application/pdf",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

pub(super) fn check_path_no_symlink(
    root: &Path,
    path: &Path,
    allow_missing_tail: bool,
) -> Result<(), ResourceError> {
    if !path.starts_with(root) {
        return Err(ResourceError::new("Resource path escaped its storage root"));
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ResourceError::new("Resource path escaped its storage root"))?;
    let mut current = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(ResourceError::new(
                "Resource path contains an invalid component",
            ));
        };
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ResourceError::new(
                    "Symbolic links are not served as resources",
                ));
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && allow_missing_tail
                    && components[index..].iter().all(|_| true) =>
            {
                // Once a parent is missing, subsequent paths cannot already be
                // symlinks. The atomic writer creates these directories below.
                break;
            }
            Err(error) => {
                return Err(ResourceError::new(format!(
                    "Cannot inspect resource path: {error}"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn percent_encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(hex_digit(byte >> 4)));
            encoded.push(char::from(hex_digit(byte & 0x0f)));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        10..=15 => b'A' + value - 10,
        _ => unreachable!("a hexadecimal nibble is at most 15"),
    }
}
