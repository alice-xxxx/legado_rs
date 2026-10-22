//! 书籍、目录、引擎列和不可变正文的关系操作。
use super::*;
use crate::{resources::ResourceRef, source_wire::TextOr};
use std::collections::{BTreeMap, HashMap};

fn engine_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EngineBook> {
    Ok(EngineBook {
        book_url: r.get("book_url")?,
        toc_url: r.get("engine_toc_url")?,
        origin: r.get("engine_origin")?,
        origin_name: r.get("engine_origin_name")?,
        name: r.get("engine_name")?,
        author: r.get("engine_author")?,
        kind: r.get("engine_kind")?,
        cover_url: r.get("engine_cover_url")?,
        intro: r.get("engine_intro")?,
        charset: r.get("engine_charset")?,
        r#type: r.get("engine_type")?,
        latest_chapter_title: r.get("engine_latest_chapter_title")?,
        latest_chapter_time: r.get("engine_latest_chapter_time")?,
        word_count: r.get("engine_word_count")?,
        article_id: r.get("engine_article_id")?,
        variable: None,
    })
}
fn chapter_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Chapter> {
    Ok(Chapter {
        id: r.get("id")?,
        title: r.get("title")?,
        index: r.get("position")?,
        volume: r.get("volume")?,
        content_id: r.get("content_id")?,
        url: r.get("url")?,
        duration_ms: r.get("duration_ms")?,
        protocol: r.get("protocol")?,
        engine_data: EngineChapter {
            url: r
                .get::<_, Option<String>>("engine_url")?
                .unwrap_or_default(),
            title: r.get("title")?,
            index: r.get("position")?,
            is_volume: r.get("volume")?,
            book_url: r.get("engine_book_url")?,
            is_vip: r.get("engine_is_vip")?,
            is_pay: r.get("engine_is_pay")?,
            resource_url: r.get("engine_resource_url")?,
            tag: r.get("engine_tag")?,
            word_count: r.get("engine_word_count")?,
            start: r.get("engine_start")?,
            end: r.get("engine_end")?,
            start_fragment_id: r.get("engine_start_fragment_id")?,
            end_fragment_id: r.get("engine_end_fragment_id")?,
            variable: None,
        },
        ..Default::default()
    })
}
impl Database {
    pub(crate) fn book_type(&self, id: &str) -> Result<Option<String>> {
        db::read(self, |s| {
            Ok(
                s.read("SELECT content_type FROM books WHERE id=?1", [id], |r| {
                    r.get(0)
                })?
                .pop(),
            )
        })
    }
    pub(crate) fn book_engine(&self, id: &str) -> Result<Option<EngineBook>> {
        db::read(self, |s| {
            let Some(mut engine) = s
                .read(
                    "SELECT * FROM books WHERE id=?1 AND book_url IS NOT NULL",
                    [id],
                    engine_row,
                )?
                .pop()
            else {
                return Ok(None);
            };
            let pairs = s.read(
                "SELECT name,value FROM engine_variables WHERE book_id=?1 AND chapter_id IS NULL",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?;
            if !pairs.is_empty() {
                engine.variable = Some(TextOr::Data(pairs.into_iter().collect()));
            }
            Ok(Some(engine))
        })
    }
    pub(crate) fn book(&self, id: &str) -> Result<Option<Book>> {
        db::read(self, |s| {
            let Some(mut book) = s
                .read("SELECT * FROM books WHERE id=?1", [id], |r| {
                    Ok(Book {
                        id: r.get("id")?,
                        content_type: r.get("content_type")?,
                        title: r.get("title")?,
                        author: r.get("author")?,
                        intro: r.get("intro")?,
                        cover: r.get("cover")?,
                        cover_override: r.get("cover_override")?,
                        source_id: r.get("source_id")?,
                        word_count: r.get("word_count")?,
                        status: r.get("status")?,
                        original: r.get("original")?,
                        book_url: r.get("book_url")?,
                        page_count: r.get("page_count")?,
                        url: r.get("url")?,
                        updated_at_ms: r.get("updated_at_ms")?,
                        latest_chapter_title: r.get("latest_chapter_title")?,
                        engine: if r.get::<_, Option<String>>("book_url")?.is_some() {
                            Some(engine_row(r)?)
                        } else {
                            None
                        },
                        ..Default::default()
                    })
                })?
                .pop()
            else {
                return Ok(None);
            };
            book.progress = s
                .read(
                    "SELECT * FROM reading_progress WHERE book_id=?1",
                    [id],
                    super::activity::progress_row,
                )?
                .pop();
            book.chapters = s.read(
                "SELECT * FROM chapter_records WHERE book_id=?1 ORDER BY position",
                [id],
                chapter_row,
            )?;
            let positions: HashMap<_, _> = book
                .chapters
                .iter()
                .enumerate()
                .map(|(index, chapter)| (chapter.id.clone(), index))
                .collect();
            s.read(
                "SELECT chapter_id,name,value FROM engine_variables WHERE book_id=?1",
                [id],
                |r| {
                    let chapter: Option<String> = r.get(0)?;
                    let variable = if let Some(chapter) = chapter {
                        &mut book.chapters[positions[&chapter]].engine_data.variable
                    } else if let Some(engine) = &mut book.engine {
                        &mut engine.variable
                    } else {
                        return Ok(());
                    };
                    if let TextOr::Data(values) =
                        variable.get_or_insert_with(|| TextOr::Data(BTreeMap::new()))
                    {
                        values.insert(r.get(1)?, r.get(2)?);
                    }
                    Ok(())
                },
            )?;
            if book.content_type == "image" {
                s.read("SELECT chapter_id,id,url,width,height FROM chapter_pages WHERE book_id=?1 ORDER BY chapter_id,position",[id],|r| {
                    let owner:String = r.get(0)?;
                    book.chapters[positions[&owner]].pages.push(Page {id:r.get(1)?,src:r.get(2)?,width:r.get(3)?,height:r.get(4)?});
                    Ok(())
                })?;
            }
            if matches!(book.content_type.as_str(), "audio" | "video") {
                s.read("SELECT chapter_id,label,url,protocol,format,unavailable_reason,is_default FROM chapter_streams WHERE book_id=?1 ORDER BY chapter_id,position",[id],|r| {
                    let owner:String = r.get(0)?;
                    book.chapters[positions[&owner]].sources.push(Stream {label:r.get(1)?,src:r.get(2)?,protocol:r.get(3)?,format:r.get(4)?,unavailable_reason:r.get(5)?,selected:r.get(6)?});
                    Ok(())
                })?;
            }
            for chapter in &mut book.chapters {
                if let Some(url) = &chapter.url {
                    let reference = ResourceRef::new(url)?;
                    let format = if reference.path().ends_with(".html") {
                        "richText"
                    } else {
                        book.content_type.as_str()
                    };
                    chapter.resource = Some(crate::models::ChapterResourceDescriptor::for_chapter(
                        &reference, format,
                    ));
                }
            }
            Ok(Some(book))
        })
    }
    pub(crate) fn import_book(&self, book: Book) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            save_record(s, &book)?;
            replace_catalog(s, &book.id, &book.content_type, &book.chapters)?;
            Ok(())
        })
    }
    pub(crate) fn save_information(
        &self,
        mut book: Book,
        chapters: Vec<EngineChapter>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            if cancellation.is_cancelled() {
                return Err(ResourceError::new("详情已取消"));
            }
            let previous = s.read(
                "SELECT id,title FROM chapters WHERE book_id=?1 ORDER BY position",
                [&book.id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?;
            if chapters.is_empty() && !previous.is_empty() {
                return Err(ResourceError::new("书源返回空目录"));
            }
            let mut changed = previous.len() != chapters.len();
            for (index, engine) in chapters.into_iter().enumerate() {
                let id = format!(
                    "chapter-{:016x}",
                    crate::support::stable_hash(&format!("{}\0{}", book.id, engine.url))
                );
                changed |= previous.get(index) != Some(&(id.clone(), engine.title.clone()));
                book.chapters.push(Chapter {
                    id,
                    title: engine.title.clone(),
                    index,
                    volume: engine.is_volume,
                    engine_data: engine,
                    ..Default::default()
                });
            }
            if changed {
                book.updated_at_ms = Some(crate::support::now_ms() as i64);
            }
            save_record(s, &book)?;
            replace_catalog(s, &book.id, &book.content_type, &book.chapters)?;
            if let Some(kind) = book
                .engine
                .as_ref()
                .and_then(|engine| engine.kind.as_deref())
            {
                s.write("INSERT INTO book_tags(book_id,position,tag) SELECT ?1,0,?2 WHERE NOT EXISTS(SELECT 1 FROM book_tags WHERE book_id=?1)",params![book.id,kind])?;
            }
            Ok(())
        })
    }
    pub(crate) fn save_main_text(
        &self,
        book: String,
        chapter: String,
        content: Chapter,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            if cancellation.is_cancelled() {
                return Err(ResourceError::new("正文已取消"));
            }
            let kind: String = s
                .read("SELECT content_type FROM books WHERE id=?1", [&book], |r| {
                    r.get(0)
                })?
                .pop()
                .ok_or_else(|| ResourceError::new("书籍不存在"))?;
            publish(s, &book, &chapter, &kind, &content)
        })
    }
    pub(crate) fn bind_book_type(&self, id: String, kind: String) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            let previous = s
                .read("SELECT content_type FROM books WHERE id=?1", [&id], |r| {
                    r.get::<_, String>(0)
                })?
                .pop();
            if previous.is_some_and(|previous| previous != kind) {
                // 类型变化只停用当前引用；生成书仍拥有旧正文，借用者不受影响。
                s.write(
                    "UPDATE chapters SET content_id=NULL WHERE book_id=?1",
                    [&id],
                )?;
            }
            s.write(
                "UPDATE books SET content_type=?2 WHERE id=?1",
                params![id, kind],
            )?;
            Ok(())
        })
    }
    pub(crate) fn shelve_import(&self, id: String) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            s.write("INSERT INTO library_books(book_id,position) SELECT ?1,coalesce(max(position),-1)+1 FROM library_books WHERE 1 ON CONFLICT(book_id) DO NOTHING",[id])?;
            Ok(())
        })
    }
}

fn save_record(s: &Session<'_>, b: &Book) -> Result<()> {
    s.write("INSERT INTO books(id,content_type,title,author,intro,cover,source_id,word_count,status,original,book_url,page_count,url,updated_at_ms,latest_chapter_title) VALUES(?1,?2,coalesce(?3,'Untitled'),?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
        ON CONFLICT(id) DO UPDATE SET content_type=excluded.content_type,title=coalesce(?3,books.title),author=coalesce(?4,books.author),intro=coalesce(?5,books.intro),cover=coalesce(?6,books.cover),source_id=coalesce(?7,books.source_id),word_count=coalesce(?8,books.word_count),status=coalesce(?9,books.status),original=coalesce(?10,books.original),book_url=coalesce(?11,books.book_url),page_count=coalesce(?12,books.page_count),url=coalesce(?13,books.url),updated_at_ms=coalesce(?14,books.updated_at_ms),latest_chapter_title=coalesce(?15,books.latest_chapter_title)",
        params![b.id,b.content_type,b.title,b.author,b.intro,b.cover,b.source_id,b.word_count,b.status,b.original,b.book_url,b.page_count,b.url,b.updated_at_ms,b.latest_chapter_title])?;
    if let Some(engine) = &b.engine {
        save_engine(s, &b.id, engine)?;
    }
    if let Some(tags) = &b.tags {
        s.write("DELETE FROM book_tags WHERE book_id=?1", [&b.id])?;
        for (position, tag) in tags.iter().enumerate() {
            s.write(
                "INSERT INTO book_tags(book_id,position,tag) VALUES(?1,?2,?3)",
                params![b.id, position as i64, tag],
            )?;
        }
    }
    Ok(())
}

fn save_engine(s: &Session<'_>, id: &str, e: &EngineBook) -> Result<()> {
    s.write("UPDATE books SET book_url=coalesce(?2,book_url),engine_toc_url=coalesce(?3,engine_toc_url),engine_origin=coalesce(?4,engine_origin),engine_origin_name=coalesce(?5,engine_origin_name),engine_name=coalesce(?6,engine_name),engine_author=coalesce(?7,engine_author),engine_kind=coalesce(?8,engine_kind),engine_cover_url=coalesce(?9,engine_cover_url),engine_intro=coalesce(?10,engine_intro),engine_charset=coalesce(?11,engine_charset),engine_type=coalesce(?12,engine_type),engine_latest_chapter_title=coalesce(?13,engine_latest_chapter_title),engine_latest_chapter_time=coalesce(?14,engine_latest_chapter_time),engine_word_count=coalesce(?15,engine_word_count),engine_article_id=coalesce(?16,engine_article_id) WHERE id=?1",
        params![id,e.book_url,e.toc_url,e.origin,e.origin_name,e.name,e.author,e.kind,e.cover_url,e.intro,e.charset,e.r#type,e.latest_chapter_title,e.latest_chapter_time,e.word_count,e.article_id])?;
    variables(s, id, None, &e.variable)
}
fn variables(
    s: &Session<'_>,
    book: &str,
    chapter: Option<&str>,
    variables: &Option<TextOr<BTreeMap<String, String>>>,
) -> Result<()> {
    if let Some(TextOr::Data(values)) = variables {
        s.write(
            "DELETE FROM engine_variables WHERE book_id=?1 AND chapter_id IS ?2",
            params![book, chapter],
        )?;
        for (name, value) in values {
            s.write(
                "INSERT INTO engine_variables(book_id,chapter_id,name,value) VALUES(?1,?2,?3,?4)",
                params![book, chapter, name, value],
            )?;
        }
    }
    Ok(())
}
fn replace_catalog(s: &Session<'_>, book: &str, kind: &str, chapters: &[Chapter]) -> Result<()> {
    s.write(
        "UPDATE chapters SET position=-position-1 WHERE book_id=?1",
        [book],
    )?;
    for (position, chapter) in chapters.iter().enumerate() {
        let e = &chapter.engine_data;
        let changed=s.write("INSERT INTO chapters(book_id,id,position,title,volume,engine_url,engine_book_url,engine_is_vip,engine_is_pay,engine_resource_url,engine_tag,engine_word_count,engine_start,engine_end,engine_start_fragment_id,engine_end_fragment_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(book_id,id) DO UPDATE SET position=excluded.position,title=excluded.title,volume=excluded.volume,engine_url=coalesce(excluded.engine_url,chapters.engine_url),engine_book_url=coalesce(excluded.engine_book_url,chapters.engine_book_url),engine_is_vip=coalesce(excluded.engine_is_vip,chapters.engine_is_vip),engine_is_pay=coalesce(excluded.engine_is_pay,chapters.engine_is_pay),engine_resource_url=coalesce(excluded.engine_resource_url,chapters.engine_resource_url),engine_tag=coalesce(excluded.engine_tag,chapters.engine_tag),engine_word_count=coalesce(excluded.engine_word_count,chapters.engine_word_count),engine_start=coalesce(excluded.engine_start,chapters.engine_start),engine_end=coalesce(excluded.engine_end,chapters.engine_end),engine_start_fragment_id=coalesce(excluded.engine_start_fragment_id,chapters.engine_start_fragment_id),engine_end_fragment_id=coalesce(excluded.engine_end_fragment_id,chapters.engine_end_fragment_id) WHERE chapters.position<0",
            params![book,chapter.id,position as i64,chapter.title,chapter.volume,(!e.url.is_empty()).then_some(&e.url),e.book_url,e.is_vip,e.is_pay,e.resource_url,e.tag,e.word_count,e.start,e.end,e.start_fragment_id,e.end_fragment_id])?;
        if changed == 0 {
            return Err(ResourceError::new("目录包含重复章节"));
        }
        variables(s, book, Some(&chapter.id), &e.variable)?;
        if chapter.content_id.is_none() {
            publish(s, book, &chapter.id, kind, chapter)?;
        }
    }
    s.write("UPDATE bookmarks SET orphaned=1 WHERE book_id=?1 AND chapter_id IN (SELECT id FROM chapters WHERE book_id=?1 AND position<0)",[book])?;
    s.write(
        "DELETE FROM chapters WHERE book_id=?1 AND position<0",
        [book],
    )?;
    s.write("WITH chosen AS (
        SELECT c.id,c.position,c.id IS p.chapter_id AS same FROM reading_progress p JOIN chapters c ON c.book_id=p.book_id
        WHERE p.book_id=?1 AND c.position=coalesce((SELECT position FROM chapters WHERE book_id=p.book_id AND id=p.chapter_id),min(coalesce(p.chapter_index,0),?2))
    ) UPDATE reading_progress SET (chapter_id,chapter_index,offset,page_id,quality,anchor)=(
        SELECT id,position,CASE WHEN same THEN offset ELSE 0 END,CASE WHEN same THEN page_id END,CASE WHEN same THEN quality END,CASE WHEN same THEN anchor END FROM chosen
    ) WHERE book_id=?1 AND EXISTS(SELECT 1 FROM chosen)",params![book,chapters.len().saturating_sub(1) as i64])?;
    s.write("UPDATE reading_progress SET chapter_id=NULL WHERE book_id=?1 AND NOT EXISTS(SELECT 1 FROM chapters c WHERE c.book_id=?1 AND c.id=reading_progress.chapter_id)",[book])?;
    purge(s)
}
fn uri(kind: &str, book: &str, value: &str) -> Result<String> {
    let uri = if value.contains("://") {
        value.to_owned()
    } else {
        format!("resource://cache/{kind}/{book}/{value}")
    };
    Ok(ResourceRef::new(uri)?.as_str().to_owned())
}
fn publish(s: &Session<'_>, book: &str, chapter: &str, kind: &str, body: &Chapter) -> Result<()> {
    let address = body
        .resource
        .as_ref()
        .map(|resource| resource.resource_id.as_str())
        .or(body.url.as_deref());
    if address.is_none() && body.pages.is_empty() && body.sources.is_empty() {
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    s.write("UPDATE chapter_contents SET producer_book_id=NULL,producer_chapter_id=NULL WHERE producer_book_id=?1 AND producer_chapter_id=?2",params![book,chapter])?;
    let address = address.map(|value| uri(kind, book, value)).transpose()?;
    s.write("INSERT INTO chapter_contents(id,producer_book_id,producer_chapter_id,kind,text_url,duration_ms,protocol,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,book,chapter,kind,address,body.duration_ms,body.protocol,crate::support::now_ms()])?;
    for (position, page) in body.pages.iter().enumerate() {
        let address = uri(kind, book, &page.src)?;
        s.write("INSERT INTO content_pages(content_id,position,id,url,width,height) VALUES(?1,?2,?3,?4,?5,?6)",params![id,position as i64,if page.id.is_empty(){&page.src}else{&page.id},address,page.width,page.height])?;
    }
    let selected = body
        .sources
        .iter()
        .position(|source| source.selected && source.src.is_some())
        .or_else(|| body.sources.iter().position(|source| source.src.is_some()));
    for (position, source) in body.sources.iter().enumerate() {
        let address = source
            .src
            .as_ref()
            .map(|value| uri(kind, book, value))
            .transpose()?;
        s.write("INSERT INTO content_streams(content_id,position,label,url,protocol,format,unavailable_reason,is_default) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,position as i64,source.label,address,source.protocol,source.format,source.unavailable_reason,selected==Some(position)])?;
    }
    for (position, file) in body.files.iter().enumerate() {
        s.write(
            "INSERT INTO content_files(content_id,position,url) VALUES(?1,?2,?3)",
            params![id, position as i64, uri(kind, book, file)?],
        )?;
    }
    s.write("INSERT OR IGNORE INTO content_media(content_id,media_id) SELECT ?1,m.id FROM media_requests m JOIN content_assets a ON a.uri='resource://proxy/'||m.id WHERE a.content_id=?1",[&id])?;
    if s.write(
        "UPDATE chapters SET content_id=?3 WHERE book_id=?1 AND id=?2",
        params![book, chapter, id],
    )? == 0
    {
        return Err(ResourceError::new("章节不存在"));
    }
    purge(s)
}
pub(super) fn detach(s: &Session<'_>, book: &str) -> Result<usize> {
    let count = s.write(
        "UPDATE chapters SET content_id=NULL WHERE book_id=?1 AND content_id IS NOT NULL",
        [book],
    )?;
    s.write("UPDATE chapter_contents SET producer_book_id=NULL,producer_chapter_id=NULL WHERE producer_book_id=?1",[book])?;
    purge(s)?;
    Ok(count)
}
pub(super) fn purge(s: &Session<'_>) -> Result<()> {
    s.write("INSERT OR IGNORE INTO pending_cleanup(path) SELECT substr(a.uri,12) FROM content_assets a JOIN chapter_contents b ON b.id=a.content_id WHERE a.uri LIKE 'resource://cache/%' AND b.producer_book_id IS NULL AND NOT EXISTS(SELECT 1 FROM chapters c WHERE c.content_id=b.id)",[])?;
    s.write("DELETE FROM chapter_contents WHERE producer_book_id IS NULL AND NOT EXISTS(SELECT 1 FROM chapters c WHERE c.content_id=chapter_contents.id)",[])?;
    s.write("DELETE FROM media_requests WHERE book_id IS NULL AND NOT EXISTS(SELECT 1 FROM content_media c WHERE c.media_id=media_requests.id)",[])?;
    Ok(())
}
pub(super) fn save_candidate(s: &Session<'_>, input: OnlineBookInput) -> Result<String> {
    let kind = s
        .read(
            "SELECT content_type FROM sources WHERE id=?1",
            [&input.source_id],
            |r| r.get::<_, String>(0),
        )?
        .pop()
        .ok_or_else(|| ResourceError::new("书源不存在"))?;
    let id = online_book_id(&input.source_id, &kind, &input.book_url);
    s.write("INSERT INTO books(id,content_type,source_id,book_url,title,author,cover,latest_chapter_title,engine_name,engine_author) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?5,?6) ON CONFLICT(id) DO NOTHING",params![id,kind,input.source_id,input.book_url,input.title,input.author,input.cover,input.latest])?;
    if let Some(engine) = input.engine {
        save_engine(s, &id, &engine)?;
    }
    Ok(id)
}
pub(super) fn use_content(s: &Session<'_>, book: &str, chapter: &str, content: &str) -> Result<()> {
    if s.write("UPDATE chapters SET content_id=?3 WHERE book_id=?1 AND id=?2 AND EXISTS(SELECT 1 FROM books b JOIN chapter_contents x ON x.kind=b.content_type WHERE b.id=?1 AND x.id=?3)",params![book,chapter,content])? == 0 {return Err(ResourceError::new("正文与章节不匹配"));}
    s.write(
        "UPDATE reading_progress SET page_id=NULL,quality=NULL WHERE book_id=?1 AND chapter_id=?2",
        params![book, chapter],
    )?;
    Ok(())
}
pub(super) fn patch_book(s: &Session<'_>, id: &str, patch: BookPatch) -> Result<()> {
    let (title_set, title) = patch.title.parts();
    let (author_set, author) = patch.author.parts();
    let (intro_set, intro) = patch.intro.parts();
    let (cover_set, cover) = patch.cover.parts();
    if s.write("UPDATE books SET title_override=CASE WHEN ?2 THEN ?3 ELSE title_override END,author_override=CASE WHEN ?4 THEN ?5 ELSE author_override END,intro_override=CASE WHEN ?6 THEN ?7 ELSE intro_override END,cover_override=CASE WHEN ?8 THEN ?9 ELSE cover_override END WHERE id=?1",params![id,title_set,title,author_set,author,intro_set,intro,cover_set,cover])? == 0 {return Err(ResourceError::new("书籍不存在"));}
    if let Some(count) = patch.page_count {
        if s.write(
            "UPDATE books SET page_count=?2 WHERE id=?1 AND content_type='pdf'",
            params![id, count],
        )? == 0
        {
            return Err(ResourceError::new("只有 PDF 可以修改页数"));
        }
    }
    if let Some(tags) = patch.tags {
        s.write("DELETE FROM book_tags WHERE book_id=?1", [id])?;
        for (position, tag) in tags.into_iter().enumerate() {
            s.write(
                "INSERT INTO book_tags(book_id,position,tag) VALUES(?1,?2,?3)",
                params![id, position as i64, tag],
            )?;
        }
    }
    if let Some(alternatives) = patch.alternatives {
        s.write("DELETE FROM book_alternatives WHERE book_id=?1", [id])?;
        for (position, alternative) in alternatives.into_iter().enumerate() {
            s.write(
                "INSERT INTO book_alternatives(book_id,alternative_id,position) VALUES(?1,?2,?3)",
                params![id, alternative, position as i64],
            )?;
        }
    }
    Ok(())
}
