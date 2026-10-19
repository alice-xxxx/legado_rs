use super::*;
use crate::resources::ResourceStore;

fn changes(value: Value) -> Vec<Change> { serde_json::from_value(value).unwrap() }

#[test]
fn shared_source_writes_preserve_ids_raw_integers_and_derived_metadata() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let raw = r#"{"bookSourceName":"Original","bookSourceUrl":"https://original","lastUpdateTime":9007199254740993,"unknown":{"rule":"keep"}}"#;
    db.modify(changes(json!([{"action":"upsert","table":"sources","keys":["id"],"values":{"definition_json":raw}}]))).unwrap();
    let saved = db.read("sources.json").unwrap().unwrap();
    let id = saved[0]["id"].as_str().unwrap();
    assert_eq!(saved[0]["source"]["lastUpdateTime"], 9007199254740993_u64);
    let edited = json!({"bookSourceName":"Edited","bookSourceUrl":"https://edited","enabled":false,"bookSourceType":2,"ruleSearch":{"bookList":"x"},"unknown":{"rule":"keep"}}).to_string();
    db.modify(changes(json!([{"action":"update","table":"sources","filter":{"id":id},"values":{"definition_json":edited}}]))).unwrap();
    // 重新导入改过地址的源仍使用原 ID，不新增副本。
    db.modify(changes(json!([{"action":"upsert","table":"sources","keys":["id"],"values":{"definition_json":edited}}]))).unwrap();
    let sources = db.read("sources.json").unwrap().unwrap();
    assert_eq!(sources.as_array().unwrap().len(), 1);
    assert_eq!(sources[0]["id"], id);
    assert_eq!(sources[0]["name"], "Edited");
    assert_eq!(sources[0]["source"]["unknown"]["rule"], "keep");
    let metadata: (bool, String, bool) = db.connect().unwrap().query_row("SELECT enabled,content_type,can_search FROM sources", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(metadata, (false, "image".into(), true));
    assert!(db.modify(changes(json!([
        {"action":"upsert","table":"sources","keys":["id"],"values":{"definition_json":{"bookSourceName":"Rollback","bookSourceUrl":"https://rollback"}}},
        {"action":"upsert","table":"sources","keys":["id"],"values":{"id":"conflict","definition_json":edited}}
    ]))).is_err());
    assert_eq!(db.read("sources.json").unwrap().unwrap(), sources);
}

#[test]
fn source_deletion_rolls_back_related_rows_and_only_cleans_its_own_rss_cache() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    db.modify(changes(json!([{"action":"upsert","table":"sources","keys":["id"],"values":{"id":"feed","definition_json":{"bookSourceName":"Feed","bookSourceUrl":"https://feed","bookSourceType":5}}}]))).unwrap();
    db.write("cache/text/book/book.json", &json!({"id":"book","title":"Keep","chapters":[]})).unwrap();
    db.write("cache/html/rss-feed/rss-feed.json", &json!({"id":"rss-feed","title":"Feed","chapters":[]})).unwrap();
    let directory = root.path().join("cache/html/rss-feed");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("body.html"), "body").unwrap();
    db.modify(changes(json!([
        {"action":"insert","table":"rss_subscriptions","values":{"source_id":"feed","position":0,"filter":"all"}},
        {"action":"insert","table":"library_books","values":{"book_id":"book","position":0}}
    ]))).unwrap();
    db.transaction(|tx| { tx.execute_batch("CREATE TRIGGER protect_source BEFORE DELETE ON sources BEGIN SELECT RAISE(ABORT,'rollback'); END;")?; Ok(()) }).unwrap();
    assert!(db.modify(changes(json!([{"action":"delete","table":"sources","filter":{"id":["feed"]}}]))).is_err());
    assert!(directory.join("body.html").is_file());
    assert_eq!(db.connect().unwrap().query_row("SELECT count(*) FROM rss_subscriptions", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    db.transaction(|tx| { tx.execute_batch("DROP TRIGGER protect_source;")?; Ok(()) }).unwrap();
    db.modify(changes(json!([{"action":"delete","table":"sources","filter":{"id":["feed"]}}]))).unwrap();
    assert!(!directory.exists());
    assert!(db.read("cache/html/rss-feed/rss-feed.json").unwrap().is_none());
    assert_eq!(db.read("content-library.json").unwrap().unwrap()["text"][0]["id"], "book");
    assert_eq!(db.connect().unwrap().query_row("SELECT count(*) FROM pending_cleanup", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn shared_writes_bind_values_and_preserve_unsupplied_columns() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let name = "a'); DELETE FROM books; -- 中文";
    db.modify(changes(json!([
        {"action":"insert","table":"http_tts_configs","values":{"id":"one","position":0,"name":name,"enabled":true,"definition_json":{"url":"https://example.com","headers":["a","b"]}}},
        {"action":"upsert","table":"http_tts_configs","keys":["id"],"values":{"id":"one","position":0,"name":"Edited","enabled":false,"definition_json":{"unchanged":true}}},
        {"action":"update","table":"http_tts_configs","filter":{"id":"one"},"values":{"name":name}}
    ]))).unwrap();
    let rows = db.read("http-tts.json").unwrap().unwrap();
    assert_eq!(rows["configs"][0]["name"], name);
    assert_eq!(rows["configs"][0]["enabled"], false);
    assert_eq!(rows["configs"][0]["config"], json!({"unchanged":true}));
    // 空数组不匹配任何行；空筛选才表示全表。
    db.modify(changes(json!([{"action":"delete","table":"http_tts_configs","filter":{"id":[]}}]))).unwrap();
    assert_eq!(db.read("http-tts.json").unwrap().unwrap()["configs"].as_array().unwrap().len(), 1);
    db.modify(changes(json!([{"action":"delete","table":"http_tts_configs","filter":{"id":["one"]}}]))).unwrap();
    assert!(db.read("http-tts.json").unwrap().unwrap()["configs"].as_array().unwrap().is_empty());
}

#[test]
fn shared_writes_roll_back_the_whole_batch_and_reject_sql_identifiers() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    for last in [
        json!({"action":"delete","table":"library_groups; DROP TABLE books","filter":{}}),
        json!({"action":"update","table":"library_groups","filter":{},"values":{"position":2}}),
        json!({"action":"insert","table":"library_books","values":{"book_id":"missing","position":0}}),
    ] {
        assert!(db.modify(changes(json!([
            {"action":"insert","table":"library_groups","values":{"name":"rollback","position":0}}, last
        ]))).is_err());
        let count: i64 = db.connect().unwrap().query_row("SELECT count(*) FROM library_groups", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }
    assert!(serde_json::from_value::<Change>(json!({"action":"delete","table":"books","filter":{},"sql":"DROP TABLE books"})).is_err());
}

#[test]
fn frontend_chapter_address_write_keeps_catalog_identity_and_other_caches() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    db.write("cache/image/original/original.json", &json!({"id":"original","title":"Book","chapters":[
        {"id":"one","title":"One","engineurl":"https://original/one","url":"old.json","pages":[{"id":"old","url":"old.jpg"}]},
        {"id":"two","title":"Two","url":"two.json"}
    ]})).unwrap();
    db.modify(changes(json!([
        {"action":"update","table":"chapters","filter":{"book_id":"original","id":"one"},"values":{"resource_url":"resource://cache/image/alternative/body.json"}},
        {"action":"delete","table":"chapter_pages","filter":{"book_id":"original","chapter_id":"one"}},
        {"action":"insert","table":"chapter_pages","values":{"book_id":"original","chapter_id":"one","position":0,"id":"page","url":"resource://cache/image/alternative/page.jpg"}}
    ]))).unwrap();
    let book = db.read("cache/image/original/original.json").unwrap().unwrap();
    assert_eq!(book["chapters"][0]["engineurl"], "https://original/one");
    assert_eq!(book["chapters"][0]["title"], "One");
    assert_eq!(book["chapters"][0]["url"], "resource://cache/image/alternative/body.json");
    assert_eq!(book["chapters"][0]["pages"][0]["url"], "resource://cache/image/alternative/page.jpg");
    assert_eq!(book["chapters"][1]["url"], "two.json");
}

#[test]
fn task_checkpoints_leave_other_tasks_and_input_snapshots_untouched() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    for id in ["one", "two"] {
        db.save_task_changes(Some(json!({"id":id,"kind":"search","status":"queued","sourceIds":["source-one","source-two"],"total":2,"completed":0,"createdAtMs":1,"updatedAtMs":1})),vec![]).unwrap();
    }
    db.transaction(|tx| {
        tx.execute_batch("CREATE TRIGGER protect_task BEFORE UPDATE ON tasks WHEN old.id='two' BEGIN SELECT RAISE(ABORT,'unrelated task updated'); END; CREATE TRIGGER protect_task_delete BEFORE DELETE ON tasks BEGIN SELECT RAISE(ABORT,'task list replaced'); END; CREATE TRIGGER protect_inputs BEFORE DELETE ON task_sources BEGIN SELECT RAISE(ABORT,'source snapshot replaced'); END;")?;
        Ok(())
    }).unwrap();
    db.save_task_changes(None, vec![json!({"id":"one","status":"running","completed":1,"total":2,"updatedAtMs":2})]).unwrap();
    let value = db.read("tasks.json").unwrap().unwrap();
    assert_eq!(value["tasks"][0]["completed"], 1);
    assert_eq!(value["tasks"][0]["sourceIds"], json!(["source-one","source-two"]));
    assert_eq!(value["tasks"][1]["status"], "queued");
    // A batch failure must also roll back a new task and earlier state updates.
    assert!(db.save_task_changes(Some(json!({"id":"three","kind":"search","status":"queued"})),vec![json!({"id":"one","status":"completed","completed":2,"total":2,"updatedAtMs":3}),json!({"id":"missing","status":"cancelled"})]).is_err());
    let value = db.read("tasks.json").unwrap().unwrap();
    assert_eq!(value["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(value["tasks"][0]["status"], "running");
}

#[test]
fn search_history_updates_never_rebuild_reading_history_or_bookmarks() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    db.transaction(|tx|{tx.execute_batch("CREATE TRIGGER protect_reading BEFORE UPDATE ON reading_totals BEGIN SELECT RAISE(ABORT,'unrelated reading update'); END; CREATE TRIGGER protect_bookmarks BEFORE DELETE ON bookmarks BEGIN SELECT RAISE(ABORT,'unrelated bookmark delete'); END;")?;Ok(())}).unwrap();
    db.record_search("search-one".into(), "First".into(), "first".into(), 100)
        .unwrap();
    db.record_search("search-one".into(), "First".into(), "first".into(), 100)
        .unwrap();
    db.record_search("search-two".into(), "FIRST".into(), "first".into(), 200)
        .unwrap();
    let history = db.read_search_history().unwrap();
    assert_eq!(history["entries"][0]["usage"], 2);
    assert_eq!(history["entries"][0]["query"], "FIRST");
    assert_eq!(history["entries"][0]["firstUseTimeMs"], 100);
    assert_eq!(history["processedSearchIds"].as_array().unwrap().len(), 2);
    db.modify(changes(json!([{"action":"delete","table":"search_history","filter":{"query":"FIRST"}}]))).unwrap();
    assert!(db.read_search_history().unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn reading_sessions_are_counted_once_and_split_at_local_midnight() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let session = crate::reading_tools::ReadingSession {
        session_id: "session-one".into(),
        book_id: "book".into(),
        duration_ms: 2000,
        ended_at_ms: 86_401_000,
        utc_offset_minutes: 0,
    };
    db.modify(vec![Change::RecordSession { session: session.clone() }]).unwrap();
    db.modify(vec![Change::RecordSession { session }]).unwrap();
    let value = db.read("history.json").unwrap().unwrap();
    assert_eq!(value["reading"]["totalSessions"], 1);
    assert_eq!(value["reading"]["totalDurationMs"], 2000);
    let days = value["reading"]["days"].as_array().unwrap();
    assert_eq!(days.len(), 2);
    assert!(days
        .iter()
        .all(|day| day["totalDurationMs"] == 1000 && day["sessionCount"] == 1));
    db.modify(changes(json!([{"action":"upsert","table":"bookmarks","keys":["id"],"values":{"id":"mark","book_id":"book","position":0,"chapter_id":"chapter","chapter_index":0,"offset":9,"note":"note","created_at_ms":1,"updated_at_ms":1}}]))).unwrap();
    assert_eq!(
        db.connect()
            .unwrap()
            .query_row("SELECT count(*) FROM reading_sessions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.modify(changes(json!([{"action":"delete","table":"bookmarks","filter":{"book_id":"book","id":"mark"}}]))).unwrap();
    assert_eq!(
        db.read("history.json").unwrap().unwrap()["reading"]["totalDurationMs"],
        2000
    );
}

#[test]
fn schema_uses_business_tables_and_readers_cannot_write() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let reader = db.connect().unwrap();
    let generic: i64 = reader
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('documents','fields','records')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(generic, 0);
    assert!(reader
        .execute(
            "INSERT INTO library_groups(name,position) VALUES('unauthorized',0)",
            []
        )
        .is_err());
    assert!(db
        .write("unknown.json", &json!({"nested":"document"}))
        .is_err());
    db.write("settings.json", &json!({"theme":"dark"})).unwrap();
    assert_eq!(
        crate::configuration::read(root.path(), "settings.json")
            .unwrap()
            .unwrap()["theme"],
        "dark"
    );
    for path in [
        "http-tts.json",
        "replacement-rules.json",
        "txt-toc-rules.json",
        "discovery/source.json",
        "source-engine/discovery-categories/source.json",
    ] {
        assert!(!crate::configuration::is_file(path));
        assert!(!root.path().join(path).exists());
    }
}

#[test]
fn wal_reader_keeps_its_snapshot_while_the_owner_commits() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let mut connection = db.connect().unwrap();
    let snapshot = connection.transaction().unwrap();
    assert_eq!(
        snapshot
            .query_row("SELECT sessions FROM reading_totals", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let writer = db.clone();
    std::thread::spawn(move || {
        writer
            .transaction(|tx| {
                tx.execute("UPDATE reading_totals SET sessions=1", [])?;
                Ok(())
            })
            .unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(
        snapshot
            .query_row("SELECT sessions FROM reading_totals", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    snapshot.commit().unwrap();
    assert_eq!(
        connection
            .query_row("SELECT sessions FROM reading_totals", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn one_owner_preserves_concurrent_updates_and_rolls_back_failed_commands() {
    let root = tempfile::tempdir().unwrap();
    let first = Database::open(root.path()).unwrap();
    let second = Database::open(root.path()).unwrap();
    let jobs = (0..20)
        .map(|i| {
            let db = if i % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            std::thread::spawn(move || {
                db.transaction(|tx| {
                    tx.execute("UPDATE reading_totals SET sessions=sessions+1", [])?;
                    Ok(())
                })
                .unwrap()
            })
        })
        .collect::<Vec<_>>();
    for job in jobs {
        job.join().unwrap();
    }
    assert_eq!(
        first
            .connect()
            .unwrap()
            .query_row("SELECT sessions FROM reading_totals", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        20
    );
    assert!(first
        .transaction(|tx| {
            tx.execute("UPDATE reading_totals SET sessions=999", [])?;
            tx.execute("INSERT INTO reading_totals VALUES(1,0,0)", [])?;
            Ok(())
        })
        .is_err());
    assert_eq!(
        first
            .connect()
            .unwrap()
            .query_row("SELECT sessions FROM reading_totals", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        20
    );
}

#[test]
fn progress_media_and_display_updates_leave_the_catalog_untouched() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let path = "cache/image/book/book.json";
    let chapters = json!([{"id":"volume","volume":true},{"id":"chapter","title":"One","pages":[{"id":"p1","url":"p.webp"}]}]);
    db.write(
        path,
        &json!({"id":"book","title":"Base","chapters":chapters}),
    )
    .unwrap();
    db.write(
        "content-library.json",
        &json!({"image":[{"id":"book","title":"Base"}]}),
    )
    .unwrap();
    db.transaction(|tx|{tx.execute_batch("CREATE TRIGGER no_chapter_update BEFORE UPDATE ON chapters BEGIN SELECT RAISE(ABORT,'catalog rewritten'); END; CREATE TRIGGER no_chapter_delete BEFORE DELETE ON chapters BEGIN SELECT RAISE(ABORT,'catalog deleted'); END; CREATE TRIGGER no_page_delete BEFORE DELETE ON chapter_pages BEGIN SELECT RAISE(ABORT,'pages deleted'); END;")?;Ok(())}).unwrap();
    let update=db.save_progress(path,"image",&json!({"chapterId":"chapter","chapterIndex":1,"pageId":"p1","offset":250000,"updatedAtMs":2000})).unwrap().unwrap();
    assert_eq!(update["read"], 1);
    assert!(db
        .save_progress(
            path,
            "image",
            &json!({"chapterId":"chapter","chapterIndex":1,"offset":0,"updatedAtMs":1000})
        )
        .unwrap()
        .is_none());
    assert!(db
        .save_progress(
            path,
            "image",
            &json!({"chapterIndex":1,"pageId":"alien","offset":0,"updatedAtMs":3000})
        )
        .is_err());
    db.set_media(
        path,
        "book",
        "opaque",
        Some(&json!({"url":"https://example.com/image"})),
    )
    .unwrap();
    db.save_display(path, "image", &json!({"title":"Edited"}))
        .unwrap();
    let stored = db.read(path).unwrap().unwrap();
    assert_eq!(stored["chapters"], chapters);
    assert_eq!(stored["progress"]["offsetRatio"], 0.25);
    assert_eq!(db.preview("book").unwrap()["title"], "Edited");
    assert_eq!(
        db.read("content-library.json").unwrap().unwrap()["image"][0]["read"],
        1
    );
    db.modify(changes(json!([{"action":"delete","table":"reading_progress","filter":{"book_id":"book"}}]))).unwrap();
    assert!(db.read(path).unwrap().unwrap()["progress"].is_null());
}

#[test]
fn a_chapter_commit_changes_only_its_own_row_and_children() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    db.write("cache/text/book/book.json",&json!({"id":"book","title":"Title","chapters":[{"id":"first","title":"One","engineurl":"https://example.com/one"},{"id":"second","title":"Two"}]})).unwrap();
    db.transaction(|tx|{tx.execute_batch("CREATE TRIGGER protect_sibling BEFORE UPDATE ON chapters WHEN old.id='second' BEGIN SELECT RAISE(ABORT,'sibling changed'); END; CREATE TRIGGER protect_catalog BEFORE DELETE ON chapters BEGIN SELECT RAISE(ABORT,'catalog replaced'); END;")?;Ok(())}).unwrap();
    db.save_chapter_view("book","first",&json!({"id":"first","title":"One","resource":{"resourceId":"resource://cache/text/book/body.txt"}})).unwrap();
    let stored = db.read("cache/text/book/book.json").unwrap().unwrap();
    assert_eq!(stored["chapters"][0]["url"], "body.txt");
    assert_eq!(
        stored["chapters"][0]["engineurl"],
        "https://example.com/one"
    );
    assert_eq!(stored["chapters"][1]["title"], "Two");
}

#[test]
fn editing_one_rule_does_not_read_or_replace_the_other_rules() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    for (position, id) in ["one", "two"].iter().enumerate() {
        db.modify(changes(json!([{"action":"upsert","table":"replacement_rules","keys":["id"],"values":{"id":id,"position":position,"name":id,"pattern":"a","replacement":"b","enabled":true,"is_regex":false,"scope":"all"}}]))).unwrap();
    }
    db.transaction(|tx|{tx.execute_batch("CREATE TRIGGER protect_rule_update BEFORE UPDATE ON replacement_rules WHEN old.id='two' BEGIN SELECT RAISE(ABORT,'unrelated rule changed'); END; CREATE TRIGGER protect_rule_delete BEFORE DELETE ON replacement_rules WHEN old.id='two' BEGIN SELECT RAISE(ABORT,'unrelated rule deleted'); END;")?;Ok(())}).unwrap();
    db.modify(changes(json!([{"action":"update","table":"replacement_rules","filter":{"id":"one"},"values":{"name":"Edited","pattern":"x","replacement":"y"}}]))).unwrap();
    db.modify(changes(json!([{"action":"delete","table":"replacement_rules","filter":{"id":"one"}}]))).unwrap();
    assert_eq!(
        db.read("replacement-rules.json").unwrap().unwrap()["rules"][0]["id"],
        "two"
    );
}

#[tokio::test]
async fn backup_restores_business_rows_settings_and_assets() {
    let root = tempfile::tempdir().unwrap();
    let store = ResourceStore::open(root.path()).unwrap();
    let chapter = store
        .write_chapter_text("book", "chapter", "body")
        .await
        .unwrap();
    let path = "cache/text/book/book.json";
    store.database().write(path,&json!({"id":"book","title":"Before","chapters":[{"id":"chapter","title":"One","url":chapter.path().rsplit('/').next().unwrap()}],"progress":{"chapterId":"chapter","charOffset":3,"updatedAtMs":1000}})).unwrap();
    store
        .database()
        .write(
            "content-library.json",
            &json!({"text":[{"id":"book","title":"Before"}]}),
        )
        .unwrap();
    let destination = tempfile::tempdir().unwrap();
    let archive = destination.path().join("backup.zip");
    crate::backup::create_backup(&store, &archive)
        .await
        .unwrap();
    store
        .database()
        .write(path, &json!({"id":"book","title":"After","chapters":[]}))
        .unwrap();
    crate::backup::restore_backup(&store, &archive)
        .await
        .unwrap();
    assert_eq!(
        store.database().read(path).unwrap().unwrap()["title"],
        "Before"
    );
    assert_eq!(store.read_json_ref(&chapter).await.unwrap()["text"], "body");
    assert!(root.path().join("settings.json").is_file());
    assert!(!root.path().join("http-tts.json").exists());
}
