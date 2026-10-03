//! Pure JSON transformations used to build a source-replacement commit.
//!
//! These helpers do not read or write resources. Callers provide snapshots
//! captured while holding the ResourceStore writer guard, then persist the
//! returned JSON as one file transaction.

use serde_json::{json, Value};

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

#[cfg(test)]
mod tests {
    use super::{migrate_bookmarks, upsert_shelf_entry};
    use serde_json::{json, Value};

    fn book(id: &str, title: &str) -> Value {
        json!({
            "id": id,
            "title": title,
            "author": "Author",
            "coverSrc": "resource://books/book-a/assets/cover.png",
            "chapterCount": 2,
            "latestChapter": "Chapter Two",
            "progress": { "chapterIndex": 1, "offset": 19 },
        })
    }

    #[test]
    fn shelf_update_preserves_groups_and_custom_order() {
        let mut shelf = json!({
            "sort": "custom",
            "sortOrder": "ascending",
            "books": [
                { "id": "book-b", "title": "B", "groups": ["later"] },
                { "id": "book-a", "title": "Old", "groups": ["favorites", "read later"] },
                { "id": "book-c", "title": "C", "groups": [] },
            ],
        });

        upsert_shelf_entry(&mut shelf, "book-a", &book("book-a", "New title")).unwrap();

        let entries = shelf["books"].as_array().unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["book-b", "book-a", "book-c"]
        );
        let updated = &entries[1];
        assert_eq!(updated["title"], "New title");
        assert_eq!(updated["groups"], json!(["favorites", "read later"]));
        assert_eq!(updated["progress"]["offset"], 19);
        assert_eq!(shelf["sort"], "custom");
        assert_eq!(shelf["sortOrder"], "ascending");
    }

    #[test]
    fn shelf_update_reapplies_derived_sort_and_appends_missing_books() {
        let mut shelf = json!({
            "sort": "title",
            "sortOrder": "ascending",
            "books": [
                { "id": "book-a", "title": "Zebra", "groups": ["group"] },
                { "id": "book-b", "title": "Mango", "groups": [] },
            ],
        });

        upsert_shelf_entry(&mut shelf, "book-a", &book("book-a", "Apple")).unwrap();
        upsert_shelf_entry(&mut shelf, "book-c", &book("book-c", "Pear")).unwrap();

        let entries = shelf["books"].as_array().unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["book-a", "book-b", "book-c"]
        );
        assert_eq!(entries[0]["groups"], json!(["group"]));
        assert_eq!(entries[2]["groups"], json!([]));
    }

    #[test]
    fn bookmark_migration_maps_unique_titles_and_reattaches_unique_orphans() {
        let old_titles = vec!["Chapter One".to_owned(), "Chapter Two".to_owned()];
        let new_titles = vec!["Chapter Two".to_owned(), "Chapter One!".to_owned()];
        let mut document = json!({
            "bookmarks": [
                { "id": "current", "bookId": "book-a", "chapterIndex": 0, "offset": 5 },
                { "id": "orphan", "bookId": "book-a", "chapterIndex": 8, "orphaned": true, "chapterTitle": "Chapter Two", "offset": 3 },
                { "id": "other-book", "bookId": "book-b", "chapterIndex": 1, "offset": 7 },
            ],
        });

        let counts = migrate_bookmarks(&mut document, "book-a", &old_titles, &new_titles).unwrap();

        assert_eq!(counts.migrated, 2);
        assert_eq!(counts.orphaned, 0);
        assert_eq!(document["bookmarks"][0]["chapterIndex"], 1);
        assert!(document["bookmarks"][0].get("orphaned").is_none());
        assert_eq!(document["bookmarks"][1]["chapterIndex"], 0);
        assert!(document["bookmarks"][1].get("chapterTitle").is_none());
        assert_eq!(document["bookmarks"][2]["chapterIndex"], 1);
    }

    #[test]
    fn ambiguous_or_removed_chapters_remain_orphaned_with_their_old_title() {
        let old_titles = vec![
            "Same Chapter".to_owned(),
            "Same-Chapter".to_owned(),
            "Removed Chapter".to_owned(),
        ];
        let new_titles = vec!["Same Chapter".to_owned()];
        let mut document = json!({
            "bookmarks": [
                { "id": "ambiguous", "bookId": "book-a", "chapterIndex": 0, "offset": 1 },
                { "id": "removed", "bookId": "book-a", "chapterIndex": 2, "offset": 2 },
            ],
        });

        let counts = migrate_bookmarks(&mut document, "book-a", &old_titles, &new_titles).unwrap();

        assert_eq!(counts.migrated, 0);
        assert_eq!(counts.orphaned, 2);
        assert_eq!(document["bookmarks"][0]["orphaned"], true);
        assert_eq!(document["bookmarks"][0]["chapterTitle"], "Same Chapter");
        assert_eq!(document["bookmarks"][1]["orphaned"], true);
        assert_eq!(document["bookmarks"][1]["chapterTitle"], "Removed Chapter");
    }

    #[test]
    fn malformed_shelf_and_bookmark_documents_are_rejected() {
        let mut shelf = json!({ "books": {} });
        assert!(upsert_shelf_entry(&mut shelf, "book-a", &book("book-a", "A")).is_err());

        let mut bookmarks = json!({ "bookmarks": null });
        assert!(migrate_bookmarks(&mut bookmarks, "book-a", &[], &[]).is_err());
    }
}
