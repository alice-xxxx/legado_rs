//! Business repositories. The internal resource adapter is not a public read API.
use crate::{resources::ResourceError, storage_owner::Owner};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
mod books;
mod collections;
mod discovery;
mod history;
mod rows;
mod sources;
mod tasks;
mod write;
pub(crate) use write::Change;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) mod fe_db;
pub(crate) const FILE_NAME: &str = "library.sqlite3";
const ROOTS: &[&str] = &[
    "sources.json",
    "content-library.json",
    "history.json",
    "tasks.json",
    "rss-state.json",
    "replacement-rules.json",
    "txt-toc-rules.json",
    "http-tts.json",
    "discovery-favorites.json",
];
struct Writer {
    root: PathBuf,
    connection: Option<Connection>,
}
type WriteOwner = Owner<Writer>;
static WRITERS: OnceLock<Mutex<HashMap<PathBuf, Weak<WriteOwner>>>> = OnceLock::new();
impl From<rusqlite::Error> for ResourceError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new(format!("Database operation failed: {e}"))
    }
}
fn check_database_path(root: &Path) -> Result<(), ResourceError> {
    for name in [
        FILE_NAME.to_owned(),
        format!("{FILE_NAME}-wal"),
        format!("{FILE_NAME}-shm"),
    ] {
        crate::resources::check_path_no_symlink(root, &root.join(name), true)?;
    }
    Ok(())
}
impl Writer {
    fn connection(&mut self) -> Result<&mut Connection, ResourceError> {
        if self.connection.is_none() {
            check_database_path(&self.root)?;
            let db = Connection::open(self.root.join(FILE_NAME))?;
            db.busy_timeout(Duration::from_secs(5))?;
            db.pragma_update(None, "foreign_keys", true)?;
            db.pragma_update(None, "journal_mode", "WAL")?;
            let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if version > 1 {
                return Err(ResourceError::new(
                    "Database was created by a newer application",
                ));
            }
            if version == 0 {
                db.execute_batch(concat!(
                    "BEGIN IMMEDIATE;",
                    include_str!("database/schema.sql"),
                    "COMMIT;"
                ))?;
            }
            self.connection = Some(db);
        }
        Ok(self.connection.as_mut().expect("writer initialized"))
    }
}
#[derive(Clone)]
pub(crate) struct Database {
    root: PathBuf,
    writer: Arc<WriteOwner>,
    _settings: Arc<crate::configuration::FileOwner>,
}
impl Database {
    #[cfg(test)]
    pub(crate) fn save_task_changes(
        &self,
        created: Option<Value>,
        states: Vec<Value>,
    ) -> Result<(), ResourceError> {
        self.transaction(move |db| {
            if let Some(task) = created {
                tasks::insert(db, &task)?;
            }
            tasks::update_states(db, &states)
        })
    }
    pub(crate) fn read_search_history(&self) -> Result<Value, ResourceError> {
        history::search(&self.connect()?)
    }
    pub(crate) fn record_search(
        &self,
        id: String,
        query: String,
        normalized: String,
        now: u64,
    ) -> Result<(), ResourceError> {
        self.transaction(move |db| history::record_search(db, &id, &query, &normalized, now))
    }
    pub(crate) fn at(root: &Path) -> Self {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
        let mut owners = WRITERS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let writer = owners
            .get(&root)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                owners.retain(|_, o| o.strong_count() > 0);
                let owner = Arc::new(
                    Owner::new(
                        "database-writer",
                        Writer {
                            root: root.clone(),
                            connection: None,
                        },
                    )
                    .expect("start database owner"),
                );
                owners.insert(root.clone(), Arc::downgrade(&owner));
                owner
            });
        let settings = crate::configuration::owner(&root).expect("start settings owner");
        Self {
            root,
            writer,
            _settings: settings,
        }
    }
    pub(crate) fn open(root: &Path) -> Result<Self, ResourceError> {
        std::fs::create_dir_all(root)?;
        let db = Self::at(root);
        db.writer.call(|writer| {
            writer.connection()?;
            Ok(())
        })?;
        Ok(db)
    }
    pub(crate) fn connect(&self) -> Result<Connection, ResourceError> {
        check_database_path(&self.root)?;
        let db = Connection::open_with_flags(
            self.root.join(FILE_NAME),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", true)?;
        Ok(db)
    }
    pub(crate) fn information_seed(&self, source_id: &str, book_url: &str, kind: &str) -> Result<Option<Value>, ResourceError> {
        let json = self.connect()?.query_row("SELECT engine_json FROM books WHERE source_id=?1 AND kind=?3 AND coalesce(json_extract(engine_json,'$.bookUrl'),json_extract(engine_json,'$.url'))=?2 UNION ALL SELECT engine_json FROM alternative_sources WHERE source_id=?1 AND engine_url=?2 LIMIT 1",
            params![source_id,book_url,kind], |row|row.get::<_, Option<String>>(0)).optional()?.flatten();
        json.map(|json|serde_json::from_str(&json).map_err(|error|ResourceError::new(error.to_string()))).transpose()
    }
    pub(crate) fn information_id(&self, source_id: &str, book_url: &str, kind: &str) -> Result<Option<String>, ResourceError> {
        Ok(self.connect()?.query_row("SELECT id FROM books WHERE source_id=?1 AND kind=?3 AND coalesce(json_extract(engine_json,'$.bookUrl'),json_extract(engine_json,'$.url'))=?2 ORDER BY id LIMIT 1",
            params![source_id,book_url,kind], |row|row.get(0)).optional()?)
    }
    pub(crate) fn file_path(&self) -> PathBuf {
        self.root.join(FILE_NAME)
    }
    pub(crate) fn transaction<T: Send + 'static>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, ResourceError> + Send + 'static,
    ) -> Result<T, ResourceError> {
        self.writer.call(move |writer| {
            let tx = writer
                .connection()?
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let result = action(&tx)?;
            tx.commit()?;
            Ok(result)
        })
    }
    pub(crate) fn with_writer_closed<T: Send + 'static>(
        &self,
        action: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, ResourceError> {
        self.writer.call(move |writer| {
            writer.connection.take();
            Ok(action())
        })
    }
    pub(crate) fn exists(&self, path: &str) -> Result<bool, ResourceError> {
        if crate::configuration::is_file(path) {
            return Ok(crate::configuration::read(&self.root, path)?.is_some());
        }
        exists(&self.connect()?, path)
    }
    pub(crate) fn read(&self, path: &str) -> Result<Option<Value>, ResourceError> {
        if crate::configuration::is_file(path) {
            return crate::configuration::read(&self.root, path);
        }
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let value = read(&tx, path)?;
        tx.commit()?;
        Ok(value)
    }
    pub(crate) fn write(&self, path: &str, value: &Value) -> Result<(), ResourceError> {
        if crate::configuration::is_file(path) {
            return crate::configuration::write(&self.root, path, value.clone());
        }
        let (path, value) = (path.to_owned(), value.clone());
        self.transaction(move |db| write(db, &path, &value))
    }
    pub(crate) fn paths(&self) -> Result<Vec<String>, ResourceError> {
        let mut p = paths(&self.connect()?)?;
        if self.root.join("settings.json").is_file() {
            p.push("settings.json".into());
        }
        Ok(p)
    }
    pub(crate) fn content_path(&self, id: &str) -> Result<Option<String>, ResourceError> {
        let kind: Option<String> = self
            .connect()?
            .query_row("SELECT kind FROM books WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        Ok(kind.map(|kind| format!("cache/{kind}/{id}/{id}.json")))
    }
    pub(crate) fn delete_prefix(&self, prefix: &str) -> Result<(), ResourceError> {
        let parts = prefix.trim_end_matches('/').split('/').collect::<Vec<_>>();
        let ["cache", kind, id] = parts.as_slice() else {
            return Err(ResourceError::new("Invalid book cache namespace"));
        };
        let (id, kind) = ((*id).to_owned(), (*kind).to_owned());
        self.transaction(move |db| {
            db.execute(
                "DELETE FROM books WHERE id=?1 AND kind=?2",
                params![id, kind],
            )?;
            Ok(())
        })
    }
    pub(crate) fn set_media(
        &self,
        path: &str,
        id: &str,
        key: &str,
        value: Option<&Value>,
    ) -> Result<(), ResourceError> {
        let (path, id, key, value) = (
            path.to_owned(),
            id.to_owned(),
            key.to_owned(),
            value.cloned(),
        );
        self.transaction(move |db| {
            if !exists(db, &path)? {
                let kind = crate::resources::content::content_path(&path)
                    .map(|v| v.0)
                    .unwrap_or("html");
                books::replace(db, &path, kind, &json!({"id":id}))?;
            }
            books::set_media(db, &id, &key, value.as_ref())
        })
    }
    pub(crate) fn media(&self) -> Result<Vec<(String, Value)>, ResourceError> {
        books::all_media(&self.connect()?)
    }
    #[cfg(test)]
    pub(crate) fn preview(&self, id: &str) -> Result<Value, ResourceError> {
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let value = books::preview(&tx, id)?;
        tx.commit()?;
        Ok(value)
    }
    #[cfg(test)]
    pub(crate) fn save_chapter_view(
        &self,
        id: &str,
        chapter_id: &str,
        view: &Value,
    ) -> Result<(), ResourceError> {
        let (id, chapter_id, view) = (id.to_owned(), chapter_id.to_owned(), view.clone());
        self.transaction(move |db| books::save_chapter_view(db, &id, &chapter_id, &view))
    }

    /// 只更新对应章节的缓存字段，取消时保留原缓存。
    pub(crate) fn save_main_text(&self, id: String, chapter_id: String, view: Value, token: tokio_util::sync::CancellationToken) -> Result<(), ResourceError> {
        self.transaction(move |db| {
            if token.is_cancelled() { return Err(ResourceError::new("Chapter content cancelled")); }
            books::save_chapter_view(db, &id, &chapter_id, &view)
        })
    }
    pub(crate) fn save_library_positions(
        &self,
        shelf: crate::models::ShelfDocument,
    ) -> Result<(), ResourceError> {
        self.transaction(move |db| collections::save_positions(db, &shelf))
    }
    pub(crate) fn sync_sources(
        &self,
        changes: &[(usize, String, Option<Value>)],
        ids: &[String],
    ) -> Result<(), ResourceError> {
        let (changes, ids) = (changes.to_vec(), ids.to_vec());
        self.transaction(move |db| {
            for (position, id, value) in changes {
                if let Some(value) = value {
                    collections::save_source(db, position, &value)?;
                } else {
                    db.execute(
                        "UPDATE sources SET position=?1 WHERE id=?2",
                        params![position as i64, id],
                    )?;
                }
            }
            db.execute(
                "DELETE FROM sources WHERE id NOT IN (SELECT value FROM json_each(?1))",
                [serde_json::to_string(&ids).map_err(|e| ResourceError::new(e.to_string()))?],
            )?;
            Ok(())
        })
    }
    pub(crate) fn save_display(
        &self,
        path: &str,
        kind: &str,
        patch: &Value,
    ) -> Result<(), ResourceError> {
        let (path, kind, patch) = (path.to_owned(), kind.to_owned(), patch.clone());
        self.transaction(move |db|{let id=book_id(db,&path)?;let metadata=books::metadata(db,&id)?.ok_or_else(||ResourceError::new("Book is no longer available"))?;let mut view=crate::resources::content::book_view(&metadata,&kind)?;crate::book_display_metadata::apply_patch(&mut view,&patch).map_err(ResourceError::new)?;let o=&view["displayOverrides"];db.execute("UPDATE books SET title_override=?2,author_override=?3,intro_override=?4,cover_override=?5 WHERE id=?1",params![id,o["title"].as_str(),o["author"].as_str(),o["intro"].as_str(),o["coverSrc"].as_str()])?;Ok(())})
    }
    pub(crate) fn save_progress(
        &self,
        path: &str,
        kind: &str,
        progress: &Value,
    ) -> Result<Option<Value>, ResourceError> {
        let (path, kind, progress) = (path.to_owned(), kind.to_owned(), progress.clone());
        self.transaction(move |db| {
            let id = book_id(db, &path)?;
            let previous = books::progress(db, &id)?;
            if previous["updatedAtMs"].as_u64().unwrap_or(0)
                > progress["updatedAtMs"].as_u64().unwrap_or(0)
            {
                return Ok(None);
            }
            let index = progress["chapterIndex"].as_u64().unwrap_or(u64::MAX);
            let offset = progress["offset"].as_u64().unwrap_or(0);
            let (chapter, read) = if kind == "pdf" {
                let count: i64 = db.query_row(
                    "SELECT coalesce(page_count,0) FROM books WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )?;
                if index >= count as u64 {
                    return Err(ResourceError::new(
                        "Progress chapterIndex is outside the book chapter list",
                    ));
                }
                (json!({"id":format!("page-{}",index+1)}), index + 1)
            } else {
                let row: Option<(String, i64)> = db
                    .query_row(
                        "SELECT id,read_position FROM chapters WHERE book_id=?1 AND position=?2",
                        params![id, index as i64],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let (chapter, read) = row.ok_or_else(|| {
                    ResourceError::new("Progress chapterIndex is outside the book chapter list")
                })?;
                (books::chapter(db, &id, &chapter)?.unwrap(), read as u64)
            };
            if progress["chapterId"]
                .as_str()
                .is_some_and(|c| chapter["id"] != c)
            {
                return Err(ResourceError::new(
                    "Progress chapterId does not match chapterIndex",
                ));
            }
            for (field, collection, key) in
                [("quality", "sources", "label"), ("pageId", "pages", "id")]
            {
                if let Some(v) = progress[field].as_str() {
                    if !rows::array(&chapter, collection)
                        .iter()
                        .any(|r| r[key] == v)
                    {
                        return Err(ResourceError::new(
                            "Progress position does not belong to the chapter",
                        ));
                    }
                }
            }
            if kind == "image" && offset > 1_000_000 {
                return Err(ResourceError::new(
                    "Image position ratio is outside the page",
                ));
            }
            let mut stored = progress.clone();
            stored
                .as_object_mut()
                .ok_or_else(|| ResourceError::new("Invalid progress"))?
                .remove("offset");
            stored["chapterId"] = chapter["id"].clone();
            match kind.as_str() {
                "audio" | "video" => stored["timeMs"] = json!(offset),
                "image" => stored["offsetRatio"] = json!(offset as f64 / 1_000_000.0),
                "pdf" => {
                    stored.as_object_mut().unwrap().remove("chapterId");
                    stored["page"] = json!(index + 1);
                }
                _ => stored["charOffset"] = json!(offset),
            }
            books::write_progress(db, &id, &stored)?;
            let changed = if kind == "pdf" {
                previous["page"].as_u64() != Some(index + 1)
            } else {
                previous["chapterId"] != chapter["id"]
            };
            let mut runtime = progress;
            runtime["chapterId"] = if kind == "pdf" {
                Value::Null
            } else {
                chapter["id"].clone()
            };
            Ok(Some(
                json!({"progress":runtime,"read":read,"chapterChanged":changed}),
            ))
        })
    }
}
fn book_id(db: &Connection, path: &str) -> Result<String, ResourceError> {
    let (kind, id, _) = crate::resources::content::content_path(path)
        .ok_or_else(|| ResourceError::new("Invalid book identity"))?;
    db.query_row(
        "SELECT id FROM books WHERE id=?1 AND kind=?2",
        params![id, kind],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| ResourceError::new("Book does not exist"))
}
fn connection_root(db: &Connection) -> Result<&Path, ResourceError> {
    db.path()
        .and_then(|p| Path::new(p).parent())
        .ok_or_else(|| ResourceError::new("Database has no data root"))
}
pub(crate) fn exists(db: &Connection, path: &str) -> Result<bool, ResourceError> {
    if crate::configuration::is_file(path) {
        return Ok(crate::configuration::read(connection_root(db)?, path)?.is_some());
    }
    if ROOTS.contains(&path) {
        return Ok(true);
    }
    if crate::resources::content::is_content_document(path) {
        let (kind, id, _) = crate::resources::content::content_path(path).unwrap();
        return Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE id=?1 AND kind=?2)",
            params![id, kind],
            |r| r.get(0),
        )?);
    }
    if let Some((id, private, page)) = discovery::address(path) {
        return Ok(discovery::read(db, id, private, page)?.is_some());
    }
    Ok(false)
}
pub(crate) fn read(db: &Connection, path: &str) -> Result<Option<Value>, ResourceError> {
    if crate::configuration::is_file(path) {
        return crate::configuration::read(connection_root(db)?, path);
    }
    if crate::resources::content::is_content_document(path) {
        let (kind, id, _) = crate::resources::content::content_path(path).unwrap();
        let id: Option<String> = db
            .query_row(
                "SELECT id FROM books WHERE id=?1 AND kind=?2",
                params![id, kind],
                |r| r.get(0),
            )
            .optional()?;
        return id.map(|id| books::read(db, &id)).unwrap_or(Ok(None));
    }
    if let Some((id, private, page)) = discovery::address(path) {
        return discovery::read(db, id, private, page);
    }
    Ok(Some(match path {
        "sources.json" => collections::sources(db)?,
        "content-library.json" => collections::library(db)?,
        "history.json" => history::read(db)?,
        "tasks.json" => tasks::read(db)?,
        _ => return collections::read(db, path),
    }))
}
pub(crate) fn write(db: &Connection, path: &str, value: &Value) -> Result<(), ResourceError> {
    if crate::configuration::is_file(path) {
        return crate::configuration::write(connection_root(db)?, path, value.clone());
    }
    if crate::resources::content::is_content_document(path) {
        let (kind, id, _) = crate::resources::content::content_path(path).unwrap();
        if value["id"] != id {
            return Err(ResourceError::new("Book ID differs from resource address"));
        }
        return books::replace(db, path, kind, value);
    }
    if let Some((id, private, page)) = discovery::address(path) {
        return discovery::write(db, id, private, page, value);
    }
    match path {
        "sources.json" => {
            db.execute("DELETE FROM sources", [])?;
            for (position, value) in value.as_array().into_iter().flatten().enumerate() {
                collections::save_source(db, position, value)?;
            }
        }
        "content-library.json" => collections::save_library(db, value)?,
        "history.json" => history::write(db, value)?,
        "tasks.json" => tasks::write(db, value)?,
        _ => {
            if !collections::write(db, path, value)? {
                return Err(ResourceError::new(format!(
                    "Unknown database resource: {path}"
                )));
            }
        }
    }
    Ok(())
}
pub(crate) fn delete(db: &Connection, path: &str) -> Result<(), ResourceError> {
    if crate::resources::content::is_content_document(path) {
        let (kind, id, _) = crate::resources::content::content_path(path).unwrap();
        db.execute(
            "DELETE FROM books WHERE id=?1 AND kind=?2",
            params![id, kind],
        )?;
        return Ok(());
    }
    if let Some((id, private, page)) = discovery::address(path) {
        if page {
            db.execute("DELETE FROM discovery_pages WHERE id=?1", [id])?;
        } else {
            db.execute(
                "DELETE FROM discovery_sources WHERE source_id=?1 AND private=?2",
                params![id, private],
            )?;
        }
        return Ok(());
    }
    Err(ResourceError::new("Resource does not support deletion"))
}
pub(crate) fn paths(db: &Connection) -> Result<Vec<String>, ResourceError> {
    let mut paths = ROOTS.iter().map(|s| (*s).into()).collect::<Vec<String>>();
    let mut stmt=db.prepare("SELECT 'cache/'||kind||'/'||id||'/'||id||'.json' FROM books UNION ALL SELECT 'discovery/'||id||'.json' FROM discovery_pages UNION ALL SELECT CASE private WHEN 1 THEN 'source-engine/discovery-categories/' ELSE 'discovery/' END||source_id||'.json' FROM discovery_sources")?;
    paths.extend(
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(paths)
}
pub(crate) fn book_preview(db: &Connection, id: &str) -> Result<(String, Value), ResourceError> {
    books::library_preview(db, id)
}
pub(crate) fn source_database(data_dir: &Path) -> Result<Database, ResourceError> {
    let root = if data_dir.file_name().is_some_and(|n| n == "source-engine") {
        data_dir.parent().unwrap_or(data_dir)
    } else {
        data_dir
    };
    Database::open(root)
}
#[cfg(test)]
#[path = "database_tests.rs"]
mod tests;
