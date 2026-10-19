use super::{
    rows::{self, array, text, Field, Kind::*},
    *,
};

const BOOK: &[Field] = &[
    ("id", "id", Text),
    ("kind", "_kind", Text),
    ("title", "title", Text),
    ("author", "author", Text),
    ("intro", "intro", Text),
    ("cover", "cover", Text),
    ("source_id", "sourceId", Text),
    ("source_name", "sourceName", Text),
    ("source_group", "sourceGroup", Text),
    ("word_count", "wordCount", Text),
    ("status", "status", Text),
    ("original_json", "original", Json),
    ("page_count", "pageCount", Integer),
    ("document_url", "url", Text),
    ("updated_at_ms", "updatedAtMs", Integer),
    ("instance_id", "bookInstanceId", Text),
    ("catalog_generation", "catalogGeneration", Text),
    ("catalog_version", "catalogVersion", Text),
    ("engine_json", "engine", Json),
    ("engine_state_json", "engineState", Json),
    ("chapter_count", "_chapterCount", Integer),
    ("readable_count", "_readableCount", Integer),
    ("latest_title", "_latest", Text),
];
const CHAPTER: &[Field] = &[
    ("book_id", "_bookId", Text),
    ("id", "id", Text),
    ("position", "_position", Integer),
    ("read_position", "_readPosition", Integer),
    ("title", "title", Text),
    ("is_volume", "volume", Bool),
    ("resource_url", "url", Text),
    ("engine_url", "engineurl", Text),
    ("engine_title", "engineTitle", Text),
    ("engine_data_json", "engineData", Json),
    ("duration_ms", "durationMs", Integer),
    ("protocol", "protocol", Text),
];
const PAGE: &[Field] = &[
    ("book_id", "_bookId", Text),
    ("chapter_id", "_chapterId", Text),
    ("position", "_position", Integer),
    ("id", "id", Text),
    ("url", "url", Text),
    ("width", "width", Integer),
    ("height", "height", Integer),
];
const STREAM: &[Field] = &[
    ("book_id", "_bookId", Text),
    ("chapter_id", "_chapterId", Text),
    ("position", "_position", Integer),
    ("label", "label", Text),
    ("url", "url", Text),
    ("protocol", "protocol", Text),
    ("headers_json", "headers", Json),
];
const PROGRESS: &[Field] = &[
    ("book_id", "_bookId", Text),
    ("chapter_id", "chapterId", Text),
    ("chapter_index", "chapterIndex", Integer),
    ("page_id", "pageId", Text),
    ("anchor", "anchor", Text),
    ("quality", "quality", Text),
    ("char_offset", "charOffset", Integer),
    ("time_ms", "timeMs", Integer),
    ("offset_ratio", "offsetRatio", Real),
    ("page", "page", Integer),
    ("updated_at_ms", "updatedAtMs", Integer),
];
const ALTERNATIVE: &[Field] = &[
    ("book_id", "_bookId", Text),
    ("source_id", "sourceId", Text),
    ("engine_url", "engineurl", Text),
    ("position", "_position", Integer),
    ("source_name", "sourceName", Text),
    ("title", "title", Text),
    ("author", "author", Text),
    ("cover", "cover", Text),
    ("latest", "latest", Text),
    ("engine_json", "engine", Json),
];
const MEDIA: &[Field] = &[
    ("id", "_id", Text),
    ("book_id", "_bookId", Text),
    ("url", "url", Text),
    ("headers_json", "headers", Json),
    ("media_type", "mediaType", Text),
    ("media_format", "format", Text),
    ("request_json", "request", Json),
];

fn clean(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.retain(|key, _| !key.starts_with('_'));
    }
    value
}

pub(super) fn metadata(db: &Connection, id: &str) -> Result<Option<Value>, ResourceError> {
    let mut values = rows::select(db, "books", BOOK, "WHERE id=?1", [id])?;
    let Some(mut book) = values.pop() else {
        return Ok(None);
    };
    let override_fields = &[
        ("title_override", "title", Text),
        ("author_override", "author", Text),
        ("intro_override", "intro", Text),
        ("cover_override", "coverSrc", Text),
    ];
    let overrides = rows::select(db, "books", override_fields, "WHERE id=?1", [id])?
        .pop()
        .unwrap_or(json!({}));
    if overrides.as_object().is_some_and(|o| !o.is_empty()) {
        book["overrides"] = overrides;
    }
    let mut tags = db.prepare("SELECT tag FROM book_tags WHERE book_id=?1 ORDER BY position")?;
    let tags = tags
        .query_map([id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if !tags.is_empty() {
        book["tags"] = json!(tags);
    }
    if let Some(progress) =
        rows::select(db, "reading_progress", PROGRESS, "WHERE book_id=?1", [id])?.pop()
    {
        book["progress"] = clean(progress);
    }
    Ok(Some(book))
}

fn attach_children(
    db: &Connection,
    id: &str,
    chapters: &mut [Value],
    only: Option<&str>,
) -> Result<(), ResourceError> {
    for (table, fields, name) in [
        ("chapter_pages", PAGE, "pages"),
        ("chapter_streams", STREAM, "sources"),
    ] {
        let suffix = if only.is_some() {
            "WHERE book_id=?1 AND chapter_id=?2 ORDER BY position"
        } else {
            "WHERE book_id=?1 ORDER BY chapter_id,position"
        };
        let rows = if let Some(chapter) = only {
            rows::select(db, table, fields, suffix, params![id, chapter])?
        } else {
            rows::select(db, table, fields, suffix, [id])?
        };
        let mut by_chapter = HashMap::<String, Vec<Value>>::new();
        for row in rows {
            by_chapter
                .entry(text(&row, "_chapterId")?.into())
                .or_default()
                .push(clean(row));
        }
        for chapter in chapters.iter_mut() {
            if let Some(values) = chapter["id"].as_str().and_then(|id| by_chapter.remove(id)) {
                chapter[name] = json!(values);
            }
        }
    }
    let mut statement=db.prepare(if only.is_some(){"SELECT chapter_id,filename FROM chapter_files WHERE book_id=?1 AND chapter_id=?2 ORDER BY position"}
        else{"SELECT chapter_id,filename FROM chapter_files WHERE book_id=?1 ORDER BY chapter_id,position"})?;
    let mut cursor = if let Some(chapter) = only {
        statement.query(params![id, chapter])?
    } else {
        statement.query([id])?
    };
    let mut files = HashMap::<String, Vec<String>>::new();
    while let Some(row) = cursor.next()? {
        files.entry(row.get(0)?).or_default().push(row.get(1)?);
    }
    for chapter in chapters {
        if let Some(values) = chapter["id"].as_str().and_then(|id| files.remove(id)) {
            chapter["files"] = json!(values);
        }
    }
    Ok(())
}

pub(super) fn read(db: &Connection, id: &str) -> Result<Option<Value>, ResourceError> {
    let Some(mut book) = metadata(db, id)? else {
        return Ok(None);
    };
    let mut chapters = rows::select(
        db,
        "chapters",
        CHAPTER,
        "WHERE book_id=?1 ORDER BY position",
        [id],
    )?;
    attach_children(db, id, &mut chapters, None)?;
    if book["_kind"] != "pdf" {
        book["chapters"] = json!(chapters.into_iter().map(clean).collect::<Vec<_>>());
    }
    let alternatives = rows::select(
        db,
        "alternative_sources",
        ALTERNATIVE,
        "WHERE book_id=?1 ORDER BY position",
        [id],
    )?;
    if !alternatives.is_empty() {
        book["alternatives"] = json!(alternatives.into_iter().map(clean).collect::<Vec<_>>());
    }
    let media = rows::select(db, "media_requests", MEDIA, "WHERE book_id=?1", [id])?;
    if !media.is_empty() {
        let mut map = Map::new();
        for row in media {
            map.insert(text(&row, "_id")?.into(), clean(row));
        }
        book["media"] = Value::Object(map);
    }
    Ok(Some(clean(book)))
}

pub(super) fn chapter(
    db: &Connection,
    id: &str,
    chapter_id: &str,
) -> Result<Option<Value>, ResourceError> {
    let mut chapters = rows::select(
        db,
        "chapters",
        CHAPTER,
        "WHERE book_id=?1 AND id=?2",
        params![id, chapter_id],
    )?;
    attach_children(db, id, &mut chapters, Some(chapter_id))?;
    Ok(chapters.pop().map(clean))
}

fn write_chapter(
    db: &Connection,
    id: &str,
    position: usize,
    read_position: usize,
    chapter: &Value,
) -> Result<(), ResourceError> {
    let chapter_id = text(chapter, "id")?;
    let mut record = chapter.clone();
    record["_bookId"] = json!(id);
    record["_position"] = json!(position);
    record["_readPosition"] = json!(read_position);
    rows::upsert(db, "chapters", CHAPTER, &["book_id", "id"], &record)?;
    for (table, fields, name) in [
        ("chapter_pages", PAGE, "pages"),
        ("chapter_streams", STREAM, "sources"),
    ] {
        db.execute(
            &format!("DELETE FROM {table} WHERE book_id=?1 AND chapter_id=?2"),
            params![id, chapter_id],
        )?;
        for (position, entry) in array(chapter, name).iter().enumerate() {
            let mut value = entry.clone();
            value["_bookId"] = json!(id);
            value["_chapterId"] = json!(chapter_id);
            value["_position"] = json!(position);
            rows::upsert(
                db,
                table,
                fields,
                &["book_id", "chapter_id", "position"],
                &value,
            )?;
        }
    }
    db.execute(
        "DELETE FROM chapter_files WHERE book_id=?1 AND chapter_id=?2",
        params![id, chapter_id],
    )?;
    for (position, file) in array(chapter, "files").iter().enumerate() {
        db.execute(
            "INSERT INTO chapter_files(book_id,chapter_id,position,filename) VALUES(?1,?2,?3,?4)",
            params![id, chapter_id, position as i64, file.as_str()],
        )?;
    }
    Ok(())
}

pub(super) fn replace(
    db: &Connection,
    _path: &str,
    kind: &str,
    value: &Value,
) -> Result<(), ResourceError> {
    let id = text(value, "id")?;
    let mut book = value.clone();
    book["_kind"] = json!(kind);
    let chapters = array(value, "chapters");
    book["_chapterCount"] = json!(if kind == "pdf" {
        value["pageCount"].as_u64().unwrap_or(0)
    } else {
        chapters.len() as u64
    });
    book["_readableCount"] = json!(if kind == "pdf" {
        value["pageCount"].as_u64().unwrap_or(0)
    } else {
        chapters.iter().filter(|c| c["volume"] != true).count() as u64
    });
    book["_latest"] = chapters
        .iter()
        .rev()
        .find(|c| c["volume"] != true)
        .map(|c| c["title"].clone())
        .unwrap_or(Value::Null);
    rows::upsert(db, "books", BOOK, &["id"], &book)?;
    db.execute("UPDATE books SET title_override=?2,author_override=?3,intro_override=?4,cover_override=?5 WHERE id=?1",params![id,value["overrides"]["title"].as_str(),value["overrides"]["author"].as_str(),value["overrides"]["intro"].as_str(),value["overrides"]["coverSrc"].as_str()])?;
    db.execute("DELETE FROM book_tags WHERE book_id=?1", [id])?;
    for (position, tag) in array(value, "tags").iter().enumerate() {
        db.execute(
            "INSERT INTO book_tags(book_id,position,tag) VALUES(?1,?2,?3)",
            params![id, position as i64, tag.as_str()],
        )?;
    }
    // Catalog replacement is a domain operation. Progress and media are independent.
    db.execute("DELETE FROM chapters WHERE book_id=?1", [id])?;
    let mut read_position = 0;
    for (position, chapter) in chapters.iter().enumerate() {
        if chapter["volume"] != true {
            read_position += 1;
        }
        write_chapter(db, id, position, read_position, chapter)?;
    }
    if value["progress"].is_object() {
        write_progress(db, id, &value["progress"])?;
    }
    if value.get("alternatives").is_some() {
        db.execute("DELETE FROM alternative_sources WHERE book_id=?1", [id])?;
        // 按调用方给出的顺序存储，不在数据库层合并可换来源。
        for (position, alternative) in array(value, "alternatives").iter().enumerate() {
            let mut entry = alternative.clone();
            entry["_bookId"] = json!(id);
            entry["_position"] = json!(position);
            rows::upsert(db, "alternative_sources", ALTERNATIVE, &["book_id", "source_id", "engine_url"], &entry)?;
        }
    }
    for (key, entry) in value["media"].as_object().into_iter().flatten() {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM media_requests WHERE id=?1)",
            [key],
            |r| r.get(0),
        )?;
        if !exists {
            set_media(db, id, key, Some(entry))?;
        }
    }
    Ok(())
}

pub(super) fn write_progress(
    db: &Connection,
    id: &str,
    value: &Value,
) -> Result<(), ResourceError> {
    let timestamp = value["updatedAtMs"].as_i64().unwrap_or(0);
    let previous: i64 = db
        .query_row(
            "SELECT updated_at_ms FROM reading_progress WHERE book_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    if previous > timestamp {
        return Ok(());
    }
    let mut progress = value.clone();
    progress["_bookId"] = json!(id);
    progress["chapterIndex"] = if let Some(chapter) = value["chapterId"].as_str() {
        json!(db
            .query_row(
                "SELECT position FROM chapters WHERE book_id=?1 AND id=?2",
                params![id, chapter],
                |r| r.get::<_, i64>(0)
            )
            .optional()?
            .unwrap_or(0))
    } else {
        json!(value["page"].as_u64().unwrap_or(1).saturating_sub(1))
    };
    rows::upsert(db, "reading_progress", PROGRESS, &["book_id"], &progress)
}

pub(super) fn set_media(
    db: &Connection,
    id: &str,
    key: &str,
    value: Option<&Value>,
) -> Result<(), ResourceError> {
    if let Some(value) = value {
        let mut entry = value.clone();
        entry["_id"] = json!(key);
        entry["_bookId"] = json!(id);
        rows::upsert(db, "media_requests", MEDIA, &["id"], &entry)?;
    } else {
        db.execute(
            "DELETE FROM media_requests WHERE id=?1 AND book_id=?2",
            params![key, id],
        )?;
    }
    Ok(())
}

/// Commit only the chapter cache fields. Engine/catalog identity stays in its row.
pub(super) fn save_chapter_view(
    db: &Connection,
    id: &str,
    chapter_id: &str,
    view: &Value,
) -> Result<(), ResourceError> {
    let (position, read_position): (i64, i64) = db.query_row(
        "SELECT position,read_position FROM chapters WHERE book_id=?1 AND id=?2",
        params![id, chapter_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut canonical =
        chapter(db, id, chapter_id)?.ok_or_else(|| ResourceError::new("Chapter was removed"))?;
    for name in [
        "title",
        "volume",
        "pages",
        "sources",
        "files",
        "durationMs",
        "protocol",
    ] {
        if let Some(value) = view.get(name) {
            canonical[name] = value.clone();
        } else {
            canonical.as_object_mut().unwrap().remove(name);
        }
    }
    let url = view["resource"]["resourceId"]
        .as_str()
        .or_else(|| view["url"].as_str());
    if let Some(url) = url {
        canonical["url"] = json!(crate::resources::content::stored_url(url, id));
    } else {
        canonical.as_object_mut().unwrap().remove("url");
    }
    for name in ["pages", "sources"] {
        for item in canonical[name].as_array_mut().into_iter().flatten() {
            if let Some(src) = item["src"]
                .as_str()
                .or_else(|| item["url"].as_str())
                .map(str::to_owned)
            {
                item["url"] = json!(crate::resources::content::stored_url(&src, id));
            }
            if let Some(format) = item.get("format").cloned() {
                item["protocol"] = format;
            }
        }
    }
    write_chapter(
        db,
        id,
        position as usize,
        read_position as usize,
        &canonical,
    )
}

pub(super) fn progress(db: &Connection, id: &str) -> Result<Value, ResourceError> {
    Ok(
        rows::select(db, "reading_progress", PROGRESS, "WHERE book_id=?1", [id])?
            .pop()
            .map(clean)
            .unwrap_or(Value::Null),
    )
}

pub(super) fn library_rows(
    db: &Connection,
    id: Option<&str>,
) -> Result<Vec<(String, Value)>, ResourceError> {
    const FIELDS: &[Field] = &[
        ("b.kind", "_kind", Text),
        ("b.id", "id", Text),
        ("b.title", "title", Text),
        ("b.author", "author", Text),
        ("b.cover", "cover", Text),
        ("b.readable_count", "total", Integer),
        ("b.latest_title", "latest", Text),
        (
            "CASE WHEN b.updated_at_ms>0 THEN b.updated_at_ms/1000 END",
            "updatedAt",
            Integer,
        ),
        (
            "CASE WHEN b.progress_updated_at_ms>0 THEN b.progress_updated_at_ms/1000 END",
            "readAt",
            Integer,
        ),
        (
            "CASE WHEN b.progress_updated_at_ms>0 THEN coalesce(c.read_position,p.page,0) END",
            "read",
            Integer,
        ),
    ];
    let base="book_previews b LEFT JOIN reading_progress p ON p.book_id=b.id LEFT JOIN chapters c ON c.book_id=b.id AND c.id=b.chapter_id";
    let values = if let Some(id) = id {
        rows::select(db, base, FIELDS, "WHERE b.id=?1", [id])?
    } else {
        rows::select(
            db,
            &format!("{base} JOIN library_books l ON l.book_id=b.id"),
            FIELDS,
            "ORDER BY l.position",
            [],
        )?
    };
    values
        .into_iter()
        .map(|mut row| {
            let kind = text(&row, "_kind")?.to_owned();
            if let Some(cover) = row["cover"].as_str() {
                row["cover"] = json!(crate::resources::content::stored_url(
                    cover,
                    text(&row, "id")?
                ));
            }
            Ok((kind, clean(row)))
        })
        .collect()
}

pub(super) fn library_preview(db: &Connection, id: &str) -> Result<(String, Value), ResourceError> {
    library_rows(db, Some(id))?
        .pop()
        .ok_or_else(|| ResourceError::new("Library content is not cached"))
}

#[cfg(test)]
pub(super) fn preview(db: &Connection, id: &str) -> Result<Value, ResourceError> {
    let book = metadata(db, id)?.ok_or_else(|| ResourceError::new("Book is not cached"))?;
    let kind = text(&book, "_kind")?.to_owned();
    let count = book["_chapterCount"].clone();
    let latest = book["_latest"].clone();
    let progress = book["progress"].clone();
    let mut view = crate::resources::content::public_book_view(&clean(book), &kind)?;
    for key in ["chapters", "displayBase", "displayOverrides"] {
        view.as_object_mut().unwrap().remove(key);
    }
    view["chapterCount"] = count;
    view["latestChapter"] = latest;
    view["progress"]["chapterIndex"] = progress["chapterIndex"].as_u64().unwrap_or(0).into();
    Ok(view)
}

pub(super) fn all_media(db: &Connection) -> Result<Vec<(String, Value)>, ResourceError> {
    rows::select(db, "media_requests", MEDIA, "ORDER BY book_id,id", [])?
        .into_iter()
        .map(|v| Ok((text(&v, "_id")?.into(), clean(v))))
        .collect()
}
