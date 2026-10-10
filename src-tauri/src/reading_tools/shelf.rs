//! 书架分组、批量归组与排序。

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    models::CURRENT_SCHEMA_VERSION,
    resources::{ResourceRef, ResourceStore},
};

use super::common::{ensure_object, validate_id};

pub(crate) const MAX_BATCH_BOOK_IDS: usize = 256;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ShelfSortKey {
    UpdatedAt,
    Title,
    Author,
    LatestChapter,
    Progress,
    Custom,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ShelfSortOrder {
    Ascending,
    Descending,
}

/// Assign one book's shelf groups in the shared bookshelf resource.
pub async fn set_book_groups(
    store: &ResourceStore,
    book_id: &str,
    groups: Vec<String>,
) -> Result<ResourceRef, String> {
    validate_id(book_id, "bookId")?;
    let groups = normalize_groups(groups)?;
    store
        .update_json_ref(&store.shelf_ref(), move |mut shelf| {
            ensure_object(&mut shelf, "shelf")?;
            normalize_shelf_groups(&mut shelf)?;
            let books = shelf
                .get_mut("books")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| "Shelf JSON has no books array".to_owned())?;
            let book = books
                .iter_mut()
                .find(|book| book.get("id").and_then(Value::as_str) == Some(book_id))
                .ok_or_else(|| format!("Book '{book_id}' is not on the shelf"))?;
            book["groups"] = json!(groups);
            normalize_shelf_groups(&mut shelf)?;
            Ok(shelf)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(store.shelf_ref())
}

/// Replace the groups on several books in one atomic shelf JSON update.
/// Unselected book entries, shelf sort settings, book order, and existing empty
/// group registrations are preserved exactly.
pub async fn set_book_groups_batch(
    store: &ResourceStore,
    book_ids: Vec<String>,
    groups: Vec<String>,
) -> Result<ResourceRef, String> {
    let book_ids = normalize_batch_book_ids(book_ids)?;
    let groups = normalize_groups(groups)?;
    let selected: HashSet<String> = book_ids.iter().cloned().collect();

    store
        .update_json_ref(&store.shelf_ref(), move |mut shelf| {
            ensure_object(&mut shelf, "shelf")?;
            validate_shelf_schema_version(&shelf)?;
            let books = shelf
                .get_mut("books")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| "Shelf JSON has no books array".to_owned())?;
            let mut found = HashSet::with_capacity(selected.len());
            for book in books.iter_mut() {
                let Some(book_object) = book.as_object_mut() else {
                    continue;
                };
                let Some(book_id) = book_object.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if selected.contains(book_id) {
                    if !found.insert(book_id.to_owned()) {
                        return Err(format!("Shelf contains duplicate book ID '{book_id}'"));
                    }
                    book_object.insert("groups".to_owned(), json!(groups));
                }
            }
            if let Some(missing) = book_ids.iter().find(|book_id| !found.contains(*book_id)) {
                return Err(format!("Book '{missing}' is not on the shelf"));
            }

            if !shelf
                .as_object()
                .is_some_and(|object| object.contains_key("groups"))
            {
                shelf["groups"] = json!([]);
            }
            let registry = match shelf.get_mut("groups") {
                Some(Value::Array(registry)) => registry,
                Some(_) => return Err("Shelf groups must be a JSON array".to_owned()),
                None => unreachable!("missing shelf group registry was initialized"),
            };
            for existing in registry.iter() {
                let name = existing
                    .as_str()
                    .ok_or_else(|| "Shelf group names must be strings".to_owned())?;
                normalize_group_name(name)?;
            }
            for group in groups {
                if !registry
                    .iter()
                    .any(|existing| existing.as_str() == Some(group.as_str()))
                {
                    registry.push(json!(group));
                }
            }
            Ok(shelf)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(store.shelf_ref())
}

/// Validate a batch selection before any mutation and keep the first occurrence
/// of each ID so duplicate UI selections cannot repeat an operation.
pub(crate) fn normalize_batch_book_ids(book_ids: Vec<String>) -> Result<Vec<String>, String> {
    if book_ids.is_empty() {
        return Err("Select at least one book".to_owned());
    }
    if book_ids.len() > MAX_BATCH_BOOK_IDS {
        return Err(format!(
            "A batch can include at most {MAX_BATCH_BOOK_IDS} books"
        ));
    }
    let mut unique = Vec::with_capacity(book_ids.len());
    let mut seen = HashSet::with_capacity(book_ids.len());
    for book_id in book_ids {
        validate_id(&book_id, "bookId")?;
        if seen.insert(book_id.clone()) {
            unique.push(book_id);
        }
    }
    Ok(unique)
}

fn validate_shelf_schema_version(shelf: &Value) -> Result<(), String> {
    let version = shelf
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Shelf JSON schemaVersion must be an unsigned integer".to_owned())?;
    if version != u64::from(CURRENT_SCHEMA_VERSION) {
        return Err(format!("Unsupported shelf schema version {version}"));
    }
    Ok(())
}

/// Create or retain a shelf group even while it has no books.
pub async fn create_shelf_group(
    store: &ResourceStore,
    group_name: &str,
) -> Result<ResourceRef, String> {
    let group_name = normalize_group_name(group_name)?;
    store
        .update_json_ref(&store.shelf_ref(), move |mut shelf| {
            ensure_object(&mut shelf, "shelf")?;
            normalize_shelf_groups(&mut shelf)?;
            let groups = shelf
                .get_mut("groups")
                .and_then(Value::as_array_mut)
                .expect("group registry normalized");
            if !groups
                .iter()
                .any(|existing| existing.as_str() == Some(group_name.as_str()))
            {
                groups.push(json!(group_name));
            }
            Ok(shelf)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(store.shelf_ref())
}

/// Rename one shelf group across every book in the shared shelf resource.
pub async fn rename_shelf_group(
    store: &ResourceStore,
    old_name: &str,
    new_name: &str,
) -> Result<ResourceRef, String> {
    let old_name = normalize_group_name(old_name)?;
    let new_name = normalize_group_name(new_name)?;
    store
        .update_json_ref(&store.shelf_ref(), move |mut shelf| {
            ensure_object(&mut shelf, "shelf")?;
            normalize_shelf_groups(&mut shelf)?;
            let books = shelf
                .get_mut("books")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| "Shelf JSON has no books array".to_owned())?;
            for book in books {
                let groups = book["groups"]
                    .as_array_mut()
                    .expect("group field normalized");
                for group in groups.iter_mut() {
                    if group.as_str() == Some(old_name.as_str()) {
                        *group = json!(new_name);
                    }
                }
                let mut seen = Vec::<String>::new();
                groups.retain(|group| {
                    let Some(name) = group.as_str() else {
                        return false;
                    };
                    if seen.iter().any(|existing| existing == name) {
                        false
                    } else {
                        seen.push(name.to_owned());
                        true
                    }
                });
            }
            let groups = shelf
                .get_mut("groups")
                .and_then(Value::as_array_mut)
                .expect("group registry normalized");
            let old_index = groups
                .iter()
                .position(|group| group.as_str() == Some(old_name.as_str()))
                .ok_or_else(|| format!("Shelf group '{old_name}' does not exist"))?;
            groups[old_index] = json!(new_name);
            normalize_shelf_groups(&mut shelf)?;
            Ok(shelf)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(store.shelf_ref())
}

/// Remove one shelf group from all books while preserving the books themselves.
pub async fn delete_shelf_group(
    store: &ResourceStore,
    group_name: &str,
) -> Result<ResourceRef, String> {
    let group_name = normalize_group_name(group_name)?;
    store
        .update_json_ref(&store.shelf_ref(), move |mut shelf| {
            ensure_object(&mut shelf, "shelf")?;
            normalize_shelf_groups(&mut shelf)?;
            let books = shelf
                .get_mut("books")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| "Shelf JSON has no books array".to_owned())?;
            for book in books {
                let groups = book["groups"]
                    .as_array_mut()
                    .expect("group field normalized");
                groups.retain(|group| group.as_str() != Some(group_name.as_str()));
            }
            shelf
                .get_mut("groups")
                .and_then(Value::as_array_mut)
                .expect("group registry normalized")
                .retain(|group| group.as_str() != Some(group_name.as_str()));
            normalize_shelf_groups(&mut shelf)?;
            Ok(shelf)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(store.shelf_ref())
}

/// Store the shelf's order and sort books in the shared shelf JSON document.
pub async fn set_shelf_sort(
    store: &ResourceStore,
    key: ShelfSortKey,
    order: ShelfSortOrder,
) -> Result<ResourceRef, String> {
    store
        .update_json_ref(&store.shelf_ref(), move |mut shelf| {
            ensure_object(&mut shelf, "shelf")?;
            shelf["sort"] = serde_json::to_value(key).map_err(|error| error.to_string())?;
            shelf["sortOrder"] = serde_json::to_value(order).map_err(|error| error.to_string())?;
            apply_shelf_sort(&mut shelf)?;
            Ok(shelf)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(store.shelf_ref())
}

/// Apply the sort settings already stored on a shelf document. The application
/// service uses this after progress changes the metadata from which the order
/// is derived, while preserving explicit custom order.
pub(crate) fn apply_shelf_sort(shelf: &mut Value) -> Result<(), String> {
    ensure_object(shelf, "shelf")?;
    let key = shelf
        .get("sort")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("Invalid shelf sort key: {error}"))?
        .unwrap_or(ShelfSortKey::UpdatedAt);
    let order = shelf
        .get("sortOrder")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("Invalid shelf sort order: {error}"))?
        .unwrap_or(ShelfSortOrder::Descending);
    let books = shelf
        .get_mut("books")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Shelf JSON has no books array".to_owned())?;
    if key != ShelfSortKey::Custom {
        books.sort_by(|left, right| {
            let ordering = compare_books(left, right, key);
            match order {
                ShelfSortOrder::Ascending => ordering,
                ShelfSortOrder::Descending => ordering.reverse(),
            }
        });
    }
    Ok(())
}

/// Persist an explicit user order. The provided IDs must be a permutation of
/// the current shelf so books cannot accidentally disappear from the index.
pub async fn set_shelf_order(
    store: &ResourceStore,
    ordered_book_ids: Vec<String>,
) -> Result<ResourceRef, String> {
    store
        .update_json_ref(&store.shelf_ref(), move |mut shelf| {
            ensure_object(&mut shelf, "shelf")?;
            normalize_shelf_groups(&mut shelf)?;
            let books = shelf
                .get_mut("books")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| "Shelf JSON has no books array".to_owned())?;
            if books.len() != ordered_book_ids.len() {
                return Err("Custom shelf order must include every book exactly once".to_owned());
            }
            let mut by_id = BTreeMap::new();
            for book in std::mem::take(books) {
                let id = book
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Shelf book is missing its ID".to_owned())?;
                if by_id.insert(id.to_owned(), book).is_some() {
                    return Err("Shelf contains duplicate book IDs".to_owned());
                }
            }
            let mut reordered = Vec::with_capacity(ordered_book_ids.len());
            for id in ordered_book_ids {
                let book = by_id.remove(&id).ok_or_else(|| {
                    "Custom shelf order contains an unknown or duplicate book ID".to_owned()
                })?;
                reordered.push(book);
            }
            if !by_id.is_empty() {
                return Err("Custom shelf order omitted one or more books".to_owned());
            }
            *books = reordered;
            shelf["sort"] = json!("custom");
            shelf["sortOrder"] = json!("ascending");
            Ok(shelf)
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(store.shelf_ref())
}

fn normalize_groups(groups: Vec<String>) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for group in groups {
        let group = group.trim();
        let group = normalize_group_name(group)?;
        if !result.iter().any(|existing: &String| existing == &group) {
            result.push(group);
        }
    }
    if result.len() > 64 {
        return Err("A book cannot belong to more than 64 shelf groups".to_owned());
    }
    Ok(result)
}

fn normalize_shelf_groups(shelf: &mut Value) -> Result<(), String> {
    let group_values = match shelf.get_mut("groups") {
        None => {
            shelf["groups"] = json!([]);
            shelf
                .get_mut("groups")
                .and_then(Value::as_array_mut)
                .expect("new group registry is an array")
        }
        Some(Value::Array(groups)) => groups,
        Some(_) => return Err("Shelf groups must be a JSON array".to_owned()),
    };
    let mut registry = Vec::<String>::new();
    for group in group_values.iter() {
        let name = group
            .as_str()
            .ok_or_else(|| "Shelf group names must be strings".to_owned())?;
        let name = normalize_group_name(name)?;
        if !registry.iter().any(|existing| existing == &name) {
            registry.push(name);
        }
    }
    let books = shelf
        .get_mut("books")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Shelf JSON has no books array".to_owned())?;
    for book in books {
        let book = book
            .as_object_mut()
            .ok_or_else(|| "Shelf book must be a JSON object".to_owned())?;
        let book_groups = match book.get_mut("groups") {
            None => {
                book.insert("groups".to_owned(), json!([]));
                book.get_mut("groups")
                    .and_then(Value::as_array_mut)
                    .expect("new book group value is an array")
            }
            Some(Value::Array(groups)) => groups,
            Some(_) => return Err("Shelf book groups must be a JSON array".to_owned()),
        };
        let mut normalized_book_groups = Vec::<String>::new();
        for group in book_groups.iter() {
            let name = group
                .as_str()
                .ok_or_else(|| "Book group names must be strings".to_owned())?;
            let name = normalize_group_name(name)?;
            if !normalized_book_groups
                .iter()
                .any(|existing| existing == &name)
            {
                normalized_book_groups.push(name.clone());
            }
            if !registry.iter().any(|existing| existing == &name) {
                registry.push(name);
            }
        }
        *book_groups = normalized_book_groups
            .into_iter()
            .map(Value::String)
            .collect();
    }
    shelf["groups"] = json!(registry);
    Ok(())
}

fn normalize_group_name(group: &str) -> Result<String, String> {
    let group = group.trim();
    if group.is_empty() || group.len() > 128 {
        return Err("Shelf group names must contain 1 to 128 bytes".to_owned());
    }
    Ok(group.to_owned())
}

fn compare_books(left: &Value, right: &Value, key: ShelfSortKey) -> std::cmp::Ordering {
    match key {
        ShelfSortKey::UpdatedAt => progress_updated_at(left).cmp(&progress_updated_at(right)),
        ShelfSortKey::Title => text_field(left, "title").cmp(&text_field(right, "title")),
        ShelfSortKey::Author => text_field(left, "author").cmp(&text_field(right, "author")),
        ShelfSortKey::LatestChapter => {
            text_field(left, "latestChapter").cmp(&text_field(right, "latestChapter"))
        }
        ShelfSortKey::Progress => progress_chapter_index(left).cmp(&progress_chapter_index(right)),
        ShelfSortKey::Custom => std::cmp::Ordering::Equal,
    }
}

fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
}

fn progress_updated_at(value: &Value) -> u64 {
    value["progress"]["updatedAtMs"]
        .as_u64()
        .unwrap_or_default()
}

fn progress_chapter_index(value: &Value) -> u64 {
    value["progress"]["chapterIndex"]
        .as_u64()
        .unwrap_or_default()
}
