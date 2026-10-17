use super::*;
use crate::resources::{ResourceRef, ResourceStore};

#[test]
fn records_round_trip_and_transactions_roll_back() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let cases = [
        Value::Null,
        json!(42),
        json!([]),
        json!({}),
        json!([{"id":"same","x":1},{"id":"same","x":2},null]),
        json!({"title":"书", "chapters":[{"id":"b"},{"id":"a"}],"media":{"a/b":{"url":"https://fixture"}},"unknown":{"nested":true}}),
        json!({"reading":{"sessions":[{"id":"one","duration":12}],"days":[]},"bookmarks":{"book":[{"id":"mark"}]},"a/b":{"\u{0000}":{"nested":{}}}}),
        json!({"reading":1,"bookmarks":[]}),
    ];
    for value in cases {
        db.write("fixture", &value).unwrap();
        assert_eq!(db.read("fixture").unwrap(), Some(value));
    }
    let before = db.read("fixture").unwrap();
    let result: Result<(), ResourceError> = db.transaction(|tx| {
        write(tx, "fixture", &json!({"chapters":[{"id":"changed"}]}))?;
        write(tx, "sibling", &json!([1, 2, 3]))?;
        Err(ResourceError::new("abort"))
    });
    assert!(result.is_err());
    assert_eq!(db.read("fixture").unwrap(), before);
    assert!(!db.exists("sibling").unwrap());
}

#[test]
fn catalogue_snapshots_preserve_independently_committed_media_and_progress() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let path = "cache/text/book/book.json";
    db.write(path,&json!({"id":"book","title":"Title","chapters":[{"id":"chapter","title":"One"}],"media":{"old":{"url":"https://old"}}})).unwrap();
    let snapshot = db.read(path).unwrap().unwrap();
    db.set_media(path, "book", "new", Some(&json!({"url":"https://new"})))
        .unwrap();
    db.set_media(path, "book", "old", Some(&json!({"url":"https://updated"})))
        .unwrap();
    db.save_progress(
        path,
        "text",
        &json!({"chapterId":"chapter","chapterIndex":0,"offset":9,"updatedAtMs":100}),
    )
    .unwrap();
    db.write(path, &snapshot).unwrap();
    let stored = db.read(path).unwrap().unwrap();
    assert_eq!(stored["media"]["new"]["url"], "https://new");
    assert_eq!(stored["media"]["old"]["url"], "https://updated");
    assert_eq!(stored["progress"]["charOffset"], 9);
    db.reset_progress(path).unwrap();
    assert!(db.read(path).unwrap().unwrap()["progress"].is_null());
}

#[test]
fn lightweight_previews_match_full_views_and_display_edits_preserve_the_catalogue() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let path = "cache/text/book/book.json";
    let document = json!({"id":"book","title":"Base","author":"Author","cover":"cover.webp","sourceId":"source","engine":{"bookUrl":"https://fixture"},
        "chapters":[{"id":"volume","title":"Volume","volume":true},{"id":"chapter","title":"One"}],
        "progress":{"chapterId":"chapter","charOffset":7,"updatedAtMs":100}});
    db.write(path, &document).unwrap();
    let expected = crate::resources::content::public_book_view(&document, "text").unwrap();
    let preview = db.preview("book").unwrap();
    for key in [
        "id",
        "title",
        "author",
        "coverSrc",
        "sourceId",
        "contentType",
        "chapterCount",
        "latestChapter",
        "canChangeSource",
        "progress",
    ] {
        assert_eq!(preview[key], expected[key], "{key}");
    }
    assert!(preview.get("chapters").is_none());
    assert!(preview.get("engine").is_none());
    db.connect().unwrap().execute_batch("CREATE TRIGGER immutable_chapters BEFORE UPDATE ON records WHEN old.field='chapters' BEGIN SELECT RAISE(ABORT,'chapter rewrite'); END;").unwrap();
    db.save_display(path, "text", &json!({"title":"Changed","author":"Edited"}))
        .unwrap();
    assert_eq!(db.preview("book").unwrap()["title"], "Changed");
    assert_eq!(db.read(path).unwrap().unwrap()["title"], "Base");
    db.save_display(path, "text", &json!({"title":null,"author":null}))
        .unwrap();
    assert_eq!(db.preview("book").unwrap()["title"], "Base");
}

#[test]
fn progress_and_media_never_rewrite_chapters_or_unrelated_fields() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    let path = "cache/image/book/book.json";
    let chapters = json!([{"id":"volume","volume":true},{"id":"chapter","pages":[{"id":"p1","url":"p.webp"}]}]);
    db.write(
        path,
        &json!({"id":"book","title":"Title","chapters":chapters,"media":{}}),
    )
    .unwrap();
    db.write(
        crate::resources::content::LIBRARY_FILE,
        &json!({"image":[{"id":"book","title":"Title","total":1}]}),
    )
    .unwrap();
    db.connect().unwrap().execute_batch("CREATE TRIGGER forbid_chapter_update BEFORE UPDATE ON records WHEN old.field='chapters' BEGIN SELECT RAISE(ABORT,'chapter rewrite'); END;
        CREATE TRIGGER forbid_chapter_delete BEFORE DELETE ON records WHEN old.field='chapters' BEGIN SELECT RAISE(ABORT,'chapter delete'); END;
        CREATE TRIGGER forbid_title_update BEFORE UPDATE ON fields WHEN old.name='title' BEGIN SELECT RAISE(ABORT,'metadata rewrite'); END;").unwrap();
    let progress = json!({"chapterId":"chapter","chapterIndex":1,"pageId":"p1","offset":250_000,"updatedAtMs":2000});
    let update = db.save_progress(path, "image", &progress).unwrap().unwrap();
    assert_eq!(update["read"], 1);
    assert_eq!(update["chapterChanged"], true);
    assert!(db.save_progress(path,"image",&json!({"chapterId":"chapter","chapterIndex":1,"pageId":"p1","offset":1,"updatedAtMs":1000})).unwrap().is_none());
    assert!(db
        .save_progress(
            path,
            "image",
            &json!({"chapterIndex":1,"pageId":"alien","offset":1,"updatedAtMs":3000})
        )
        .is_err());
    db.set_media(
        path,
        "book",
        "opaque",
        Some(&json!({"url":"https://fixture/1","headers":{"Referer":"https://fixture"}})),
    )
    .unwrap();
    db.set_media(
        path,
        "book",
        "opaque",
        Some(&json!({"url":"https://fixture/2"})),
    )
    .unwrap();
    let document = db.read(path).unwrap().unwrap();
    assert_eq!(document["chapters"], chapters);
    assert_eq!(document["progress"]["offsetRatio"], 0.25);
    assert_eq!(document["media"]["opaque"]["url"], "https://fixture/2");
    assert_eq!(
        db.read(crate::resources::content::LIBRARY_FILE)
            .unwrap()
            .unwrap()["image"][0]["read"],
        1
    );
    db.set_media(path, "book", "opaque", None).unwrap();
    assert!(db.media().unwrap().is_empty());
}

#[test]
fn wal_reader_keeps_a_snapshot_while_an_independent_writer_commits() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path()).unwrap();
    db.write("settings.json", &json!({"counter":0})).unwrap();
    let mut connection = db.connect().unwrap();
    let snapshot = connection.transaction().unwrap();
    assert_eq!(
        read(&snapshot, "settings.json").unwrap().unwrap()["counter"],
        0
    );
    let writer = db.clone();
    std::thread::spawn(move || {
        writer
            .write("settings.json", &json!({"counter":1}))
            .unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(
        read(&snapshot, "settings.json").unwrap().unwrap()["counter"],
        0
    );
    snapshot.commit().unwrap();
    assert_eq!(db.read("settings.json").unwrap().unwrap()["counter"], 1);
}

#[tokio::test]
async fn separate_store_instances_preserve_concurrent_database_updates() {
    let root = tempfile::tempdir().unwrap();
    let first = ResourceStore::open(root.path()).unwrap();
    let second = ResourceStore::open(root.path()).unwrap();
    let reference = first.document_ref("tasks").unwrap();
    first
        .write_json_ref(&reference, &json!({"counter":0}))
        .await
        .unwrap();
    let mut jobs = Vec::new();
    for index in 0..20 {
        let store = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let reference = reference.clone();
        jobs.push(tokio::spawn(async move {
            store
                .update_json_ref(&reference, |mut value| {
                    value["counter"] = json!(value["counter"].as_u64().unwrap() + 1);
                    Ok(value)
                })
                .await
                .unwrap();
        }));
    }
    for job in jobs {
        job.await.unwrap();
    }
    assert_eq!(
        first.read_json_ref(&reference).await.unwrap()["counter"],
        20
    );
    assert!(!root.path().join("tasks.json").exists());
}

#[tokio::test]
async fn database_backup_restores_state_private_records_and_assets() {
    let root = tempfile::tempdir().unwrap();
    let store = ResourceStore::open(root.path()).unwrap();
    let history = serde_json::to_value(crate::reading_tools::HistoryDocument::default()).unwrap();
    store
        .write_json_ref(&store.history_ref(), &history)
        .await
        .unwrap();
    let chapter = store
        .write_chapter_text("book", "chapter", "body")
        .await
        .unwrap();
    let path = "cache/text/book/book.json";
    store.database().write(path,&json!({"id":"book","title":"Title","chapters":[{"id":"chapter","title":"One","url":chapter.path().rsplit('/').next().unwrap()}],
        "progress":{"chapterId":"chapter","charOffset":3,"updatedAtMs":1000},"media":{"opaque":{"url":"https://fixture","headers":{"Authorization":"private"}}}})).unwrap();
    store.database().write(crate::resources::content::LIBRARY_FILE,&json!({"text":[{"id":"book","title":"Title","total":1}],"groups":[{"name":"Group","id":["book"]}]})).unwrap();
    store
        .database()
        .write(
            "source-engine/discovery-categories/source-0123456789abcdef.json",
            &json!({"x":"https://fixture"}),
        )
        .unwrap();
    store.database().transaction(|tx| {
        tx.execute("INSERT INTO source_kv(namespace,key,value) VALUES('book-variables','key','saved'),('source-cache','local','before')",[])?;
        Ok(())
    }).unwrap();
    let destination = tempfile::tempdir().unwrap();
    let archive = destination.path().join("backup.zip");
    crate::backup::create_backup(&store, &archive)
        .await
        .unwrap();
    store
        .database()
        .write(path, &json!({"id":"book","title":"Changed","chapters":[]}))
        .unwrap();
    store
        .database()
        .transaction(|tx| {
            tx.execute("UPDATE source_kv SET value='after'", [])?;
            Ok(())
        })
        .unwrap();
    crate::backup::restore_backup(&store, &archive)
        .await
        .unwrap();
    let restored = store
        .read_raw_document(&ResourceRef::new(format!("resource://{path}")).unwrap())
        .unwrap();
    assert_eq!(restored["title"], "Title");
    assert_eq!(restored["progress"]["charOffset"], 3);
    assert_eq!(
        restored["media"]["opaque"]["headers"]["Authorization"],
        "private"
    );
    assert_eq!(store.read_json_ref(&chapter).await.unwrap()["text"], "body");
    let connection = store.database().connect().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM source_kv WHERE namespace='book-variables' AND key='key'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "saved"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM source_kv WHERE namespace='source-cache' AND key='local'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "after"
    );
    assert!(store
        .database()
        .exists("source-engine/discovery-categories/source-0123456789abcdef.json")
        .unwrap());
    assert!(!root.path().join(path).exists());
}

#[tokio::test]
async fn metadata_transaction_rolls_back_all_documents_on_failure() {
    let root = tempfile::tempdir().unwrap();
    let store = ResourceStore::open(root.path()).unwrap();
    let reference = store.document_ref("tasks").unwrap();
    store
        .write_json_ref(&reference, &json!({"counter":0}))
        .await
        .unwrap();
    let sibling = store.history_ref();
    store.database().connect().unwrap().execute_batch("CREATE TRIGGER fail_history BEFORE INSERT ON documents WHEN new.path='history.json' BEGIN SELECT RAISE(ABORT,'failure'); END;").unwrap();
    let writer = store.transaction_writer_guard().await.unwrap();
    let replacements = vec![
        writer
            .public_json_replacement(&reference, &json!({"counter":1}))
            .unwrap(),
        writer
            .public_json_replacement(&sibling, &json!({}))
            .unwrap(),
    ];
    let transaction = crate::resource_transactions::FileTransaction::prepare(
        root.path(),
        "fixture",
        replacements,
        vec![],
        &writer,
    )
    .unwrap();
    assert!(transaction.commit().is_err());
    assert_eq!(store.read_json_ref(&reference).await.unwrap()["counter"], 0);
    assert!(!store.database().exists(sibling.path()).unwrap());
    assert!(!root.path().join("transactions").exists());
}

#[tokio::test]
async fn committed_cleanup_intents_retry_after_restart_and_block_new_publication() {
    let root = tempfile::tempdir().unwrap();
    let store = ResourceStore::open(root.path()).unwrap();
    let chapter = store
        .write_chapter_text("book", "chapter", "body")
        .await
        .unwrap();
    let path = root.path().join(chapter.path());
    let reference = store.document_ref("tasks").unwrap();
    store
        .write_json_ref(&reference, &json!({"counter":0}))
        .await
        .unwrap();
    let writer = store.transaction_writer_guard().await.unwrap();
    let replacement = writer
        .public_json_replacement(&reference, &json!({"counter":1}))
        .unwrap();
    let deletion =
        crate::resource_transactions::PostCommitDelete::chapter(chapter.clone()).unwrap();
    let prepared = crate::resource_transactions::FileTransaction::prepare(
        root.path(),
        "cleanup-fixture",
        vec![replacement],
        vec![deletion],
        &writer,
    )
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let failure = prepared.commit().unwrap_err();
    assert_eq!(
        failure.commit_state(),
        crate::resource_transactions::CommitState::Committed
    );
    assert!(failure.recovery_required_flag());
    drop(writer);
    assert_eq!(store.read_json_ref(&reference).await.unwrap()["counter"], 1);
    assert!(store.transaction_writer_guard().await.is_err());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, "body").unwrap();
    crate::resource_transactions::recover_all(root.path()).unwrap();
    crate::resource_transactions::recover_all(root.path()).unwrap();
    assert!(!path.exists());
    assert!(store.transaction_writer_guard().await.is_ok());
}

#[tokio::test]
async fn deleting_rss_resources_removes_the_private_database_media_records() {
    let root = tempfile::tempdir().unwrap();
    let store = ResourceStore::open(root.path()).unwrap();
    store
        .database()
        .set_media(
            "cache/html/rss-source/rss-source.json",
            "rss-source",
            "image",
            Some(&json!({"url":"https://fixture/image"})),
        )
        .unwrap();
    store.remove_rss_chapter_resources("source").await.unwrap();
    assert!(store.database().media().unwrap().is_empty());
    assert!(!store
        .database()
        .exists("cache/html/rss-source/rss-source.json")
        .unwrap());
}
