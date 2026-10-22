use super::*;

impl Database {
    pub(crate) fn assert_shelf(&self, expected: &[&str]) {
        let ids = db::read(self, |s| {
            s.read(
                "SELECT book_id FROM library_books ORDER BY position",
                [],
                |r| r.get::<_, String>(0),
            )
        })
        .unwrap();
        // 并发导入只要求成员一致，顺序由实际入队顺序决定。
        let mut ids = ids;
        let mut expected = expected.to_vec();
        ids.sort();
        expected.sort();
        assert_eq!(ids, expected);
    }
    pub(crate) fn assert_shelf_groups(&self, book: &str, expected: &[&str]) {
        let groups = db::read(self, |s| {
            s.read(
                "SELECT group_name FROM library_group_books WHERE book_id=?1 ORDER BY group_name",
                [book],
                |r| r.get::<_, String>(0),
            )
        })
        .unwrap();
        assert_eq!(groups, expected);
    }
    pub(crate) fn test_search_history(&self) -> Vec<(String, u64)> {
        db::read(self, |s| {
            s.read(
                "SELECT query,usage FROM search_history ORDER BY last_use_time_ms DESC",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap()
    }
}

fn fixture(id: &str) -> Book {
    Book {
        id: id.into(),
        content_type: "text".into(),
        chapters: vec![Chapter {
            id: "first".into(),
            title: "第一章".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn completion_means_commit_and_dropping_notification_keeps_work() {
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    drop(database.import_book(fixture("book")).unwrap());
    database
        .record_reading_time(crate::reading_tools::ReadingSession {
            book_id: "book".into(),
            session_id: "session".into(),
            duration_ms: 100,
            ended_at_ms: 1000,
            utc_offset_minutes: 0,
        })
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(database.book("book").unwrap().unwrap().chapters.len(), 1);
    let total = db::read(&database, |s| {
        s.read("SELECT duration_ms FROM reading_totals", [], |r| {
            r.get::<_, u64>(0)
        })
    })
    .unwrap();
    assert_eq!(total, [100]);
    assert!(db::read(&database, |s| s.write("DELETE FROM books", [])).is_err());
}

#[test]
fn failed_batch_rolls_back_and_writer_continues() {
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    database
        .import_book(fixture("book"))
        .unwrap()
        .wait()
        .unwrap();
    let failed = database
        .submit(vec![
            Mutation::SaveBook {
                book_id: "book".into(),
                changes: BookPatch {
                    title: Change::Set("修改".into()),
                    ..Default::default()
                },
            },
            Mutation::SaveBook {
                book_id: "missing".into(),
                changes: BookPatch::default(),
            },
        ])
        .unwrap()
        .wait();
    assert!(failed.is_err());
    database
        .save_reading_progress(
            "book".into(),
            ProgressInput {
                updated_at_ms: Some(1),
                ..Default::default()
            },
        )
        .unwrap()
        .wait()
        .unwrap();
    let title = db::read(&database, |s| {
        s.read(
            "SELECT title_override FROM books WHERE id='book'",
            [],
            |r| r.get::<_, Option<String>>(0),
        )
    })
    .unwrap();
    assert_eq!(title, [None]);
}

#[test]
fn borrowed_content_survives_owner_deletion_then_is_collected() {
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    let mut producer = fixture("producer");
    let directory = root.path().join("cache/text/producer");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("chapter.txt"), "正文").unwrap();
    producer.chapters[0].url = Some("chapter.txt".into());
    database.import_book(producer).unwrap().wait().unwrap();
    database
        .import_book(fixture("consumer"))
        .unwrap()
        .wait()
        .unwrap();
    let content_id = database.book("producer").unwrap().unwrap().chapters[0]
        .content_id
        .clone()
        .unwrap();
    database
        .submit(vec![Mutation::UseChapterContent {
            book_id: "consumer".into(),
            chapter_id: "first".into(),
            content_id,
        }])
        .unwrap()
        .wait()
        .unwrap();
    futures_executor::block_on(
        database
            .remove_book_assets("producer".into(), Vec::new())
            .unwrap(),
    )
    .unwrap();
    assert!(directory.join("chapter.txt").is_file());
    assert!(database.book("consumer").unwrap().unwrap().chapters[0]
        .url
        .is_some());
    futures_executor::block_on(
        database
            .remove_book_assets("consumer".into(), Vec::new())
            .unwrap(),
    )
    .unwrap();
    assert!(!directory.join("chapter.txt").exists());
}

#[test]
fn sessions_are_idempotent_and_split_at_local_midnight() {
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    let session = crate::reading_tools::ReadingSession {
        book_id: "book".into(),
        session_id: "session".into(),
        duration_ms: 120_000,
        ended_at_ms: 86_460_000,
        utc_offset_minutes: 0,
    };
    for _ in 0..2 {
        database
            .record_reading_time(session.clone())
            .unwrap()
            .wait()
            .unwrap();
    }
    let days = db::read(&database, |s| {
        s.read(
            "SELECT day,total_duration_ms FROM reading_days ORDER BY day",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)),
        )
    })
    .unwrap();
    assert_eq!(
        days,
        [("19700101".into(), 60_000), ("19700102".into(), 60_000)]
    );
}

#[test]
fn tts_keeps_relational_headers_and_login_choices() {
    use crate::source_wire::TextOr;
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    database
        .submit(vec![Mutation::SaveHttpTts {
            configs: vec![TtsInput {
                id: "tts".into(),
                name: "朗读".into(),
                enabled: true,
                definition: TtsDefinition {
                    id: Some(9_007_199_254_740_993),
                    name: "朗读".into(),
                    url: "https://tts.invalid".into(),
                    header: Some(TextOr::Data(
                        [("Authorization".into(), "secret".into())].into(),
                    )),
                    login_ui: Some(TextOr::Data(vec![LoginItem {
                        name: Some("音色".into()),
                        options: vec!["甲".into(), "乙".into()],
                        ..Default::default()
                    }])),
                    ..Default::default()
                },
            }],
        }])
        .unwrap()
        .wait()
        .unwrap();
    let config = database.tts_config("tts").unwrap().unwrap();
    assert_eq!(config.id, Some(9_007_199_254_740_993));
    let Some(TextOr::Data(headers)) = config.header else {
        panic!("请求头必须来自关系行");
    };
    assert_eq!(headers["Authorization"], "secret");
    let Some(TextOr::Data(items)) = config.login_ui else {
        panic!("表单必须来自关系行");
    };
    assert_eq!(items[0].options, ["甲", "乙"]);
}

#[tokio::test]
async fn restore_replaces_snapshot_and_next_write_reconnects() {
    let root = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let database = super::Database::new(root.path()).unwrap();
    let resources = crate::resources::Resources::open(root.path(), database.clone())
        .await
        .unwrap();
    resources
        .settings
        .save(&serde_json::to_value(crate::settings::SettingsDocument::default()).unwrap())
        .await
        .unwrap();
    database
        .import_book(fixture("original"))
        .unwrap()
        .await
        .unwrap();
    let path = archive.path().join("backup.zip");
    crate::backup::create_backup(root.path(), &database, &path)
        .await
        .unwrap();
    database
        .import_book(fixture("later"))
        .unwrap()
        .await
        .unwrap();
    crate::backup::restore::restore_backup(&database, &path)
        .await
        .unwrap();
    assert!(database.book("original").unwrap().is_some());
    assert!(database.book("later").unwrap().is_none());
    database
        .import_book(fixture("after-restore"))
        .unwrap()
        .await
        .unwrap();
    assert!(database.book("after-restore").unwrap().is_some());
}
