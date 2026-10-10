//! 约束书籍、章节与阅读进度变更的提交边界。
//! Pure JSON transformations used to build a source-replacement commit.
//!
//! These helpers do not read or write resources. Callers provide snapshots
//! captured while holding the ResourceStore writer guard, then persist the
//! returned JSON as one file transaction.

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BookmarkMigrationCounts {
    pub(crate) migrated: usize,
    pub(crate) orphaned: usize,
}

/// Update one shelf entry while retaining its groups and the shelf's current
/// sort configuration. `apply_shelf_sort` preserves custom order and reapplies
/// the selected derived order for all other sort modes.
pub(crate) fn upsert_shelf_entry(
    shelf: &mut Value,
    book_id: &str,
    book: &Value,
) -> Result<(), String> {
    if book.get("id").and_then(Value::as_str) != Some(book_id) {
        return Err("Book ID does not match its shelf entry".to_owned());
    }
    let mut entry = json!({
        "id": book["id"],
        "title": book["title"],
        "author": book["author"],
        "kind": book["kind"],
        "mediaType": book["mediaType"],
        "coverSrc": book["coverSrc"],
        "chapterCount": book["chapterCount"],
        "latestChapter": book["latestChapter"],
        "progress": book["progress"],
        "groups": [],
    });
    {
        let books = shelf
            .get_mut("books")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "Invalid shelf JSON: books must be an array".to_owned())?;
        if let Some(existing) = books
            .iter_mut()
            .find(|existing| existing.get("id").and_then(Value::as_str) == Some(book_id))
        {
            if let Some(groups) = existing.get("groups").cloned() {
                entry["groups"] = groups;
            }
            *existing = entry;
        } else {
            books.push(entry);
        }
    }
    crate::reading_tools::apply_shelf_sort(shelf)
}

/// Remove every shelf entry for one book, preserving shelf groups and sort
/// configuration for the remaining books.
pub(crate) fn remove_shelf_entry(shelf: &mut Value, book_id: &str) -> Result<usize, String> {
    let books = shelf
        .get_mut("books")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Invalid shelf JSON: books must be an array".to_owned())?;
    let previous_len = books.len();
    books.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(book_id));
    let removed = previous_len.saturating_sub(books.len());
    crate::reading_tools::apply_shelf_sort(shelf)?;
    Ok(removed)
}

/// Remove one book's bookmarks from the captured document. Reading history is
/// a separate resource and is intentionally not part of this transformation.
pub(crate) fn remove_bookmarks(document: &mut Value, book_id: &str) -> Result<usize, String> {
    let bookmarks = document
        .get_mut("bookmarks")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Bookmark resource has no bookmarks array".to_owned())?;
    let previous_len = bookmarks.len();
    bookmarks.retain(|bookmark| bookmark.get("bookId").and_then(Value::as_str) != Some(book_id));
    let removed = previous_len.saturating_sub(bookmarks.len());
    document["schemaVersion"] = json!(crate::models::CURRENT_SCHEMA_VERSION);
    Ok(removed)
}

/// Remap bookmarks for one book from old chapter titles to new chapter titles.
/// Matching follows the source-change rules: normalized titles must be unique
/// in both catalogs, and a pre-existing orphan can reattach only when its saved
/// title is unique in the new catalog.
pub(crate) fn migrate_bookmarks(
    document: &mut Value,
    book_id: &str,
    old_titles: &[String],
    new_titles: &[String],
) -> Result<BookmarkMigrationCounts, String> {
    let bookmarks = document
        .get_mut("bookmarks")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Bookmark resource has no bookmarks array".to_owned())?;
    let mut counts = BookmarkMigrationCounts::default();

    for bookmark in bookmarks
        .iter_mut()
        .filter(|bookmark| bookmark.get("bookId").and_then(Value::as_str) == Some(book_id))
    {
        let was_orphaned = bookmark.get("orphaned").and_then(Value::as_bool) == Some(true);
        let old_title = if was_orphaned {
            bookmark
                .get("chapterTitle")
                .and_then(Value::as_str)
                .map(str::to_owned)
        } else {
            bookmark
                .get("chapterIndex")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| old_titles.get(index))
                .cloned()
        };
        let target_index = old_title.as_deref().and_then(|title| {
            if was_orphaned {
                unique_target_title_index(title, new_titles)
            } else {
                unique_chapter_title_index(title, old_titles, new_titles)
            }
        });

        if let Some(index) = target_index {
            bookmark["chapterIndex"] = json!(index);
            if let Some(fields) = bookmark.as_object_mut() {
                fields.remove("orphaned");
                fields.remove("chapterTitle");
            }
            counts.migrated = counts.migrated.saturating_add(1);
        } else {
            bookmark["orphaned"] = json!(true);
            bookmark["chapterTitle"] = json!(old_title.unwrap_or_else(|| "Unknown chapter".into()));
            counts.orphaned = counts.orphaned.saturating_add(1);
        }
    }

    Ok(counts)
}

pub(crate) fn unique_target_title_index(title: &str, targets: &[String]) -> Option<usize> {
    let normalized = normalize_identity(title);
    if normalized.is_empty() {
        return None;
    }
    let mut matches = targets
        .iter()
        .enumerate()
        .filter(|(_, candidate)| normalize_identity(candidate) == normalized)
        .map(|(index, _)| index);
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

pub(crate) fn unique_chapter_title_index(
    title: &str,
    old: &[String],
    new: &[String],
) -> Option<usize> {
    let normalized = normalize_identity(title);
    if normalized.is_empty() {
        return None;
    }
    let old_matches = old
        .iter()
        .filter(|candidate| normalize_identity(candidate) == normalized)
        .count();
    (old_matches == 1)
        .then(|| unique_target_title_index(title, new))
        .flatten()
}

fn normalize_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
