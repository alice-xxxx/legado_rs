use super::*;
use crate::resources::ResourceStore;

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
    db.remove_search_history(Some("FIRST".into())).unwrap();
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
    db.record_session(session.clone()).unwrap();
    db.record_session(session).unwrap();
    let value = db.read("history.json").unwrap().unwrap();
    assert_eq!(value["reading"]["totalSessions"], 1);
    assert_eq!(value["reading"]["totalDurationMs"], 2000);
    let days = value["reading"]["days"].as_array().unwrap();
    assert_eq!(days.len(), 2);
    assert!(days
        .iter()
        .all(|day| day["totalDurationMs"] == 1000 && day["sessionCount"] == 1));
    db.save_bookmark("book".into(),json!({"id":"mark","chapterId":"chapter","chapterIndex":0,"offset":9,"note":"note","createdAtMs":1,"updatedAtMs":1})).unwrap();
    assert_eq!(
        db.connect()
            .unwrap()
            .query_row("SELECT count(*) FROM reading_sessions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.remove_bookmark("book".into(), "mark".into()).unwrap();
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
    db.reset_progress(path).unwrap();
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
    for id in ["one", "two"] {
        db.save_rule("replacementRules",id,&json!({"id":id,"name":id,"pattern":"a","replacement":"b","enabled":true,"isRegex":false,"scope":"all"})).unwrap();
    }
    db.transaction(|tx|{tx.execute_batch("CREATE TRIGGER protect_rule_update BEFORE UPDATE ON replacement_rules WHEN old.id='two' BEGIN SELECT RAISE(ABORT,'unrelated rule changed'); END; CREATE TRIGGER protect_rule_delete BEFORE DELETE ON replacement_rules WHEN old.id='two' BEGIN SELECT RAISE(ABORT,'unrelated rule deleted'); END;")?;Ok(())}).unwrap();
    db.save_rule("replacementRules","one",&json!({"id":"one","name":"Edited","pattern":"x","replacement":"y","enabled":true,"isRegex":false,"scope":"all"})).unwrap();
    db.save_rule("replacementRules", "one", &Value::Null)
        .unwrap();
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
