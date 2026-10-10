//! 校验并保存书名、作者、封面等展示信息。
//! Processed book display defaults and user-authored display overrides.
//!
//! This resource deliberately contains only the small public fields used by
//! the shelf and book details. KMP objects, source definitions, and canonical
//! book identity stay in the private engine record.

use serde_json::{Value, json};

use crate::{
    models::{BookDisplayBase, BookDisplayOverrides},
    resources::ResourceRef,
};

const MAX_TITLE_CHARS: usize = 512;
const MAX_AUTHOR_CHARS: usize = 512;
const MAX_INTRO_CHARS: usize = 12_000;
const MAX_COVER_URL_CHARS: usize = 4_096;

#[derive(Clone, Debug)]
struct BookDisplayState {
    base: BookDisplayBase,
    overrides: BookDisplayOverrides,
}

/// Create processed base metadata from the fields already projected by Rust.
/// Used once for new online and local books.
pub(crate) fn initial_base(
    title: &str,
    author: &str,
    intro: Option<&str>,
    cover_src: Option<&str>,
) -> Result<BookDisplayBase, String> {
    let title = sanitize_text(title, MAX_TITLE_CHARS);
    if title.is_empty() {
        return Err("Book display title cannot be empty".to_owned());
    }
    let author = sanitize_text(author, MAX_AUTHOR_CHARS);
    let intro = intro.map(|value| sanitize_text(value, MAX_INTRO_CHARS));
    let cover_src = cover_src.and_then(normalize_base_cover);
    Ok(BookDisplayBase {
        title,
        author,
        intro,
        cover_src,
    })
}

/// Initialize legacy book JSON from its current processed fields, merge new
/// processed defaults, and apply existing user overrides to its top-level
/// display values. Missing optional fields in a partial KMP response retain
/// their last known base value.
pub(crate) fn update_base(
    book: &mut Value,
    metadata: &crate::book_metadata::ProcessedBookMetadata,
) -> Result<(), String> {
    let mut state = read_or_initialize_state(book)?;
    if metadata.title != "Untitled" {
        let title = metadata.title.clone();
        if !title.is_empty() && title != "Untitled" {
            state.base.title = title;
        }
    }
    if let Some(author) = metadata.author.as_deref() {
        state.base.author = author.to_owned();
    }
    if let Some(intro) = metadata.intro.as_deref() {
        state.base.intro = Some(intro.to_owned());
    }
    if let Some(cover) = metadata.cover_src.as_ref() {
        state.base.cover_src = normalize_base_cover(cover.as_str());
    }
    apply_state(book, &state)
}

/// Apply a user-authored sparse patch to the override map. This helper never
/// reads engine-supplied display fields or arbitrary book fields as
/// an override source.
pub(crate) fn apply_patch(book: &mut Value, patch: &Value) -> Result<(), String> {
    let fields = patch
        .as_object()
        .ok_or_else(|| "Book display patch must be an object".to_owned())?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "title" | "author" | "intro" | "coverSrc"))
    {
        return Err("Book display patch contains an unsupported field".to_owned());
    }
    let mut state = read_or_initialize_state(book)?;
    apply_text_patch(
        &mut state.overrides.title,
        fields.get("title"),
        "title",
        MAX_TITLE_CHARS,
        false,
    )?;
    apply_text_patch(
        &mut state.overrides.author,
        fields.get("author"),
        "author",
        MAX_AUTHOR_CHARS,
        true,
    )?;
    apply_text_patch(
        &mut state.overrides.intro,
        fields.get("intro"),
        "intro",
        MAX_INTRO_CHARS,
        true,
    )?;
    match fields.get("coverSrc") {
        None => {}
        Some(Value::Null) => state.overrides.cover_src = None,
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.len() > MAX_COVER_URL_CHARS {
                return Err("Cover URL exceeds 4096 bytes".to_owned());
            }
            state.overrides.cover_src = Some(if value.is_empty() {
                String::new()
            } else {
                normalize_user_cover(value)?
            });
        }
        Some(_) => return Err("Book display coverSrc must be a string or null".to_owned()),
    }
    apply_state(book, &state)
}

fn apply_state(book: &mut Value, state: &BookDisplayState) -> Result<(), String> {
    book["title"] = json!(
        state
            .overrides
            .title
            .as_deref()
            .unwrap_or(&state.base.title)
    );
    book["author"] = json!(
        state
            .overrides
            .author
            .as_deref()
            .unwrap_or(&state.base.author)
    );
    let intro = state
        .overrides
        .intro
        .as_ref()
        .map(String::as_str)
        .or(state.base.intro.as_deref());
    set_optional_string(book, "intro", intro);
    let cover = state
        .overrides
        .cover_src
        .as_ref()
        .map(String::as_str)
        .or(state.base.cover_src.as_deref());
    set_optional_string(book, "coverSrc", cover.filter(|value| !value.is_empty()));
    write_state(book, &state)
}

fn apply_text_patch(
    target: &mut Option<String>,
    patch: Option<&Value>,
    field: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), String> {
    match patch {
        None => {}
        Some(Value::Null) => *target = None,
        Some(Value::String(value)) => {
            if value.chars().count() > max_chars {
                return Err(format!(
                    "Book display {field} exceeds {max_chars} characters"
                ));
            }
            let value = sanitize_text(value, max_chars);
            if value.is_empty() && !allow_empty {
                return Err(format!("Book display {field} cannot be empty"));
            }
            *target = Some(value);
        }
        Some(_) => return Err(format!("Book display {field} must be a string or null")),
    }
    Ok(())
}

fn read_or_initialize_state(book: &mut Value) -> Result<BookDisplayState, String> {
    if !book.is_object() {
        return Err("Book display metadata requires a book object".to_owned());
    }
    let base = match book.get("displayBase") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| format!("Book display base is invalid: {error}"))?,
        None => initial_base(
            book.get("title")
                .and_then(Value::as_str)
                .unwrap_or("Untitled"),
            book.get("author")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            book.get("intro").and_then(Value::as_str),
            book.get("coverSrc").and_then(Value::as_str),
        )?,
    };
    let overrides = match book.get("displayOverrides") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| format!("Book display overrides are invalid: {error}"))?,
        None => BookDisplayOverrides::default(),
    };
    Ok(BookDisplayState { base, overrides })
}

fn write_state(book: &mut Value, state: &BookDisplayState) -> Result<(), String> {
    book["displayBase"] = serde_json::to_value(&state.base)
        .map_err(|error| format!("Cannot encode book display base: {error}"))?;
    if state.overrides.title.is_some()
        || state.overrides.author.is_some()
        || state.overrides.intro.is_some()
        || state.overrides.cover_src.is_some()
    {
        book["displayOverrides"] = serde_json::to_value(&state.overrides)
            .map_err(|error| format!("Cannot encode book display overrides: {error}"))?;
    } else if let Some(fields) = book.as_object_mut() {
        fields.remove("displayOverrides");
    }
    Ok(())
}

fn set_optional_string(book: &mut Value, key: &str, value: Option<&str>) {
    if let Some(fields) = book.as_object_mut() {
        if let Some(value) = value {
            fields.insert(key.to_owned(), json!(value));
        } else {
            fields.remove(key);
        }
    }
}

fn normalize_base_cover(value: &str) -> Option<String> {
    normalize_cover(value, true).ok().flatten()
}

fn normalize_user_cover(value: &str) -> Result<String, String> {
    normalize_cover(value, false)?.ok_or_else(|| "Cover URL cannot be empty".to_owned())
}

fn normalize_cover(value: &str, allow_external_http: bool) -> Result<Option<String>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_COVER_URL_CHARS {
        return Err("Cover URL exceeds 4096 bytes".to_owned());
    }
    if value.starts_with("resource://") {
        let reference = ResourceRef::new(value).map_err(|error| error.to_string())?;
        return Ok(Some(reference.to_string()));
    }
    let url = reqwest::Url::parse(value).map_err(|_| "Cover URL is invalid".to_owned())?;
    let scheme_allowed = url.scheme() == "https" || (allow_external_http && url.scheme() == "http");
    if !scheme_allowed
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || is_runtime_host(url.host_str().unwrap_or_default())
    {
        return Err("Cover must be a stable resource reference or a safe HTTPS URL".to_owned());
    }
    Ok(Some(url.to_string()))
}

fn is_runtime_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn sanitize_text(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    let mut output_chars = 0;
    for character in value.chars() {
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            continue;
        }
        if output_chars >= max_chars {
            break;
        }
        output.push(character);
        output_chars += 1;
    }
    output.trim().to_owned()
}
