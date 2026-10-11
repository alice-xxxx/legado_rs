//! 书架分组、批量归组与排序。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

pub(crate) fn normalize_shelf_groups(shelf: &mut Value) -> Result<(), String> {
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
        if normalized_book_groups.len() > 64 {
            return Err("A book cannot belong to more than 64 shelf groups".to_owned());
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
