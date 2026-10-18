use super::{
    collections::{clean, list, replace},
    rows::{self, array, text, Field, Kind::*},
    *,
};
const SESSION: &[Field] = &[
    ("session_id", "sessionId", Text),
    ("book_id", "bookId", Text),
    ("position", "_position", Integer),
    ("duration_ms", "durationMs", Integer),
    ("ended_at_ms", "endedAtMs", Integer),
    ("utc_offset_minutes", "utcOffsetMinutes", Integer),
];
const BOOK: &[Field] = &[
    ("book_id", "bookId", Text),
    ("position", "_position", Integer),
    ("session_count", "sessionCount", Integer),
    ("total_duration_ms", "totalDurationMs", Integer),
    ("last_read_at_ms", "lastReadAtMs", Integer),
];
const DAY: &[Field] = &[
    ("day", "day", Text),
    ("position", "_position", Integer),
    ("session_count", "sessionCount", Integer),
    ("total_duration_ms", "totalDurationMs", Integer),
];
const DAY_BOOK: &[Field] = &[
    ("day", "_day", Text),
    ("book_id", "bookId", Text),
    ("position", "_position", Integer),
    ("session_count", "sessionCount", Integer),
    ("total_duration_ms", "totalDurationMs", Integer),
];
const SEARCH: &[Field] = &[
    ("normalized_query", "normalizedQuery", Text),
    ("query", "query", Text),
    ("position", "_position", Integer),
    ("usage", "usage", Integer),
    ("first_use_time_ms", "firstUseTimeMs", Integer),
    ("last_use_time_ms", "lastUseTimeMs", Integer),
];
const BOOKMARK: &[Field] = &[
    ("id", "id", Text),
    ("book_id", "_bookId", Text),
    ("position", "_position", Integer),
    ("chapter_id", "chapterId", Text),
    ("chapter_index", "chapterIndex", Integer),
    ("chapter_title", "chapterTitle", Text),
    ("offset", "offset", Integer),
    ("page_id", "pageId", Text),
    ("anchor", "anchor", Text),
    ("note", "note", Text),
    ("orphaned", "orphaned", Bool),
    ("created_at_ms", "createdAtMs", Integer),
    ("updated_at_ms", "updatedAtMs", Integer),
];

pub(super) fn search(db: &Connection) -> Result<Value, ResourceError> {
    let entries = rows::select(
        db,
        "search_history",
        SEARCH,
        "ORDER BY last_use_time_ms DESC,usage DESC,normalized_query",
        [],
    )?
    .into_iter()
    .map(clean)
    .collect::<Vec<_>>();
    let mut stmt = db.prepare("SELECT id FROM recorded_searches ORDER BY position")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"entries":entries,"processedSearchIds":ids}))
}

pub(super) fn record_search(
    db: &Connection,
    id: &str,
    query: &str,
    normalized: &str,
    now: u64,
) -> Result<(), ResourceError> {
    let inserted=db.execute("INSERT OR IGNORE INTO recorded_searches(id,position) SELECT ?1,coalesce(max(position)+1,0) FROM recorded_searches",[id])?;
    if inserted == 0 {
        return Ok(());
    }
    db.execute("INSERT INTO search_history(normalized_query,query,position,usage,first_use_time_ms,last_use_time_ms) VALUES(?1,?2,0,1,?3,?3) ON CONFLICT(normalized_query) DO UPDATE SET query=excluded.query,usage=usage+1,first_use_time_ms=min(first_use_time_ms,excluded.first_use_time_ms),last_use_time_ms=max(last_use_time_ms,excluded.last_use_time_ms)",params![normalized,query,now.min(i64::MAX as u64) as i64])?;
    db.execute("DELETE FROM search_history WHERE normalized_query NOT IN (SELECT normalized_query FROM search_history ORDER BY last_use_time_ms DESC,usage DESC,normalized_query LIMIT 100)",[])?;
    db.execute("DELETE FROM recorded_searches WHERE id NOT IN (SELECT id FROM recorded_searches ORDER BY position DESC LIMIT 256)",[])?;
    Ok(())
}

pub(super) fn save_bookmark(
    db: &Connection,
    book_id: &str,
    bookmark: &Value,
) -> Result<(), ResourceError> {
    let id = text(bookmark, "id")?;
    let previous: Option<(String, i64)> = db
        .query_row(
            "SELECT book_id,position FROM bookmarks WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if previous.as_ref().is_some_and(|(book, _)| book != book_id) {
        return Err(ResourceError::new("Bookmark belongs to another book"));
    }
    let position = match previous {
        Some((_, position)) => position,
        None => db.query_row(
            "SELECT coalesce(max(position)+1,0) FROM bookmarks WHERE book_id=?1",
            [book_id],
            |r| r.get::<_, i64>(0),
        )?,
    };
    let mut value = bookmark.clone();
    value["_bookId"] = json!(book_id);
    value["_position"] = json!(position);
    rows::upsert(db, "bookmarks", BOOKMARK, &["id"], &value)
}

pub(super) fn record_session(
    db: &Connection,
    session: &crate::reading_tools::ReadingSession,
) -> Result<(), ResourceError> {
    let inserted=db.execute("INSERT OR IGNORE INTO reading_sessions(session_id,book_id,position,duration_ms,ended_at_ms,utc_offset_minutes) SELECT ?1,?2,coalesce(max(position)+1,0),?3,?4,?5 FROM reading_sessions",
        params![session.session_id,session.book_id,session.duration_ms as i64,session.ended_at_ms as i64,session.utc_offset_minutes])?;
    if inserted == 0 {
        return Ok(());
    }
    db.execute(
        "UPDATE reading_totals SET sessions=sessions+1,duration_ms=duration_ms+?1",
        [session.duration_ms as i64],
    )?;
    db.execute("INSERT INTO reading_books(book_id,position,session_count,total_duration_ms,last_read_at_ms) VALUES(?1,0,1,?2,?3) ON CONFLICT(book_id) DO UPDATE SET session_count=session_count+1,total_duration_ms=total_duration_ms+excluded.total_duration_ms,last_read_at_ms=max(last_read_at_ms,excluded.last_read_at_ms)",params![session.book_id,session.duration_ms as i64,session.ended_at_ms as i64])?;
    for (day, duration) in crate::reading_tools::split_session_by_local_day(session) {
        db.execute("INSERT INTO reading_days(day,position,session_count,total_duration_ms) VALUES(?1,0,1,?2) ON CONFLICT(day) DO UPDATE SET session_count=session_count+1,total_duration_ms=total_duration_ms+excluded.total_duration_ms",params![day,duration as i64])?;
        db.execute("INSERT INTO reading_day_books(day,book_id,position,session_count,total_duration_ms) VALUES(?1,?2,0,1,?3) ON CONFLICT(day,book_id) DO UPDATE SET session_count=session_count+1,total_duration_ms=total_duration_ms+excluded.total_duration_ms",params![day,session.book_id,duration as i64])?;
    }
    Ok(())
}
pub(super) fn read(db: &Connection) -> Result<Value, ResourceError> {
    let (duration, sessions): (i64, i64) = db.query_row(
        "SELECT duration_ms,sessions FROM reading_totals WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut days = rows::select(db, "reading_days", DAY, "ORDER BY day DESC", [])?
        .into_iter()
        .map(clean)
        .collect::<Vec<_>>();
    let all = rows::select(
        db,
        "reading_day_books",
        DAY_BOOK,
        "ORDER BY day,position",
        [],
    )?;
    let mut by_day = HashMap::<String, Vec<Value>>::new();
    for row in all {
        by_day
            .entry(text(&row, "_day")?.into())
            .or_default()
            .push(clean(row));
    }
    for day in &mut days {
        day["books"] = json!(by_day
            .remove(day["day"].as_str().unwrap_or(""))
            .unwrap_or_default());
    }
    let mut bookmarks = Map::new();
    for bookmark in rows::select(db, "bookmarks", BOOKMARK, "ORDER BY book_id,position", [])? {
        let book = text(&bookmark, "_bookId")?.to_owned();
        bookmarks
            .entry(book)
            .or_insert(json!([]))
            .as_array_mut()
            .unwrap()
            .push(clean(bookmark));
    }
    Ok(
        json!({"schemaVersion":2,"reading":{"sessions":list(db,"reading_sessions",SESSION)?,"books":rows::select(db,"reading_books",BOOK,"ORDER BY last_read_at_ms DESC,book_id",[])?.into_iter().map(clean).collect::<Vec<_>>(),"days":days,"totalDurationMs":duration,"totalSessions":sessions},"search":search(db)?,"bookmarks":bookmarks}),
    )
}
pub(super) fn write(db: &Connection, value: &Value) -> Result<(), ResourceError> {
    let reading = &value["reading"];
    let search = &value["search"];
    replace(
        db,
        "reading_sessions",
        SESSION,
        &["session_id"],
        array(reading, "sessions"),
    )?;
    replace(
        db,
        "reading_books",
        BOOK,
        &["book_id"],
        array(reading, "books"),
    )?;
    replace(db, "reading_days", DAY, &["day"], array(reading, "days"))?;
    for day in array(reading, "days") {
        for (position, book) in array(day, "books").iter().enumerate() {
            let mut book = book.clone();
            book["_day"] = day["day"].clone();
            book["_position"] = json!(position);
            rows::upsert(
                db,
                "reading_day_books",
                DAY_BOOK,
                &["day", "book_id"],
                &book,
            )?;
        }
    }
    db.execute(
        "UPDATE reading_totals SET duration_ms=?1,sessions=?2 WHERE id=1",
        params![
            reading["totalDurationMs"].as_i64().unwrap_or(0),
            reading["totalSessions"].as_i64().unwrap_or(0)
        ],
    )?;
    replace(
        db,
        "search_history",
        SEARCH,
        &["normalized_query"],
        array(search, "entries"),
    )?;
    db.execute("DELETE FROM recorded_searches", [])?;
    for (position, id) in array(search, "processedSearchIds").iter().enumerate() {
        db.execute(
            "INSERT INTO recorded_searches(id,position) VALUES(?1,?2)",
            params![id.as_str(), position as i64],
        )?;
    }
    db.execute("DELETE FROM bookmarks", [])?;
    for (id, values) in value["bookmarks"].as_object().into_iter().flatten() {
        for (position, bookmark) in values.as_array().into_iter().flatten().enumerate() {
            let mut bookmark = bookmark.clone();
            bookmark["_bookId"] = json!(id);
            bookmark["_position"] = json!(position);
            rows::upsert(db, "bookmarks", BOOKMARK, &["id"], &bookmark)?;
        }
    }
    Ok(())
}
