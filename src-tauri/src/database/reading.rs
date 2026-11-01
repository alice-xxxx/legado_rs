//! 正文请求只读取当前章节及其规则上下文，不加载整本目录。
use super::*;
use tauri_plugin_source_engine::TextOr;
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
