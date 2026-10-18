use super::{
    collections::clean,
    rows::{self, array, Field, Kind::*},
    *,
};
const CATEGORY: &[Field] = &[
    ("source_id", "_sourceId", Text),
    ("private", "_private", Bool),
    ("position", "_position", Integer),
    ("category_id", "categoryId", Text),
    ("title", "title", Text),
    ("kind", "kind", Text),
    ("style_json", "style", Json),
    ("url", "url", Text),
];
const PAGE: &[Field] = &[
    ("id", "_id", Text),
    ("keyword", "keyword", Text),
    ("page", "page", Integer),
    ("complete", "complete", Bool),
];
const ITEM: &[Field] = &[
    ("page_id", "_pageId", Text),
    ("position", "_position", Integer),
    ("result_id", "resultId", Text),
    ("article_id", "articleId", Text),
    ("source_id", "sourceId", Text),
    ("source_name", "sourceName", Text),
    ("title", "title", Text),
    ("author", "author", Text),
    ("intro", "intro", Text),
    ("cover", "cover", Text),
    ("url", "url", Text),
    ("content_src", "contentSrc", Text),
    ("published_at", "latestChapter", Text),
    ("is_read", "isRead", Bool),
    ("is_favorite", "isFavorite", Bool),
];
pub(super) fn address(path: &str) -> Option<(&str, bool, bool)> {
    if let Some(id) = path
        .strip_prefix("source-engine/discovery-categories/")
        .and_then(|p| p.strip_suffix(".json"))
    {
        Some((id, true, false))
    } else {
        path.strip_prefix("discovery/")
            .and_then(|p| p.strip_suffix(".json"))
            .map(|id| (id, false, id.starts_with("rss-")))
    }
}
pub(super) fn read(
    db: &Connection,
    id: &str,
    private: bool,
    page: bool,
) -> Result<Option<Value>, ResourceError> {
    if page {
        let Some(mut value) = rows::select(db, "discovery_pages", PAGE, "WHERE id=?1", [id])?.pop()
        else {
            return Ok(None);
        };
        value = clean(value);
        value["schemaVersion"] = json!(1);
        value["results"] = json!(rows::select(
            db,
            "discovery_page_items",
            ITEM,
            "WHERE page_id=?1 ORDER BY position",
            [id]
        )?
        .into_iter()
        .map(clean)
        .collect::<Vec<_>>());
        value["errors"] = json!([]);
        return Ok(Some(value));
    }
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM discovery_sources WHERE source_id=?1 AND private=?2)",
        params![id, private],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(None);
    }
    let categories = rows::select(
        db,
        "discovery_categories",
        CATEGORY,
        "WHERE source_id=?1 AND private=?2 ORDER BY position",
        params![id, private],
    )?;
    Ok(Some(
        json!({"schemaVersion":1,"sourceId":id,"categories":categories.into_iter().map(clean).collect::<Vec<_>>()}),
    ))
}
pub(super) fn write(
    db: &Connection,
    id: &str,
    private: bool,
    page: bool,
    value: &Value,
) -> Result<(), ResourceError> {
    if page {
        let mut page = value.clone();
        page["_id"] = json!(id);
        rows::upsert(db, "discovery_pages", PAGE, &["id"], &page)?;
        db.execute("DELETE FROM discovery_page_items WHERE page_id=?1", [id])?;
        for (position, item) in array(value, "results").iter().enumerate() {
            let mut item = item.clone();
            item["_pageId"] = json!(id);
            item["_position"] = json!(position);
            rows::upsert(
                db,
                "discovery_page_items",
                ITEM,
                &["page_id", "position"],
                &item,
            )?;
        }
        return Ok(());
    }
    db.execute(
        "INSERT OR IGNORE INTO discovery_sources(source_id,private) VALUES(?1,?2)",
        params![id, private],
    )?;
    db.execute(
        "DELETE FROM discovery_categories WHERE source_id=?1 AND private=?2",
        params![id, private],
    )?;
    for (position, category) in array(value, "categories").iter().enumerate() {
        let mut category = category.clone();
        category["_sourceId"] = json!(id);
        category["_private"] = json!(private);
        category["_position"] = json!(position);
        rows::upsert(
            db,
            "discovery_categories",
            CATEGORY,
            &["source_id", "private", "position"],
            &category,
        )?;
    }
    Ok(())
}
