//! Content identities, engine boundary conversion and file descriptors.
use super::{ResourceError, ResourceRef};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub(crate) const CONTENT_TYPES: [&str; 6] = ["text", "html", "image", "audio", "video", "pdf"];
pub(crate) const LIBRARY_FILE: &str = "content-library.json";

pub(crate) fn canonical_type(kind: &str) -> &str {
    match kind {
        "comic" => "image",
        "richText" => "html",
        kind => kind,
    }
}

pub(crate) fn content_path(path: &str) -> Option<(&str, &str, &str)> {
    let parts = path.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["cache", kind, id, name] if CONTENT_TYPES.contains(kind) => Some((kind, id, name)),
        _ => None,
    }
}

pub(crate) fn is_content_document(path: &str) -> bool {
    content_path(path).is_some_and(|(_, id, name)| name == format!("{id}.json"))
}

pub(crate) fn find_content_dir(root: &Path, id: &str) -> Result<Option<PathBuf>, ResourceError> {
    super::validation::validate_id(id)?;
    Ok(crate::database::Database::at(root).content_path(id)?.and_then(|path| root.join(path).parent().map(Path::to_path_buf)))
}

fn object(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}
fn copy(from: &Value, to: &mut Map<String, Value>, source: &str, target: &str) {
    if let Some(value) = from.get(source).filter(|value| !value.is_null()) {
        to.insert(target.to_owned(), value.clone());
    }
}

pub(crate) fn local_ref(kind: &str, id: &str, value: &str) -> Result<ResourceRef, ResourceError> {
    if value.starts_with("resource://")
        || value.starts_with("https://")
        || value.starts_with("http://")
    {
        ResourceRef::new(value)
    } else {
        super::validation::validate_asset_id(value)?;
        ResourceRef::new(format!("resource://cache/{kind}/{id}/{value}"))
    }
}

pub(crate) fn stored_url(value: &str, id: &str) -> String {
    if let Some(path) = value.strip_prefix("resource://") {
        if let Some((_, owner, name)) = content_path(path) {
            if owner == id {
                return name.to_owned();
            }
        }
    }
    value.to_owned()
}

/// Reconstruct the source executor's context from the same complete document.
pub(crate) fn source_state(document: &Value) -> Value {
    let mut state = object(&document["engineState"]);
    for key in [
        "sourceId",
        "bookInstanceId",
        "catalogGeneration",
        "catalogVersion",
        "catalogFingerprint",
    ] {
        copy(document, &mut state, key, key);
    }
    state.insert(
        "book".into(),
        document.get("engine").cloned().unwrap_or(Value::Null),
    );
    let chapters = document["chapters"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, chapter)| {
            let mut raw = object(&chapter["engineData"]);
            raw.insert("index".into(), json!(index));
            if let Some(url) = chapter["engineurl"].as_str() {
                raw.insert("url".into(), json!(url));
            }
            raw.insert(
                "title".into(),
                chapter
                    .get("engineTitle")
                    .unwrap_or(&chapter["title"])
                    .clone(),
            );
            if chapter["volume"] == true {
                raw.insert("isVolume".into(), json!(true));
            }
            Value::Object(raw)
        })
        .collect::<Vec<_>>();
    state.insert("chapters".into(), json!(chapters));
    Value::Object(state)
}

/// The application model retains descriptors; only the canonical disk form is persisted.
pub(crate) fn book_view(document: &Value, kind: &str) -> Result<Value, ResourceError> {
    let id = document["id"]
        .as_str()
        .ok_or_else(|| ResourceError::new("Content document has no ID"))?;
    let mut view = document.clone();
    let values = view
        .as_object_mut()
        .ok_or_else(|| ResourceError::new("Content document must be an object"))?;
    values.insert("schemaVersion".into(), json!(1));
    values.insert("contentType".into(), json!(kind));
    if matches!(kind, "audio" | "video") {
        values.insert("mediaType".into(), json!(kind));
    }
    values.insert(
        "canChangeSource".into(),
        json!(document["sourceId"].is_string() && document["engine"].is_object()),
    );
    if let Some(cover) = document["cover"].as_str().filter(|url| !url.is_empty()) {
        values.insert(
            "coverSrc".into(),
            json!(local_ref(kind, id, cover)?.as_str()),
        );
    }
    if let Some(overrides) = document.get("overrides") {
        values.insert("displayOverrides".into(), overrides.clone());
    }
    if let Some(tags) = document["tags"].as_array() {
        values.insert(
            "kind".into(),
            json!(tags
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")),
        );
    }
    let base = json!({"title": document["title"], "author": document["author"].as_str().unwrap_or_default(), "intro": document["intro"],
        "coverSrc": values.get("coverSrc")});
    values.insert("displayBase".into(), base);
    if let Some(overrides) = document["overrides"].as_object() {
        for key in ["title", "author", "intro", "coverSrc"] {
            if let Some(value) = overrides.get(key) {
                values.insert(key.into(), value.clone());
            }
        }
    }
    let mut chapters = Vec::new();
    if kind == "pdf" {
        if let Some(url) = document["url"].as_str() {
            let pdf = local_ref(kind, id, url)?;
            for index in 0..document["pageCount"].as_u64().unwrap_or(0) as usize {
                chapters.push(json!({"id": format!("page-{}", index + 1), "title": format!("{}", index + 1),
                    "index": index, "resource": {"resourceId": pdf.as_str(), "src": pdf.as_str(),
                        "contentType": "application/pdf", "format": "pdf", "contentFormat": "pdf"},
                    "pdfPage": {"schemaVersion": 1, "kind": "pdfPage", "src": pdf.as_str(), "pageIndex": index, "defaultZoom": "page-fit"}}));
            }
        }
    } else {
        for (index, chapter) in document["chapters"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let mut next = chapter.clone();
            next["index"] = json!(index);
            let reference = chapter["url"]
                .as_str()
                .map(|url| local_ref(kind, id, url))
                .transpose()?;
            if let Some(reference) = reference {
                let format = if kind == "html" { "richText" } else { kind };
                next["resource"] = json!({"resourceId": reference.as_str(), "src": reference.as_str(),
                    "contentType": super::validation::mime_type(reference.path()),
                    "format": if kind == "html" { "html" } else { kind }, "contentFormat": format});
            }
            if kind == "image" && chapter["pages"].is_array() {
                next["pages"] = json!(chapter["pages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|page| {
                        let mut page = page.clone();
                        if let Some(url) = page["url"].as_str() {
                            page["src"] = json!(local_ref(kind, id, url)
                                .map(|r| r.as_str().to_owned())
                                .unwrap_or_default());
                        }
                        page
                    })
                    .collect::<Vec<_>>());
            }
            if matches!(kind, "audio" | "video") {
                let selected = if document["progress"]["chapterId"] == chapter["id"] {
                    document["progress"]["quality"]
                        .as_str()
                        .and_then(|quality| {
                            chapter["sources"].as_array().and_then(|sources| {
                                sources.iter().find(|source| {
                                    source["label"] == quality && source["url"].is_string()
                                })
                            })
                        })
                } else {
                    None
                };
                let selected_url = selected
                    .and_then(|source| source["url"].as_str())
                    .or_else(|| chapter["url"].as_str());
                let selected_protocol = selected
                    .and_then(|source| source["protocol"].as_str())
                    .or_else(|| chapter["protocol"].as_str())
                    .unwrap_or("direct");
                let src = selected_url
                    .map(|url| local_ref(kind, id, url))
                    .transpose()?;
                if let Some(src) = src {
                    next["media"] = json!({"schemaVersion": 1, "kind": "media", "mediaType": kind, "src": src.as_str(),
                        "format": selected_protocol,
                        "sources": chapter["sources"].as_array().into_iter().flatten().map(|source| {
                            let mut source = source.clone();
                            source["selected"] = json!(false);
                            if let Some(url) = source["url"].as_str().map(str::to_owned) {
                                source["src"] = json!(local_ref(kind, id, &url).map(|r| r.as_str().to_owned()).unwrap_or_default());
                                source["selected"] = json!(Some(url.as_str()) == selected_url);
                            }
                            if let Some(protocol) = source.get("protocol").cloned() { source["format"] = protocol; }
                            source
                        }).collect::<Vec<_>>()});
                }
            }
            chapters.push(next);
        }
    }
    view["chapterCount"] = json!(chapters.len());
    view["latestChapter"] = chapters
        .iter()
        .rev()
        .find(|chapter| chapter["volume"] != true)
        .map(|c| c["title"].clone())
        .unwrap_or(Value::Null);
    view["chapters"] = json!(chapters);
    let progress = &document["progress"];
    let index = if kind == "pdf" {
        progress["page"].as_u64().unwrap_or(1).saturating_sub(1) as usize
    } else {
        view["chapters"]
            .as_array()
            .into_iter()
            .flatten()
            .position(|c| c["id"] == progress["chapterId"])
            .unwrap_or(0)
    };
    let offset = match kind {
        "audio" | "video" => progress["timeMs"].as_u64().unwrap_or(0),
        "image" => progress["offsetRatio"]
            .as_f64()
            .map(|value| (value * 1_000_000.0) as u64)
            .unwrap_or(0),
        _ => progress["charOffset"].as_u64().unwrap_or(0),
    };
    view["progress"] = json!({"chapterId": progress.get("chapterId"), "chapterIndex": index,
        "pageId": progress.get("pageId"), "anchor": progress.get("anchor"), "quality": progress.get("quality"), "offset": offset, "updatedAtMs": progress["updatedAtMs"].as_u64().unwrap_or(0)});
    Ok(view)
}

pub(crate) fn encode_book(
    view: &Value,
    state: Option<&Value>,
    existing: &Value,
    kind: &str,
) -> Result<Value, ResourceError> {
    let id = view["id"]
        .as_str()
        .ok_or_else(|| ResourceError::new("Content has no ID"))?;
    let mut document = object(existing);
    for key in [
        "id",
        "title",
        "author",
        "intro",
        "sourceId",
        "sourceName",
        "sourceGroup",
        "wordCount",
        "status",
        "original",
        "pageCount",
        "updatedAtMs",
        "alternatives",
    ] {
        copy(view, &mut document, key, key);
    }
    if let Some(base) = view["displayBase"].as_object() {
        for key in ["title", "author", "intro"] {
            if let Some(value) = base.get(key) {
                document.insert(key.into(), value.clone());
            }
        }
    }
    copy(view, &mut document, "displayOverrides", "overrides");
    let cover = view["displayBase"]["coverSrc"]
        .as_str()
        .or_else(|| view["coverSrc"].as_str());
    if let Some(cover) = cover {
        document.insert("cover".into(), json!(stored_url(cover, id)));
    }
    if let Some(tags) = view.get("tags") {
        document.insert("tags".into(), tags.clone());
    } else if let Some(tag) = view["kind"].as_str().filter(|tag| !tag.is_empty()) {
        document.insert("tags".into(), json!([tag]));
    }
    let state = state.cloned().unwrap_or_else(|| source_state(existing));
    if state["book"].is_object() {
        document.insert("engine".into(), state["book"].clone());
    }
    for key in [
        "sourceId",
        "bookInstanceId",
        "catalogGeneration",
        "catalogVersion",
        "catalogFingerprint",
    ] {
        copy(&state, &mut document, key, key);
    }
    let extras = state
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "book"
                    | "chapters"
                    | "sourceId"
                    | "bookInstanceId"
                    | "catalogGeneration"
                    | "catalogVersion"
                    | "catalogFingerprint"
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<String, Value>>();
    if extras.is_empty() {
        document.remove("engineState");
    } else {
        document.insert("engineState".into(), Value::Object(extras));
    }
    let raw_chapters = state["chapters"].as_array();
    if kind != "pdf" {
        let mut chapters = Vec::new();
        for (index, chapter) in view["chapters"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let mut next = Map::new();
            for key in [
                "id",
                "title",
                "volume",
                "engineurl",
                "pages",
                "sources",
                "files",
                "durationMs",
                "protocol",
            ] {
                copy(chapter, &mut next, key, key);
            }
            if let Some(raw) = raw_chapters.and_then(|chapters| chapters.get(index)) {
                let mut raw = object(raw);
                if let Some(url) = raw.remove("url").or_else(|| raw.remove("chapterUrl")) {
                    next.insert("engineurl".into(), url);
                }
                if let Some(title) = raw.remove("title") {
                    if title != chapter["title"] {
                        next.insert("engineTitle".into(), title);
                    }
                }
                raw.remove("index");
                if raw.get("isVolume").is_some_and(|value| value == true) {
                    raw.remove("isVolume");
                    next.insert("volume".into(), json!(true));
                }
                if !raw.is_empty() {
                    next.insert("engineData".into(), Value::Object(raw));
                }
            } else {
                copy(chapter, &mut next, "engineData", "engineData");
                copy(chapter, &mut next, "engineTitle", "engineTitle");
            }
            let url = chapter["resource"]["resourceId"]
                .as_str()
                .or_else(|| chapter["url"].as_str());
            if let Some(url) = url {
                let mut url = stored_url(url, id);
                if kind == "html" && url.ends_with(".txt") && !url.starts_with("original.") {
                    url = format!("{}.html", url.trim_end_matches(".txt"));
                }
                next.insert("url".into(), json!(url));
            }
            for key in ["pages", "sources"] {
                if let Some(records) = next.get_mut(key).and_then(Value::as_array_mut) {
                    for record in records {
                        if let Some(values) = record.as_object_mut() {
                            if let Some(src) = values
                                .remove("src")
                                .and_then(|v| v.as_str().map(str::to_owned))
                            {
                                values.insert("url".into(), json!(stored_url(&src, id)));
                            } else if let Some(url) =
                                values.get("url").and_then(Value::as_str).map(str::to_owned)
                            {
                                values.insert("url".into(), json!(stored_url(&url, id)));
                            }
                            if let Some(format) = values.remove("format") {
                                values.insert("protocol".into(), format);
                            }
                            values.remove("selected");
                        }
                    }
                }
            }
            chapters.push(Value::Object(next));
        }
        document.insert("chapters".into(), json!(chapters));
    } else {
        if let Some(url) = view["url"]
            .as_str()
            .or_else(|| view["chapters"][0]["pdfPage"]["src"].as_str())
            .or_else(|| view["chapters"][0]["resource"]["resourceId"].as_str())
        {
            document.insert("url".into(), json!(stored_url(url, id)));
        }
        document
            .entry("pageCount")
            .or_insert_with(|| json!(view["chapters"].as_array().map(Vec::len).unwrap_or(0)));
    }
    let progress = &view["progress"];
    if progress.is_object() && progress["updatedAtMs"].as_u64().unwrap_or(0) > 0 {
        let mut next = Map::new();
        copy(progress, &mut next, "chapterId", "chapterId");
        copy(progress, &mut next, "pageId", "pageId");
        copy(progress, &mut next, "anchor", "anchor");
        copy(progress, &mut next, "quality", "quality");
        copy(progress, &mut next, "updatedAtMs", "updatedAtMs");
        let offset = progress["offset"].as_u64().unwrap_or(0);
        match kind {
            "audio" | "video" => {
                next.insert("timeMs".into(), json!(offset));
            }
            "image" => {
                next.insert("offsetRatio".into(), json!(offset as f64 / 1_000_000.0));
            }
            "pdf" => {
                next.remove("chapterId");
                next.insert(
                    "page".into(),
                    json!(progress["chapterIndex"].as_u64().unwrap_or(0) + 1),
                );
            }
            _ => {
                next.insert("charOffset".into(), json!(offset));
            }
        }
        document.insert("progress".into(), Value::Object(next));
    }
    if progress.is_object() && progress["updatedAtMs"].as_u64().unwrap_or(0) == 0 {
        document.remove("progress");
    }
    for key in [
        "schemaVersion",
        "contentType",
        "mediaType",
        "chapterCount",
        "latestChapter",
        "sourceBookIds",
        "displayBase",
        "displayOverrides",
    ] {
        document.remove(key);
    }
    Ok(Value::Object(document))
}

#[cfg(test)]
pub(crate) fn public_book_view(document: &Value, kind: &str) -> Result<Value, ResourceError> {
    let mut view = book_view(document, kind)?;
    if let Some(values) = view.as_object_mut() {
        values.remove("media");
    }
    strip_private(&mut view);
    Ok(view)
}

#[cfg(test)]
pub(crate) fn strip_private(value: &mut Value) {
    match value {
        Value::Object(values) => {
            for key in [
                "engine",
                "engineState",
                "engineurl",
                "engineData",
                "engineTitle",
                "headers",
                "sourceRevision",
                "sourceDefinitionFingerprint",
                "bookInstanceId",
                "catalogGeneration",
                "catalogVersion",
                "catalogFingerprint",
            ] {
                values.remove(key);
            }
            for value in values.values_mut() {
                strip_private(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                strip_private(value);
            }
        }
        _ => {}
    }
}

pub(crate) fn library_positions(library: &Value) -> Value {
    let groups = library["groups"].as_array().cloned().unwrap_or_default();
    let mut books = Vec::new();
    for kind in CONTENT_TYPES {
        for item in library[kind].as_array().into_iter().flatten() {
            let names = groups
                .iter()
                .filter(|g| {
                    g["id"]
                        .as_array()
                        .is_some_and(|ids| ids.contains(&item["id"]))
                })
                .filter_map(|g| g["name"].as_str())
                .collect::<Vec<_>>();
            books.push(json!({"id": item["id"], "groups": names}));
        }
    }
    json!({"schemaVersion": 1, "books": books,
        "groups": groups.iter().filter_map(|g| g["name"].as_str()).collect::<Vec<_>>(),
        "sort": "custom", "sortOrder": "ascending"})
}

pub(crate) fn validate_library(library: &Value) -> Result<(), ResourceError> {
    let values = library
        .as_object()
        .ok_or_else(|| ResourceError::new("Content library must be an object"))?;
    let mut ids = std::collections::HashSet::new();
    for (kind, items) in values {
        if kind == "groups" {
            continue;
        }
        if !CONTENT_TYPES.contains(&kind.as_str()) {
            return Err(ResourceError::new("Unknown content library field"));
        }
        let items = items
            .as_array()
            .ok_or_else(|| ResourceError::new("Content type must be an array"))?;
        for item in items {
            let id = item["id"]
                .as_str()
                .ok_or_else(|| ResourceError::new("Library entry has no ID"))?;
            super::validation::validate_id(id)?;
            if !item["title"].is_string() || !ids.insert(id) {
                return Err(ResourceError::new("Invalid or duplicate library entry"));
            }
        }
    }
    if let Some(groups) = library.get("groups") {
        let groups = groups
            .as_array()
            .ok_or_else(|| ResourceError::new("Groups must be an array"))?;
        let mut names = std::collections::HashSet::new();
        for group in groups {
            let name = group["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| ResourceError::new("Group has no name"))?;
            if !names.insert(name) {
                return Err(ResourceError::new("Duplicate group name"));
            }
            let members = group["id"]
                .as_array()
                .ok_or_else(|| ResourceError::new("Group IDs must be an array"))?;
            let mut seen = std::collections::HashSet::new();
            for id in members {
                let id = id
                    .as_str()
                    .ok_or_else(|| ResourceError::new("Invalid group member"))?;
                if !ids.contains(id) || !seen.insert(id) {
                    return Err(ResourceError::new("Group member is missing or duplicated"));
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn encode_library(input: &Value, connection: &rusqlite::Connection) -> Result<Value, ResourceError> {
    let normalized;
    let positions = if input.get("books").is_none() {
        validate_library(input)?;
        normalized = library_positions(input);
        &normalized
    } else {
        input
    };
    let mut library = Map::new();
    let mut groups = positions["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| json!({"name": name, "id": []}))
        .collect::<Vec<_>>();
    for position in positions["books"].as_array().into_iter().flatten() {
        let id = position["id"]
            .as_str()
            .ok_or_else(|| ResourceError::new("Library entry has no ID"))?;
        let (kind, item) = crate::database::book_preview(connection,id)?;
        library.entry(kind).or_insert_with(||json!([])).as_array_mut().expect("array").push(item);
        for name in position["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !groups.iter().any(|group| group["name"] == name) {
                groups.push(json!({"name": name, "id": []}));
            }
            if let Some(group) = groups.iter_mut().find(|group| group["name"] == name) {
                group["id"].as_array_mut().expect("array").push(json!(id));
            }
        }
    }
    if input.get("books").is_none() {
        groups = input["groups"].as_array().cloned().unwrap_or_default();
    }
    if !groups.is_empty() {
        library.insert("groups".into(), json!(groups));
    }
    Ok(Value::Object(library))
}
