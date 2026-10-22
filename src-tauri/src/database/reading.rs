//! 正文请求只读取当前章节及其规则上下文，不加载整本目录。
use super::*;
use crate::source_wire::TextOr;
use std::collections::BTreeMap;

pub(crate) struct ChapterContext {
    pub content_type: String,
    pub source_id: Option<String>,
    pub file_url: Option<String>,
    pub engine: Option<EngineBook>,
    pub chapter: Chapter,
    pub next_chapter_url: Option<String>,
    pub progress_index: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_context_preserves_rules_and_only_the_requested_pages() {
        let root = tempfile::tempdir().unwrap();
        let db = Database::new(root.path()).unwrap();
        db.import_book(Book {
            id: "book".into(),
            content_type: "image".into(),
            engine: Some(EngineBook {
                book_url: Some("https://example/book".into()),
                variable: Some(TextOr::Data(BTreeMap::from([(
                    "book-var".into(),
                    "value".into(),
                )]))),
                ..Default::default()
            }),
            chapters: (0..1000)
                .map(|index| Chapter {
                    id: format!("c{index}"),
                    index,
                    title: format!("Chapter {index}"),
                    volume: index == 501,
                    engine_data: EngineChapter {
                        url: format!("https://example/{index}"),
                        index,
                        variable: Some(TextOr::Data(BTreeMap::from([(
                            "chapter-var".into(),
                            index.to_string(),
                        )]))),
                        ..Default::default()
                    },
                    pages: vec![Page {
                        id: "p1".into(),
                        src: format!("resource://cache/image/book/{index}.png"),
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .unwrap()
        .wait()
        .unwrap();
        db.submit(vec![Mutation::SaveProgress {
            book_id: "book".into(),
            progress: ProgressInput {
                chapter_index: 500,
                ..Default::default()
            },
        }])
        .unwrap()
        .wait()
        .unwrap();
        let context = db.chapter_context("book", "c500").unwrap().unwrap();
        let full = db.book("book").unwrap().unwrap();
        assert_eq!(context.chapter.index, 500);
        assert_eq!(context.progress_index, Some(500));
        assert_eq!(
            context.next_chapter_url.as_deref(),
            Some("https://example/501")
        );
        assert_eq!(
            serde_json::to_value(context.engine).unwrap(),
            serde_json::to_value(full.engine).unwrap()
        );
        assert_eq!(
            serde_json::to_value(context.chapter.engine_data).unwrap(),
            serde_json::to_value(&full.chapters[500].engine_data).unwrap()
        );
        assert_eq!(
            serde_json::to_value(context.chapter.pages).unwrap(),
            serde_json::to_value(&full.chapters[500].pages).unwrap()
        );
        assert_eq!(
            db.reading_chapter_ids("book", 500, 3).unwrap(),
            ["c500", "c502"]
        );
        assert_eq!(db.reading_chapter_ids("book", 999, 6).unwrap(), ["c999"]);
        assert!(db.chapter_context("book", "missing").unwrap().is_none());
        assert!(db.chapter_context("missing", "c500").unwrap().is_none());
    }
}

impl Database {
    pub(crate) fn reading_chapter_ids(
        &self,
        book: &str,
        start: usize,
        count: usize,
    ) -> Result<Vec<String>> {
        db::read(self, |s| {
            s.read(
                "SELECT id FROM chapters WHERE book_id=?1 AND position>=?2 AND position<?3 AND volume=0 ORDER BY position",
                params![book, start, start.saturating_add(count)],
                |r| r.get(0),
            )
        })
    }

    pub(crate) fn chapter_context(
        &self,
        book: &str,
        chapter: &str,
    ) -> Result<Option<ChapterContext>> {
        db::read(self, |s| {
            let Some(mut context) = s.read(
                "SELECT b.*,p.chapter_index FROM books b LEFT JOIN reading_progress p ON p.book_id=b.id WHERE b.id=?1",
                [book],
                |r| Ok(ChapterContext {
                    content_type: r.get("content_type")?,
                    source_id: r.get("source_id")?,
                    file_url: r.get("url")?,
                    engine: if r.get::<_, Option<String>>("book_url")?.is_some() {
                        Some(library::engine_row(r)?)
                    } else { None },
                    chapter: Chapter::default(),
                    next_chapter_url: None,
                    progress_index: r.get("chapter_index")?,
                }),
            )?.pop() else { return Ok(None); };
            let Some(current) = s
                .read(
                    "SELECT * FROM chapter_records WHERE book_id=?1 AND id=?2",
                    params![book, chapter],
                    library::chapter_row,
                )?
                .pop()
            else {
                return Ok(None);
            };
            context.chapter = current;
            context.next_chapter_url = s
                .read(
                    "SELECT engine_url FROM chapters WHERE book_id=?1 AND position=?2",
                    params![book, context.chapter.index + 1],
                    |r| r.get::<_, Option<String>>(0),
                )?
                .pop()
                .flatten()
                .filter(|url| !url.trim().is_empty());
            s.read(
                "SELECT chapter_id,name,value FROM engine_variables WHERE book_id=?1 AND chapter_id IS NULL UNION ALL SELECT chapter_id,name,value FROM engine_variables WHERE book_id=?1 AND chapter_id=?2",
                params![book, chapter],
                |r| {
                    let owner: Option<String> = r.get(0)?;
                    let variable = if owner.is_some() {
                        &mut context.chapter.engine_data.variable
                    } else if let Some(engine) = &mut context.engine {
                        &mut engine.variable
                    } else { return Ok(()); };
                    if let TextOr::Data(values) = variable.get_or_insert_with(|| TextOr::Data(BTreeMap::new())) {
                        values.insert(r.get(1)?, r.get(2)?);
                    }
                    Ok(())
                },
            )?;
            if context.content_type == "image" {
                context.chapter.pages = s.read(
                    "SELECT id,url,width,height FROM chapter_pages WHERE book_id=?1 AND chapter_id=?2 ORDER BY position",
                    params![book, chapter],
                    |r| Ok(Page { id: r.get(0)?, src: r.get(1)?, width: r.get(2)?, height: r.get(3)? }),
                )?;
            }
            Ok(Some(context))
        })
    }
}
