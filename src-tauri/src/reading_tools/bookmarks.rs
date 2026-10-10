//! 书签文档与书签增删改。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    models::CURRENT_SCHEMA_VERSION,
    resources::{ResourceRef, ResourceStore},
};

use super::common::{current_schema_version, ensure_document, ensure_object, now_ms, validate_id};

const BOOKMARKS_PATH: &str = "resource://bookmarks.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkDocument {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
}

impl Default for BookmarkDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            bookmarks: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub book_id: String,
    pub chapter_index: usize,
    pub offset: u64,
    #[serde(default)]
    pub note: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkInput {
    #[serde(default)]
    pub id: Option<String>,
    pub book_id: String,
    pub chapter_index: usize,
    pub offset: u64,
    #[serde(default)]
    pub note: String,
}

pub async fn bookmarks_resource(store: &ResourceStore) -> Result<ResourceRef, String> {
    ensure_document(store, bookmarks_ref(), BookmarkDocument::default()).await
}

/// Create or update a bookmark. Existing IDs retain their creation timestamp
/// and cannot be reassigned to a different book.
pub async fn upsert_bookmark(
    store: &ResourceStore,
    input: BookmarkInput,
) -> Result<ResourceRef, String> {
    validate_id(&input.book_id, "bookId")?;
    if input.note.len() > 16_384 {
        return Err("Bookmark note exceeds 16 KiB".to_owned());
    }

    let id = match input.id {
        Some(id) => {
            validate_id(&id, "bookmarkId")?;
            id
        }
        None => format!("bookmark-{}", uuid::Uuid::new_v4().simple()),
    };
    let updated_at_ms = now_ms();
    let mut created_at_ms = updated_at_ms;
    let document = store
        .update_json_ref(&bookmarks_ref(), move |mut value| {
            let mut document = bookmark_document(&mut value)?;
            let existing = document
                .get_mut("bookmarks")
                .and_then(Value::as_array_mut)
                .expect("default bookmark document has an array");
            if let Some(position) = existing.iter().position(|bookmark| {
                bookmark.get("id").and_then(Value::as_str) == Some(id.as_str())
            }) {
                let old = &existing[position];
                if old.get("bookId").and_then(Value::as_str) != Some(&input.book_id) {
                    return Err("Bookmark ID already belongs to another book".to_owned());
                }
                created_at_ms = old
                    .get("createdAtMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(updated_at_ms);
                existing[position] = json!({
                    "id": id,
                    "bookId": input.book_id,
                    "chapterIndex": input.chapter_index,
                    "offset": input.offset,
                    "note": input.note,
                    "createdAtMs": created_at_ms,
                    "updatedAtMs": updated_at_ms,
                });
            } else {
                existing.push(json!({
                    "id": id,
                    "bookId": input.book_id,
                    "chapterIndex": input.chapter_index,
                    "offset": input.offset,
                    "note": input.note,
                    "createdAtMs": created_at_ms,
                    "updatedAtMs": updated_at_ms,
                }));
            }
            document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
            Ok(document)
        })
        .await
        .map_err(|error| error.to_string())?;
    let _ = document;
    Ok(bookmarks_ref())
}

pub async fn delete_bookmark(store: &ResourceStore, id: &str) -> Result<ResourceRef, String> {
    validate_id(id, "bookmarkId")?;
    store
        .update_json_ref(&bookmarks_ref(), |mut value| {
            let mut document = bookmark_document(&mut value)?;
            let bookmarks = document
                .get_mut("bookmarks")
                .and_then(Value::as_array_mut)
                .expect("default bookmark document has an array");
            bookmarks.retain(|bookmark| bookmark.get("id").and_then(Value::as_str) != Some(id));
            document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
            Ok(document)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(bookmarks_ref())
}

/// Remove bookmarks whose chapter locations disappear with a removed book.
/// Reading-history sessions are intentionally stored separately and retained.
pub async fn remove_bookmarks_for_book(
    store: &ResourceStore,
    book_id: &str,
) -> Result<ResourceRef, String> {
    validate_id(book_id, "bookId")?;
    store
        .update_json_ref(&bookmarks_ref(), move |mut value| {
            let mut document = bookmark_document(&mut value)?;
            document
                .get_mut("bookmarks")
                .and_then(Value::as_array_mut)
                .expect("default bookmark document has an array")
                .retain(|bookmark| bookmark.get("bookId").and_then(Value::as_str) != Some(book_id));
            document["schemaVersion"] = json!(CURRENT_SCHEMA_VERSION);
            Ok(document)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(bookmarks_ref())
}

fn bookmark_document(value: &mut Value) -> Result<Value, String> {
    ensure_object(value, "bookmark")?;
    value
        .as_object_mut()
        .expect("object checked")
        .entry("schemaVersion")
        .or_insert_with(|| json!(CURRENT_SCHEMA_VERSION));
    value
        .as_object_mut()
        .expect("object checked")
        .entry("bookmarks")
        .or_insert_with(|| json!([]));
    if !value["bookmarks"].is_array() {
        return Err("Bookmark JSON field 'bookmarks' must be an array".to_owned());
    }
    Ok(value.clone())
}

fn bookmarks_ref() -> ResourceRef {
    ResourceRef::new(BOOKMARKS_PATH).expect("static bookmark resource reference is valid")
}
