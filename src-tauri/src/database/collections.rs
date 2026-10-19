//! Explicit business collections; JSON is only the external resource format.
use super::{
    rows::{self, array, text, Field, Kind::*},
    *,
};

const REPLACEMENT: &[Field] = &[
    ("id", "id", Text),
    ("position", "_position", Integer),
    ("name", "name", Text),
    ("pattern", "pattern", Text),
    ("replacement", "replacement", Text),
    ("enabled", "enabled", Bool),
    ("is_regex", "isRegex", Bool),
    ("scope", "scope", Text),
];
const TOC: &[Field] = &[
    ("id", "id", Text),
    ("position", "_position", Integer),
    ("name", "name", Text),
    ("rule", "rule", Text),
    ("example", "example", Text),
    ("serial_number", "serialNumber", Integer),
    ("enabled", "enable", Bool),
];
const TTS: &[Field] = &[
    ("id", "id", Text),
    ("position", "_position", Integer),
    ("name", "name", Text),
    ("enabled", "enabled", Bool),
    ("definition_json", "config", Json),
];
const FAVORITE: &[Field] = &[
    ("source_id", "sourceId", Text),
    ("category_id", "categoryId", Text),
    ("position", "position", Integer),
    ("source_name", "sourceName", Text),
    ("title", "title", Text),
    ("kind", "kind", Text),
    ("style_json", "style", Json),
    ("created_at_ms", "createdAtMs", Integer),
];
const SUBSCRIPTION: &[Field] = &[
    ("source_id", "sourceId", Text),
    ("position", "_position", Integer),
    ("filter", "filter", Text),
];
const ARTICLE: &[Field] = &[
    ("source_id", "sourceId", Text),
    ("article_id", "articleId", Text),
    ("position", "_position", Integer),
    ("is_read", "isRead", Bool),
    ("is_favorite", "isFavorite", Bool),
    ("updated_at_ms", "updatedAtMs", Integer),
    ("content_ref", "contentRef", Text),
];

pub(super) fn clean(mut value: Value) -> Value {
    if let Some(o) = value.as_object_mut() {
        o.retain(|k, _| !k.starts_with('_'));
    }
    value
}
pub(super) fn list(
    db: &Connection,
    table: &str,
    fields: &[Field],
) -> Result<Vec<Value>, ResourceError> {
    Ok(rows::select(db, table, fields, "ORDER BY position", [])?
        .into_iter()
        .map(clean)
        .collect())
}
pub(super) fn replace(
    db: &Connection,
    table: &str,
    fields: &[Field],
    keys: &[&str],
    values: &[Value],
) -> Result<(), ResourceError> {
    db.execute(&format!("DELETE FROM {table}"), [])?;
    for (position, value) in values.iter().enumerate() {
        let mut value = value.clone();
        value["_position"] = json!(position);
        rows::upsert(db, table, fields, keys, &value)?;
    }
    Ok(())
}

pub(super) fn read(db: &Connection, path: &str) -> Result<Option<Value>, ResourceError> {
    Ok(Some(match path {
        "replacement-rules.json" => {
            json!({"schemaVersion":1,"rules":list(db,"replacement_rules",REPLACEMENT)?})
        }
        "txt-toc-rules.json" => json!({"schemaVersion":1,"rules":list(db,"txt_toc_rules",TOC)?}),
        "http-tts.json" => json!({"schemaVersion":1,"configs":list(db,"http_tts_configs",TTS)?}),
        "discovery-favorites.json" => {
            json!({"schemaVersion":1,"favorites":list(db,"discovery_favorites",FAVORITE)?})
        }
        "rss-state.json" => {
            json!({"schemaVersion":1,"subscriptions":list(db,"rss_subscriptions",SUBSCRIPTION)?,"articles":list(db,"rss_articles",ARTICLE)?})
        }
        _ => return Ok(None),
    }))
}
pub(super) fn write(db: &Connection, path: &str, value: &Value) -> Result<bool, ResourceError> {
    match path {
        "replacement-rules.json" => replace(
            db,
            "replacement_rules",
            REPLACEMENT,
            &["id"],
            array(value, "rules"),
        )?,
        "txt-toc-rules.json" => replace(db, "txt_toc_rules", TOC, &["id"], array(value, "rules"))?,
        "http-tts.json" => replace(
            db,
            "http_tts_configs",
            TTS,
            &["id"],
            array(value, "configs"),
        )?,
        "discovery-favorites.json" => replace(
            db,
            "discovery_favorites",
            FAVORITE,
            &["source_id", "category_id"],
            array(value, "favorites"),
        )?,
        "rss-state.json" => {
            replace(
                db,
                "rss_subscriptions",
                SUBSCRIPTION,
                &["source_id"],
                array(value, "subscriptions"),
            )?;
            replace(
                db,
                "rss_articles",
                ARTICLE,
                &["source_id", "article_id"],
                array(value, "articles"),
            )?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

const SOURCE: &[Field] = &[
    ("id", "id", Text),
    ("position", "_position", Integer),
    ("name", "name", Text),
    ("source_group", "group", Text),
    ("enabled", "enabled", Bool),
    ("definition_json", "source", Json),
];
pub(super) fn sources(db: &Connection) -> Result<Value, ResourceError> {
    Ok(json!(list(db, "sources", SOURCE)?))
}
pub(super) fn save_source(
    db: &Connection,
    position: usize,
    value: &Value,
) -> Result<(), ResourceError> {
    let record: crate::application::SourceRecord =
        serde_json::from_value(value.clone()).map_err(|e| ResourceError::new(e.to_string()))?;
    super::write::apply(db, Change::Upsert {
        table: "sources".into(), keys: vec!["id".into()],
        values: source_values(position, &record),
    })
}

pub(super) fn source_values(position: usize, record: &crate::application::SourceRecord) -> Map<String, Value> {
    let metadata = crate::application::source_metadata_rows(std::slice::from_ref(record)).remove(0);
    json!({"id":record.id,"position":position,"name":record.name,"source_group":record.group,
            "enabled":record.enabled,"definition_json":record.source,"is_rss":metadata.is_rss,
            "content_type":metadata.content_type,"search_type":metadata.search_type,"media_type":metadata.media_type,
            "can_search":metadata.capabilities.search,"can_detail":metadata.capabilities.detail,
            "can_toc":metadata.capabilities.toc,"can_content":metadata.capabilities.content,
            "user_agent_override":metadata.user_agent_override}).as_object().unwrap().clone()
}

pub(super) fn library(db: &Connection) -> Result<Value, ResourceError> {
    let mut out = json!({"groups":[]});
    for kind in crate::resources::content::CONTENT_TYPES {
        out[kind] = json!([]);
    }
    for (kind, preview) in books::library_rows(db, None)? {
        out[&kind]
            .as_array_mut()
            .ok_or_else(|| ResourceError::new("Unknown library content type"))?
            .push(preview);
    }
    let mut stmt = db.prepare("SELECT name FROM library_groups ORDER BY position")?;
    for name in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let name = name?;
        let mut ids = db.prepare(
            "SELECT book_id FROM library_group_books WHERE group_name=?1 ORDER BY position",
        )?;
        let ids = ids
            .query_map([&name], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        out["groups"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":name,"id":ids}));
    }
    Ok(out)
}
pub(super) fn save_library(db: &Connection, value: &Value) -> Result<(), ResourceError> {
    db.execute("DELETE FROM library_books", [])?;
    db.execute("DELETE FROM library_groups", [])?;
    let mut position = 0;
    for kind in crate::resources::content::CONTENT_TYPES {
        for book in array(value, kind) {
            db.execute(
                "INSERT INTO library_books(book_id,position) VALUES(?1,?2)",
                params![text(book, "id")?, position],
            )?;
            position += 1;
        }
    }
    for (position, group) in array(value, "groups").iter().enumerate() {
        let name = text(group, "name")?;
        db.execute(
            "INSERT INTO library_groups(name,position) VALUES(?1,?2)",
            params![name, position as i64],
        )?;
        for (position, id) in array(group, "id").iter().enumerate() {
            db.execute(
                "INSERT INTO library_group_books(group_name,book_id,position) VALUES(?1,?2,?3)",
                params![name, id.as_str(), position as i64],
            )?;
        }
    }
    Ok(())
}

pub(super) fn save_positions(
    db: &Connection,
    shelf: &crate::models::ShelfDocument,
) -> Result<(), ResourceError> {
    db.execute("DELETE FROM library_books", [])?;
    db.execute("DELETE FROM library_groups", [])?;
    for (position, name) in shelf.groups.iter().enumerate() {
        db.execute(
            "INSERT INTO library_groups(name,position) VALUES(?1,?2)",
            params![name, position as i64],
        )?;
    }
    for (position, book) in shelf.books.iter().enumerate() {
        db.execute(
            "INSERT INTO library_books(book_id,position) VALUES(?1,?2)",
            params![book.id, position as i64],
        )?;
        for name in &book.groups {
            db.execute(
                "INSERT INTO library_group_books(group_name,book_id,position) VALUES(?1,?2,?3)",
                params![name, book.id, position as i64],
            )?;
        }
    }
    Ok(())
}
