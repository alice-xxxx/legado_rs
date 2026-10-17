//! SQLite owns mutable state. Resource IDs remain stable API addresses, not files.
//!
//! Collections are records, not one large JSON blob: changing progress or one
//! media mapping never reads, serializes or writes the chapter catalogue.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::resources::ResourceError;

pub(crate) const FILE_NAME: &str = "app.sqlite3";
const ROOT_ITEMS: &str = "";
type Writer = Mutex<Option<Connection>>;
static WRITERS: OnceLock<Mutex<HashMap<PathBuf, Weak<Writer>>>> = OnceLock::new();

impl From<rusqlite::Error> for ResourceError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(format!("Database operation failed: {error}"))
    }
}

fn decode(text: &str) -> Result<Value, ResourceError> {
    serde_json::from_str(text).map_err(|error| ResourceError::new(error.to_string()))
}
fn encode(value: &Value) -> Result<String, ResourceError> {
    serde_json::to_string(value).map_err(|error| ResourceError::new(error.to_string()))
}

/// One shared writer connection per database, separate snapshot readers.
/// The writer mutex protects the connection only during SQL work; it never
/// covers networking or file publication. Restore explicitly drains the writer.
#[derive(Clone)]
pub(crate) struct Database {
    root: PathBuf,
    writer: Arc<Writer>,
}

impl Database {
    pub(crate) fn at(root: &Path) -> Self {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
        let mut writers = WRITERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if writers.len() > 64 {
            writers.retain(|_, writer| writer.strong_count() > 0);
        }
        let writer = writers
            .get(&root)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let writer = Arc::new(Mutex::new(None));
                writers.insert(root.clone(), Arc::downgrade(&writer));
                writer
            });
        Self { root, writer }
    }

    pub(crate) fn open(root: &Path) -> Result<Self, ResourceError> {
        std::fs::create_dir_all(root)?;
        let db = Self::at(root);
        let mut connection = db.connect()?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
            return Err(ResourceError::new(
                "Database was created by a newer application",
            ));
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        if version == 0 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS documents (
                    path TEXT PRIMARY KEY, shape INTEGER NOT NULL, value TEXT,
                    book_id TEXT UNIQUE, content_type TEXT,
                    owner_path TEXT, owner_field TEXT,
                    FOREIGN KEY(owner_path,owner_field) REFERENCES fields(path,name) ON DELETE CASCADE
                 ) WITHOUT ROWID;
                 CREATE INDEX IF NOT EXISTS document_owner ON documents(owner_path,owner_field);
                 CREATE TABLE IF NOT EXISTS fields (
                    path TEXT NOT NULL REFERENCES documents(path) ON DELETE CASCADE,
                    name TEXT NOT NULL, shape INTEGER NOT NULL, value TEXT,
                    PRIMARY KEY(path,name)
                 ) WITHOUT ROWID;
                 CREATE TABLE IF NOT EXISTS records (
                    path TEXT NOT NULL, field TEXT NOT NULL, key TEXT NOT NULL,
                    position INTEGER NOT NULL, read_position INTEGER NOT NULL, item_id TEXT, value TEXT NOT NULL,
                    PRIMARY KEY(path,field,key),
                    FOREIGN KEY(path,field) REFERENCES fields(path,name) ON DELETE CASCADE
                 ) WITHOUT ROWID;
                 CREATE INDEX IF NOT EXISTS record_order ON records(path,field,position);
                 CREATE INDEX IF NOT EXISTS record_identity ON records(path,field,item_id);
                 CREATE TABLE IF NOT EXISTS source_kv (
                    namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
                    expires_at_ms INTEGER, PRIMARY KEY(namespace,key)
                 ) WITHOUT ROWID;
                 CREATE INDEX IF NOT EXISTS source_kv_expiry ON source_kv(expires_at_ms)
                    WHERE expires_at_ms IS NOT NULL;
                 CREATE TABLE IF NOT EXISTS pending_cleanup (path TEXT PRIMARY KEY) WITHOUT ROWID;
                 PRAGMA user_version=1;"
            )?;
            tx.commit()?;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(i64::MAX as u128) as i64;
        connection.execute("DELETE FROM source_kv WHERE expires_at_ms<=?1", [now])?;
        {
            let mut writer = db
                .writer
                .lock()
                .map_err(|_| ResourceError::new("Database writer is poisoned"))?;
            if writer.is_none() {
                *writer = Some(connection);
            }
        }
        Ok(db)
    }

    pub(crate) fn connect(&self) -> Result<Connection, ResourceError> {
        for name in [
            FILE_NAME.to_owned(),
            format!("{FILE_NAME}-wal"),
            format!("{FILE_NAME}-shm"),
        ] {
            crate::resources::check_path_no_symlink(&self.root, &self.root.join(name), true)?;
        }
        let connection = Connection::open(self.root.join(FILE_NAME))?;
        connection.busy_timeout(Duration::from_secs(10))?;
        // Keep WAL state between short-lived connections. Closing the final
        // reader must not checkpoint/unlink WAL while another worker opens it.
        // Commit-time auto-checkpointing still bounds the WAL size.
        connection.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )?;
        connection.pragma_update(None, "foreign_keys", true)?;
        Ok(connection)
    }

    pub(crate) fn exists(&self, path: &str) -> Result<bool, ResourceError> {
        exists(&self.connect()?, path)
    }
    pub(crate) fn read(&self, path: &str) -> Result<Option<Value>, ResourceError> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let value = read(&tx, path)?;
        tx.commit()?;
        Ok(value)
    }
    pub(crate) fn write(&self, path: &str, value: &Value) -> Result<(), ResourceError> {
        self.transaction(|tx| write(tx, path, value))
    }
    pub(crate) fn transaction<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, ResourceError>,
    ) -> Result<T, ResourceError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| ResourceError::new("Database writer is poisoned"))?;
        if writer.is_none() {
            *writer = Some(self.connect()?);
        }
        let tx = writer
            .as_mut()
            .expect("initialized writer")
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = action(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    /// Keep the writer drained while a validated backup replaces the root.
    /// No connection remains attached to the old database after the rename.
    pub(crate) fn with_writer_closed<T>(
        &self,
        action: impl FnOnce() -> T,
    ) -> Result<T, ResourceError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| ResourceError::new("Database writer is poisoned"))?;
        writer.take();
        Ok(action())
    }
    pub(crate) fn paths(&self) -> Result<Vec<String>, ResourceError> {
        let connection = self.connect()?;
        let mut stmt = connection
            .prepare("SELECT path FROM documents WHERE owner_path IS NULL ORDER BY path")?;
        let values = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(values)
    }
    pub(crate) fn content_path(&self, id: &str) -> Result<Option<String>, ResourceError> {
        let connection = self.connect()?;
        Ok(connection
            .query_row("SELECT path FROM documents WHERE book_id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }
    pub(crate) fn delete_prefix(&self, prefix: &str) -> Result<(), ResourceError> {
        self.transaction(|tx| {
            tx.execute(
                "DELETE FROM documents WHERE substr(path,1,length(?1))=?1",
                [prefix],
            )?;
            Ok(())
        })
    }
    pub(crate) fn set_media(
        &self,
        path: &str,
        book_id: &str,
        key: &str,
        value: Option<&Value>,
    ) -> Result<(), ResourceError> {
        self.transaction(|tx| {
            if !exists(tx, path)? {
                write(tx, path, &json!({"id": book_id}))?;
            }
            set_collection(tx, path, "media", 2)?;
            if let Some(value) = value {
                upsert_record(tx, path, "media", key, 0, 0, None, value)?;
            } else {
                tx.execute(
                    "DELETE FROM records WHERE path=?1 AND field='media' AND key=?2",
                    params![path, key],
                )?;
            }
            Ok(())
        })
    }

    /// Source discovery updates only alternatives, never reads/rebuilds the catalogue.
    pub(crate) fn merge_alternatives(&self, path: &str, candidates: Vec<Value>) -> Result<bool, ResourceError> {
        if candidates.is_empty() { return Ok(false); }
        self.transaction(|tx| {
            if !exists(tx, path)? { return Err(ResourceError::new("Book does not exist")); }
            let mut alternatives = field(tx, path, "alternatives")?.as_array().cloned().unwrap_or_default();
            let mut changed = false;
            for candidate in candidates {
                if let Some(existing) = alternatives.iter_mut().find(|old|
                    old["sourceId"] == candidate["sourceId"] && old["engineurl"] == candidate["engineurl"]) {
                    if *existing != candidate { *existing = candidate; changed = true; }
                } else {
                    alternatives.push(candidate);
                    changed = true;
                }
            }
            if changed { write_field(tx, path, "alternatives", &json!(alternatives))?; }
            Ok(changed)
        })
    }
    pub(crate) fn media(&self) -> Result<Vec<(String, Value)>, ResourceError> {
        let connection = self.connect()?;
        let mut stmt = connection.prepare("SELECT key,value FROM records WHERE field='media' AND path LIKE 'cache/%' ORDER BY path,key")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (key, value) = row?;
            Ok((key, decode(&value)?))
        })
        .collect()
    }

    pub(crate) fn save_progress(
        &self,
        path: &str,
        kind: &str,
        progress: &Value,
    ) -> Result<Option<Value>, ResourceError> {
        self.transaction(|tx| {
            if !exists(tx,path)? { return Err(ResourceError::new("Book does not exist")); }
            let previous = field(tx,path,"progress")?;
            let timestamp = progress["updatedAtMs"].as_u64().unwrap_or(0);
            if previous["updatedAtMs"].as_u64().unwrap_or(0)>timestamp { return Ok(None); }
            let index = progress["chapterIndex"].as_u64().unwrap_or(u64::MAX) as usize;
            let offset = progress["offset"].as_u64().unwrap_or(0);
            let chapter = if kind == "pdf" {
                if index >= field(tx,path,"pageCount")?.as_u64().unwrap_or(0) as usize { return Err(ResourceError::new("Progress chapterIndex is outside the book chapter list")); }
                json!({"id": format!("page-{}",index+1)})
            } else { record_at(tx,path,"chapters",index)?.ok_or_else(||ResourceError::new("Progress chapterIndex is outside the book chapter list"))? };
            if let Some(id) = progress["chapterId"].as_str() {
                if chapter["id"] != id { return Err(ResourceError::new("Progress chapterId does not match chapterIndex")); }
            }
            if let Some(quality) = progress["quality"].as_str() {
                if !chapter["sources"].as_array().is_some_and(|sources|sources.iter().any(|source|source["label"]==quality)) { return Err(ResourceError::new("Video quality does not belong to the chapter")); }
            }
            if let Some(page_id) = progress["pageId"].as_str() {
                if !chapter["pages"].as_array().is_some_and(|pages|pages.iter().any(|page|page["id"]==page_id)) { return Err(ResourceError::new("Progress image page does not belong to the chapter")); }
            }
            if kind=="image" && offset>1_000_000 { return Err(ResourceError::new("Image position ratio is outside the page")); }
            let mut stored = progress.as_object().cloned().unwrap_or_default();
            if kind!="pdf" { stored.insert("chapterId".into(),chapter["id"].clone()); }
            stored.remove("chapterIndex");
            stored.remove("offset");
            match kind {
                "audio"|"video" => { stored.insert("timeMs".into(),json!(offset)); }
                "image" => { stored.insert("offsetRatio".into(),json!(offset as f64/1_000_000.0)); }
                "pdf" => { stored.remove("chapterId"); stored.insert("page".into(),json!(index+1)); }
                _ => { stored.insert("charOffset".into(),json!(offset)); }
            }
            set_scalar(tx,path,"progress",&Value::Object(stored))?;
            // Count only preceding non-volume chapters for the shelf's read count.
            let read = if kind=="pdf" { index+1 } else {
                tx.query_row("SELECT read_position FROM records WHERE path=?1 AND field='chapters' AND position=?2",params![path,index as i64],|row|row.get::<_,i64>(0))? as usize
            };
            let id = field(tx,path,"id")?;
            tx.execute("UPDATE records SET value=json_set(value,'$.readAt',?1,'$.read',?2) WHERE path=?3 AND field=?4 AND item_id=?5",
                params![(timestamp/1000) as i64,read as i64,crate::resources::content::LIBRARY_FILE,kind,id.as_str()])?;
            let chapter_changed = if kind=="pdf" { previous["page"].as_u64()!=Some(index as u64+1) }
                else { previous["chapterId"]!=chapter["id"] };
            let mut runtime = progress.clone();
            runtime["chapterId"] = if kind=="pdf" { Value::Null } else { chapter["id"].clone() };
            Ok(Some(json!({"progress":runtime,"read":read,"chapterChanged":chapter_changed})))
        })
    }

    pub(crate) fn sync_sources(
        &self,
        changes: &[(usize, String, Option<Value>)],
        ids: &[String],
    ) -> Result<(), ResourceError> {
        self.transaction(|tx| {
            if !exists(tx,"sources.json")? { write(tx,"sources.json",&json!([]))?; }
            for (position,id,value) in changes {
                let key=format!("id:{id}");
                if let Some(value)=value { upsert_record(tx,"sources.json",ROOT_ITEMS,&key,*position,0,Some(id),value)?; }
                else { tx.execute("UPDATE records SET position=?1 WHERE path='sources.json' AND field='' AND key=?2",params![*position as i64,key])?; }
            }
            let keys=ids.iter().map(|id|format!("id:{id}")).collect::<Vec<_>>();
            tx.execute("DELETE FROM records WHERE path='sources.json' AND field='' AND key NOT IN (SELECT value FROM json_each(?1))",[serde_json::to_string(&keys).unwrap()])?;
            Ok(())
        })
    }

    pub(crate) fn save_display(
        &self,
        path: &str,
        kind: &str,
        patch: &Value,
    ) -> Result<(), ResourceError> {
        self.transaction(|tx| {
            if !exists(tx,path)? { return Err(ResourceError::new("Book is no longer available")); }
            let mut metadata=Map::new();
            for name in ["id","title","author","intro","cover","overrides"] {
                let value=field(tx,path,name)?;
                if !value.is_null() {metadata.insert(name.into(),value);}
            }
            let mut view=crate::resources::content::book_view(&Value::Object(metadata),kind)?;
            crate::book_display_metadata::apply_patch(&mut view,patch).map_err(ResourceError::new)?;
            if let Some(overrides)=view.get("displayOverrides") {set_scalar(tx,path,"overrides",overrides)?;}
            else {tx.execute("DELETE FROM fields WHERE path=?1 AND name='overrides'",[path])?;}
            let id=view["id"].as_str().ok_or_else(||ResourceError::new("Book has no ID"))?;
            let (_,preview)=book_preview(tx,id)?;
            tx.execute("UPDATE records SET value=?1 WHERE path=?2 AND field=?3 AND item_id=?4 AND value!=?1",params![preview.to_string(),crate::resources::content::LIBRARY_FILE,kind,id])?;
            Ok(())
        })
    }

    pub(crate) fn reset_progress(&self, path: &str) -> Result<(), ResourceError> {
        self.transaction(|tx| {
            let id=field(tx,path,"id")?;
            if !id.is_string() {return Err(ResourceError::new("Book is no longer available"));}
            tx.execute("DELETE FROM fields WHERE path=?1 AND name='progress'",[path])?;
            tx.execute("UPDATE records SET value=json_remove(value,'$.readAt','$.read') WHERE path=?1 AND item_id=?2",params![crate::resources::content::LIBRARY_FILE,id.as_str()])?;
            Ok(())
        })
    }

    pub(crate) fn preview(&self, id: &str) -> Result<Value, ResourceError> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let (kind, item) = book_preview(&tx, id)?;
        let path: String =
            tx.query_row("SELECT path FROM documents WHERE book_id=?1", [id], |row| {
                row.get(0)
            })?;
        let mut metadata = Map::new();
        for name in [
            "id",
            "title",
            "author",
            "cover",
            "overrides",
            "tags",
            "sourceId",
            "sourceName",
            "progress",
        ] {
            let value = field(&tx, &path, name)?;
            if !value.is_null() {
                metadata.insert(name.into(), value);
            }
        }
        let engine: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM fields WHERE path=?1 AND name='engine')",
            [&path],
            |row| row.get(0),
        )?;
        if engine {
            metadata.insert("engine".into(), json!({}));
        }
        let mut view =
            crate::resources::content::public_book_view(&Value::Object(metadata), &kind)?;
        if let Some(object) = view.as_object_mut() {
            object.remove("chapters");
            object.remove("displayBase");
            object.remove("displayOverrides");
        }
        view["chapterCount"] = if kind == "pdf" {
            field(&tx, &path, "pageCount")?
        } else {
            json!(tx.query_row("SELECT position+1 FROM records WHERE path=?1 AND field='chapters' ORDER BY position DESC LIMIT 1",[&path],|row|row.get::<_,i64>(0)).optional()?.unwrap_or(0))
        };
        view["latestChapter"] = item["latest"].clone();
        let index = if kind == "pdf" {
            field(&tx, &path, "progress")?["page"]
                .as_u64()
                .unwrap_or(1)
                .saturating_sub(1)
        } else {
            tx.query_row(
                "SELECT position FROM records WHERE path=?1 AND field='chapters' AND item_id=?2",
                params![path, view["progress"]["chapterId"].as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u64
        };
        view["progress"]["chapterIndex"] = json!(index);
        tx.commit()?;
        Ok(view)
    }
}

pub(crate) fn exists(connection: &Connection, path: &str) -> Result<bool, ResourceError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM documents WHERE path=?1)",
        [path],
        |row| row.get(0),
    )?)
}

pub(crate) fn read(connection: &Connection, path: &str) -> Result<Option<Value>, ResourceError> {
    let root: Option<(i64, Option<String>)> = connection
        .query_row(
            "SELECT shape,value FROM documents WHERE path=?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((shape, scalar)) = root else {
        return Ok(None);
    };
    if shape == 0 {
        return decode(scalar.as_deref().unwrap_or("null")).map(Some);
    }
    let mut fields =
        connection.prepare("SELECT name,shape,value FROM fields WHERE path=?1 ORDER BY name")?;
    let rows = fields.query_map([path], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut object = Map::new();
    for row in rows {
        let (name, kind, scalar) = row?;
        object.insert(
            name.clone(),
            read_field_value(connection, path, &name, kind, scalar.as_deref())?,
        );
    }
    Ok(Some(if shape == 2 {
        object.remove(ROOT_ITEMS).unwrap_or_else(|| json!([]))
    } else {
        Value::Object(object)
    }))
}

fn read_field_value(
    connection: &Connection,
    path: &str,
    field: &str,
    shape: i64,
    scalar: Option<&str>,
) -> Result<Value, ResourceError> {
    if shape == 0 {
        return decode(scalar.unwrap_or("null"));
    }
    if shape == 3 {
        return read(
            connection,
            scalar.ok_or_else(|| ResourceError::new("Object record has no address"))?,
        )?
        .ok_or_else(|| ResourceError::new("Object record is missing"));
    }
    let mut stmt = connection.prepare(
        "SELECT key,value FROM records WHERE path=?1 AND field=?2 ORDER BY position,key",
    )?;
    let rows = stmt.query_map(params![path, field], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut array = Vec::new();
    let mut object = Map::new();
    for row in rows {
        let (key, value) = row?;
        let value = decode(&value)?;
        if shape == 1 {
            array.push(value);
        } else {
            object.insert(key, value);
        }
    }
    Ok(if shape == 1 {
        Value::Array(array)
    } else {
        Value::Object(object)
    })
}

pub(crate) fn field(
    connection: &Connection,
    path: &str,
    name: &str,
) -> Result<Value, ResourceError> {
    let row: Option<(i64, Option<String>)> = connection
        .query_row(
            "SELECT shape,value FROM fields WHERE path=?1 AND name=?2",
            params![path, name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((shape, value)) => read_field_value(connection, path, name, shape, value.as_deref()),
        None => Ok(Value::Null),
    }
}
pub(crate) fn record_at(
    connection: &Connection,
    path: &str,
    field: &str,
    index: usize,
) -> Result<Option<Value>, ResourceError> {
    let text: Option<String> = connection
        .query_row(
            "SELECT value FROM records WHERE path=?1 AND field=?2 AND position=?3",
            params![path, field, index as i64],
            |row| row.get(0),
        )
        .optional()?;
    text.map(|text| decode(&text)).transpose()
}
pub(crate) fn set_scalar(
    connection: &Connection,
    path: &str,
    name: &str,
    value: &Value,
) -> Result<(), ResourceError> {
    connection.execute(
        "DELETE FROM documents WHERE owner_path=?1 AND owner_field=?2",
        params![path, name],
    )?;
    connection.execute(
        "DELETE FROM records WHERE path=?1 AND field=?2",
        params![path, name],
    )?;
    connection.execute("INSERT INTO fields(path,name,shape,value) VALUES(?1,?2,0,?3) ON CONFLICT(path,name) DO UPDATE SET shape=0,value=excluded.value WHERE fields.shape!=0 OR fields.value IS NOT excluded.value",params![path,name,encode(value)?])?;
    Ok(())
}
fn set_collection(
    connection: &Connection,
    path: &str,
    name: &str,
    shape: i64,
) -> Result<(), ResourceError> {
    connection.execute(
        "DELETE FROM documents WHERE owner_path=?1 AND owner_field=?2",
        params![path, name],
    )?;
    connection.execute("INSERT INTO fields(path,name,shape,value) VALUES(?1,?2,?3,NULL) ON CONFLICT(path,name) DO UPDATE SET shape=excluded.shape,value=NULL WHERE fields.shape!=excluded.shape OR fields.value IS NOT NULL",params![path,name,shape])?;
    Ok(())
}
fn upsert_record(
    connection: &Connection,
    path: &str,
    field: &str,
    key: &str,
    position: usize,
    read_position: usize,
    id: Option<&str>,
    value: &Value,
) -> Result<(), ResourceError> {
    connection.execute("INSERT INTO records(path,field,key,position,read_position,item_id,value) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(path,field,key) DO UPDATE SET position=excluded.position,read_position=excluded.read_position,item_id=excluded.item_id,value=excluded.value WHERE records.position!=excluded.position OR records.read_position!=excluded.read_position OR records.item_id IS NOT excluded.item_id OR records.value!=excluded.value",params![path,field,key,position as i64,read_position as i64,id,encode(value)?])?;
    Ok(())
}
fn write_field(
    connection: &Connection,
    path: &str,
    name: &str,
    value: &Value,
) -> Result<(), ResourceError> {
    if let Some(items) = value.as_array() {
        set_collection(connection, path, name, 1)?;
        let mut keys = Vec::with_capacity(items.len());
        let mut seen = HashSet::new();
        let mut read_position = 0;
        for (index, value) in items.iter().enumerate() {
            let id = value["id"].as_str();
            let key = id
                .filter(|id| seen.insert((*id).to_owned()))
                .map(|id| format!("id:{id}"))
                .unwrap_or_else(|| format!("index:{index}"));
            if value["volume"] != true {
                read_position += 1;
            }
            upsert_record(
                connection,
                path,
                name,
                &key,
                index,
                read_position,
                id,
                value,
            )?;
            keys.push(key);
        }
        connection.execute("DELETE FROM records WHERE path=?1 AND field=?2 AND key NOT IN (SELECT value FROM json_each(?3))",params![path,name,serde_json::to_string(&keys).unwrap()])?;
    } else if name == "media" && value.is_object() {
        set_collection(connection, path, name, 2)?;
        let mut keys = Vec::new();
        let independent = crate::resources::content::is_content_document(path);
        for (key, value) in value.as_object().unwrap() {
            if independent {
                connection.execute("INSERT OR IGNORE INTO records(path,field,key,position,read_position,value) VALUES(?1,?2,?3,0,0,?4)",params![path,name,key,encode(value)?])?;
            } else {
                upsert_record(connection, path, name, key, 0, 0, None, value)?;
            }
            keys.push(key);
        }
        if !independent {
            connection.execute("DELETE FROM records WHERE path=?1 AND field=?2 AND key NOT IN (SELECT value FROM json_each(?3))",params![path,name,serde_json::to_string(&keys).unwrap()])?;
        }
    } else if value.is_object()
        && !matches!(
            name,
            "progress" | "overrides" | "engine" | "engineState" | "engineData"
        )
    {
        // Structural maps (history, bookmarks, RSS state, settings) also split
        // into fields and ordered records. Source/chapter engine payloads stay
        // opaque within their own individual record.
        let child = format!(
            "\0{:x}",
            Sha256::digest(serde_json::to_vec(&(path, name)).unwrap())
        );
        connection.execute(
            "DELETE FROM records WHERE path=?1 AND field=?2",
            params![path, name],
        )?;
        connection.execute("INSERT INTO fields(path,name,shape,value) VALUES(?1,?2,3,?3) ON CONFLICT(path,name) DO UPDATE SET shape=3,value=excluded.value WHERE fields.shape!=3 OR fields.value!=excluded.value",params![path,name,child])?;
        write_owned(connection, &child, value, Some((path, name)))?;
    } else {
        if name == "progress"
            && crate::resources::content::is_content_document(path)
            && field(connection, path, name)?["updatedAtMs"]
                .as_u64()
                .unwrap_or(0)
                > value["updatedAtMs"].as_u64().unwrap_or(0)
        {
            return Ok(());
        }
        set_scalar(connection, path, name, value)?;
    }
    Ok(())
}

pub(crate) fn write(
    connection: &Connection,
    path: &str,
    value: &Value,
) -> Result<(), ResourceError> {
    write_owned(connection, path, value, None)
}

fn write_owned(
    connection: &Connection,
    path: &str,
    value: &Value,
    owner: Option<(&str, &str)>,
) -> Result<(), ResourceError> {
    let shape = if value.is_object() {
        1
    } else if value.is_array() {
        2
    } else {
        0
    };
    let scalar = if shape == 0 {
        Some(encode(value)?)
    } else {
        None
    };
    let identity = crate::resources::content::content_path(path)
        .filter(|_| crate::resources::content::is_content_document(path));
    let (kind, id) = identity
        .map(|(kind, id, _)| (Some(kind), Some(id)))
        .unwrap_or((None, None));
    if let Some(id) = id {
        if value["id"].as_str() != Some(id) {
            return Err(ResourceError::new(
                "Book resource ID does not match its path",
            ));
        }
    }
    connection.execute("INSERT INTO documents(path,shape,value,book_id,content_type,owner_path,owner_field) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(path) DO UPDATE SET shape=excluded.shape,value=excluded.value WHERE documents.shape!=excluded.shape OR documents.value IS NOT excluded.value",params![path,shape,scalar,id,kind,owner.map(|owner|owner.0),owner.map(|owner|owner.1)])?;
    let mut names = Vec::new();
    if let Some(fields) = value.as_object() {
        for (name, value) in fields {
            write_field(connection, path, name, value)?;
            names.push(name.as_str());
        }
    } else if shape == 2 {
        write_field(connection, path, ROOT_ITEMS, value)?;
        names.push(ROOT_ITEMS);
    }
    if crate::resources::content::is_content_document(path) {
        // Independently committed progress and media cannot be erased by a
        // catalogue snapshot captured before those operations completed.
        for name in ["media", "progress"] {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    connection.execute(
        "DELETE FROM fields WHERE path=?1 AND name NOT IN (SELECT value FROM json_each(?2))",
        params![path, serde_json::to_string(&names).unwrap()],
    )?;
    Ok(())
}

/// A shelf preview reads metadata and at most the latest chapter. It never
/// materializes every chapter just to list a book or save library membership.
pub(crate) fn book_preview(
    connection: &Connection,
    id: &str,
) -> Result<(String, Value), ResourceError> {
    let (path, kind): (String, String) = connection
        .query_row(
            "SELECT path,content_type FROM documents WHERE book_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| ResourceError::new("Library content is not cached"))?;
    let mut stmt=connection.prepare("SELECT name,value FROM fields WHERE path=?1 AND shape=0 AND name IN ('id','title','author','cover','updatedAtMs','pageCount','progress','overrides')")?;
    let rows = stmt.query_map([&path], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut metadata = Map::new();
    for row in rows {
        let (key, value) = row?;
        metadata.insert(key, decode(&value)?);
    }
    let document = Value::Object(metadata);
    let mut item = Map::new();
    for key in ["id", "title", "author", "cover"] {
        if let Some(value) = document.get(key) {
            item.insert(key.into(), value.clone());
        }
    }
    let latest: Option<(i64,String)>=connection.query_row("SELECT read_position,value FROM records WHERE path=?1 AND field='chapters' AND coalesce(json_extract(value,'$.volume'),0)!=1 ORDER BY position DESC LIMIT 1",[&path],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let total = if kind == "pdf" {
        document["pageCount"].as_u64().unwrap_or(0)
    } else {
        latest.as_ref().map_or(0, |(position, _)| *position as u64)
    };
    item.insert("total".into(), json!(total));
    if let Some((_, latest)) = latest {
        if let Some(title) = decode(&latest)?.get("title") {
            item.insert("latest".into(), title.clone());
        }
    }
    if let Some(timestamp) = document["progress"]["updatedAtMs"]
        .as_u64()
        .filter(|timestamp| *timestamp > 0)
    {
        item.insert("readAt".into(), json!(timestamp / 1000));
        let read = if kind == "pdf" {
            document["progress"]["page"].as_u64().unwrap_or(1)
        } else {
            connection.query_row("SELECT read_position FROM records WHERE path=?1 AND field='chapters' AND item_id=?2",params![path,document["progress"]["chapterId"].as_str()],|row|row.get::<_,i64>(0)).optional()?.unwrap_or(1) as u64
        };
        item.insert("read".into(), json!(read));
    }
    if let Some(timestamp) = document["updatedAtMs"]
        .as_u64()
        .filter(|timestamp| *timestamp > 0)
    {
        item.insert("updatedAt".into(), json!(timestamp / 1000));
    }
    if let Some(overrides) = document["overrides"].as_object() {
        for key in ["title", "author"] {
            if let Some(value) = overrides.get(key) {
                item.insert(key.into(), value.clone());
            }
        }
        if let Some(cover) = overrides.get("coverSrc").and_then(Value::as_str) {
            item.insert(
                "cover".into(),
                json!(crate::resources::content::stored_url(cover, id)),
            );
        }
    }
    item.retain(|_, value| !value.is_null() && value.as_str().is_none_or(|text| !text.is_empty()));
    Ok((kind, Value::Object(item)))
}

/// Map the engine's data directory to the one application database.
pub(crate) fn source_database(data_dir: &Path) -> Result<Database, ResourceError> {
    let root = if data_dir
        .file_name()
        .is_some_and(|name| name == "source-engine")
    {
        data_dir.parent().unwrap_or(data_dir)
    } else {
        data_dir
    };
    if root.join(FILE_NAME).is_file() {
        Ok(Database::at(root))
    } else {
        Database::open(root)
    }
}

#[cfg(test)]
#[path = "database_tests.rs"]
mod tests;
