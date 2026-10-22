//! 阅读、书架、配置、RSS 和书源键值的固定业务操作。
use super::*;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum Mutation {
    SaveSources {
        sources: Vec<SourceInput>,
    },
    SaveBooks {
        books: Vec<OnlineBookInput>,
    },
    SaveBook {
        book_id: String,
        changes: BookPatch,
    },
    UseChapterContent {
        book_id: String,
        chapter_id: String,
        content_id: String,
    },
    SaveProgress {
        book_id: String,
        progress: ProgressInput,
    },
    ResetProgress {
        book_id: String,
    },
    SaveShelf {
        shelf: crate::database::ShelfDocument,
    },
    SaveReplacementRules {
        rules: Vec<crate::reading_tools::DisplayReplacementRule>,
    },
    DeleteReplacementRule {
        id: String,
    },
    SaveTxtTocRules {
        rules: Vec<crate::local_books::txt_toc_rules::TxtTocRule>,
    },
    DeleteTxtTocRule {
        id: String,
    },
    SaveHttpTts {
        configs: Vec<TtsInput>,
    },
    DeleteHttpTts {
        id: String,
    },
    SaveBookmark {
        book_id: String,
        bookmark: crate::reading_tools::Bookmark,
    },
    DeleteBookmark {
        book_id: String,
        id: String,
    },
    RecordSession {
        session: crate::reading_tools::ReadingSession,
    },
    ClearSearchHistory {
        query: Option<String>,
    },
}
impl Database {
    /// 前端固定操作批次共用一个事务，不接收 SQL 或表字段。
    pub(crate) fn submit(&self, operations: Vec<Mutation>) -> Result<Pending<Vec<ReadingUpdate>>> {
        db::write(self, WriteMode::Transaction, move |s| {
            let mut updates = Vec::new();
            for operation in operations {
                match operation {
                    Mutation::SaveSources { sources } => {
                        for source in sources {
                            catalog::save_source(s, source)?;
                        }
                    }
                    Mutation::SaveBooks { books } => {
                        for book in books {
                            library::save_candidate(s, book)?;
                        }
                    }
                    Mutation::SaveBook { book_id, changes } => {
                        library::patch_book(s, &book_id, changes)?
                    }
                    Mutation::UseChapterContent {
                        book_id,
                        chapter_id,
                        content_id,
                    } => library::use_content(s, &book_id, &chapter_id, &content_id)?,
                    Mutation::SaveProgress { book_id, progress } => {
                        if let Some(update) = save_progress(s, book_id, progress)? {
                            updates.push(update);
                        }
                    }
                    Mutation::ResetProgress { book_id } => {
                        s.write("DELETE FROM reading_progress WHERE book_id=?1", [book_id])?;
                    }
                    Mutation::SaveShelf { shelf } => save_shelf(s, shelf)?,
                    Mutation::SaveReplacementRules { rules } => {
                        for rule in rules {
                            s.write("INSERT INTO replacement_rules(id,position,name,pattern,replacement,enabled,is_regex,scope)
                                VALUES(?1,coalesce((SELECT position FROM replacement_rules WHERE id=?1),(SELECT coalesce(max(position),-1)+1 FROM replacement_rules)),?2,?3,?4,?5,?6,?7)
                                ON CONFLICT(id) DO UPDATE SET name=excluded.name,pattern=excluded.pattern,replacement=excluded.replacement,enabled=excluded.enabled,is_regex=excluded.is_regex,scope=excluded.scope",
                                params![rule.id,rule.name,rule.pattern,rule.replacement,rule.enabled,rule.is_regex,rule.scope])?;
                        }
                    }
                    Mutation::DeleteReplacementRule { id } => {
                        s.write("DELETE FROM replacement_rules WHERE id=?1", [id])?;
                    }
                    Mutation::SaveTxtTocRules { rules } => {
                        for rule in rules {
                            s.write("INSERT INTO txt_toc_rules(id,position,name,rule,example,serial_number,enable)
                                VALUES(?1,coalesce((SELECT position FROM txt_toc_rules WHERE id=?1),(SELECT coalesce(max(position),-1)+1 FROM txt_toc_rules)),?2,?3,?4,?5,?6)
                                ON CONFLICT(id) DO UPDATE SET name=excluded.name,rule=excluded.rule,example=excluded.example,serial_number=excluded.serial_number,enable=excluded.enable",
                                params![rule.id,rule.name,rule.rule,rule.example,rule.serial_number,rule.enable])?;
                        }
                    }
                    Mutation::DeleteTxtTocRule { id } => {
                        s.write("DELETE FROM txt_toc_rules WHERE id=?1", [id])?;
                    }
                    Mutation::SaveHttpTts { configs } => {
                        for config in configs {
                            catalog::save_tts(s, config)?;
                        }
                    }
                    Mutation::DeleteHttpTts { id } => {
                        s.write("DELETE FROM http_tts_configs WHERE id=?1", [id])?;
                    }
                    Mutation::SaveBookmark { book_id, bookmark } => {
                        save_bookmark(s, &book_id, bookmark)?
                    }
                    Mutation::DeleteBookmark { book_id, id } => {
                        s.write(
                            "DELETE FROM bookmarks WHERE book_id=?1 AND id=?2",
                            params![book_id, id],
                        )?;
                    }
                    Mutation::RecordSession { session } => record_time(s, session)?,
                    Mutation::ClearSearchHistory { query } => {
                        s.write(
                            "DELETE FROM search_history WHERE ?1 IS NULL OR query=?1",
                            [query],
                        )?;
                    }
                }
            }
            Ok(updates)
        })
    }
    pub(crate) fn record_reading_time(
        &self,
        session: crate::reading_tools::ReadingSession,
    ) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            record_time(s, session)
        })
    }
    pub(crate) fn save_reading_progress(
        &self,
        book_id: String,
        progress: ProgressInput,
    ) -> Result<Pending<Option<ReadingUpdate>>> {
        db::write(self, WriteMode::Transaction, move |s| {
            save_progress(s, book_id, progress)
        })
    }
    pub(crate) fn save_bookmark(
        &self,
        book_id: String,
        bookmark: crate::reading_tools::Bookmark,
    ) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            save_bookmark(s, &book_id, bookmark)
        })
    }
    pub(crate) fn record_search(&self, query: String, time: u64) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            let query = query.trim();
            let normalized = query
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            s.write("INSERT INTO search_history(normalized_query,query,usage,first_use_time_ms,last_use_time_ms) VALUES(?1,?2,1,?3,?3)
                ON CONFLICT(normalized_query) DO UPDATE SET query=excluded.query,usage=usage+1,
                first_use_time_ms=min(first_use_time_ms,excluded.first_use_time_ms),last_use_time_ms=max(last_use_time_ms,excluded.last_use_time_ms)",params![normalized,query,time])?;
            s.write("DELETE FROM search_history WHERE normalized_query NOT IN(SELECT normalized_query FROM search_history ORDER BY last_use_time_ms DESC,usage DESC,normalized_query LIMIT 100)",[])?;
            Ok(())
        })
    }
    pub(crate) fn txt_toc_rules(
        &self,
    ) -> Result<crate::local_books::txt_toc_rules::TxtTocRulesDocument> {
        db::read(self, |s| {
            Ok(crate::local_books::txt_toc_rules::TxtTocRulesDocument {
            schema_version:1,
            rules:s.read("SELECT id,name,rule,example,serial_number,enable FROM txt_toc_rules ORDER BY position",[],|r|Ok(crate::local_books::txt_toc_rules::TxtTocRule {
                id:r.get(0)?,name:r.get(1)?,rule:r.get(2)?,example:r.get(3)?,serial_number:r.get(4)?,enable:r.get(5)?,
            }))?,
        })
        })
    }
    pub(crate) fn ensure_rss_subscription(&self, source: String) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            s.write(
                "INSERT OR IGNORE INTO rss_subscriptions(source_id,filter) VALUES(?1,'all')",
                [source],
            )?;
            Ok(())
        })
    }
    pub(crate) fn save_rss_articles(
        &self,
        source: String,
        articles: Vec<RssArticlePatch>,
    ) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            for article in articles {
                if article.is_read.is_none() && article.is_favorite.is_none() && article.content_ref.is_none() {
                    s.write("INSERT OR IGNORE INTO rss_articles(source_id,article_id,is_read,is_favorite,updated_at_ms) VALUES(?1,?2,0,0,?3)",params![source,article.article_id,crate::support::now_ms()])?;
                } else if s.write("UPDATE rss_articles SET is_read=coalesce(?3,is_read),is_favorite=coalesce(?4,is_favorite),content_ref=coalesce(?5,content_ref),updated_at_ms=?6 WHERE source_id=?1 AND article_id=?2",params![source,article.article_id,article.is_read,article.is_favorite,article.content_ref,crate::support::now_ms()])?==0 {
                    return Err(ResourceError::new("RSS 文章不存在"));
                }
            }
            Ok(())
        })
    }
    pub(crate) fn rss_state(&self) -> Result<crate::rss::RssStateDocument> {
        db::read(self, |s| {
            Ok(crate::rss::RssStateDocument {
            schema_version:1,
            subscriptions:s.read("SELECT source_id,filter FROM rss_subscriptions ORDER BY source_id",[],|r|Ok(crate::rss::RssSubscriptionState {source_id:r.get(0)?,filter:r.get(1)?}))?,
            articles:s.read("SELECT source_id,article_id,is_read,is_favorite,updated_at_ms,content_ref FROM rss_articles ORDER BY source_id,article_id",[],|r|Ok(crate::rss::RssArticleState {
                source_id:r.get(0)?,article_id:r.get(1)?,is_read:r.get(2)?,is_favorite:r.get(3)?,updated_at_ms:r.get(4)?,content_ref:r.get(5)?,
            }))?,
        })
        })
    }
    pub(crate) fn save_discovery_categories(
        &self,
        source: String,
        categories: Vec<crate::discovery::DiscoveryCategory>,
        targets: Vec<crate::discovery::PrivateCategory>,
    ) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            let targets: HashMap<_, _> = targets
                .into_iter()
                .map(|target| (target.category_id.clone(), target))
                .collect();
            s.write(
                "DELETE FROM discovery_categories WHERE source_id=?1",
                [&source],
            )?;
            for (position, category) in categories.into_iter().enumerate() {
                let target = category.category_id.as_ref().and_then(|id| targets.get(id));
                let style = category.style.as_ref();
                s.write("INSERT INTO discovery_categories(source_id,position,category_id,title,kind,url,request_kind,style_cols,style_rows,style_width) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![source,position as i64,category.category_id,category.title,category.kind,target.and_then(|value|value.url.as_ref()),target.map(|value|&value.kind),style.and_then(|style|style.cols),style.and_then(|style|style.rows),style.and_then(|style|style.layout_flex_basis_percent)])?;
            }
            Ok(())
        })
    }
    pub(crate) fn discovery_category(
        &self,
        source: &str,
        id: &str,
    ) -> Result<Option<crate::discovery::PrivateCategory>> {
        db::read(self, |s| {
            Ok(s.read("SELECT category_id,title,url,request_kind FROM discovery_categories WHERE source_id=?1 AND category_id=?2 AND request_kind IS NOT NULL",params![source,id],|r|Ok(crate::discovery::PrivateCategory {
            category_id:r.get(0)?,title:r.get(1)?,url:r.get(2)?,kind:r.get(3)?,
        }))?.pop())
        })
    }
}
fn record_time(s: &Session<'_>, session: crate::reading_tools::ReadingSession) -> Result<()> {
    s.write("INSERT INTO reading_sessions(session_id,book_id,duration_ms,ended_at_ms,utc_offset_minutes) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(session_id) DO NOTHING",params![session.session_id,session.book_id,session.duration_ms,session.ended_at_ms,session.utc_offset_minutes])?;
    Ok(())
}
pub(super) fn progress_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProgressInput> {
    Ok(ProgressInput {
        chapter_id: r.get("chapter_id")?,
        chapter_index: r.get("chapter_index")?,
        offset: r.get("offset")?,
        updated_at_ms: r.get("updated_at_ms")?,
        page_id: r.get("page_id")?,
        anchor: r.get("anchor")?,
        quality: r.get("quality")?,
    })
}
fn save_progress(
    s: &Session<'_>,
    book: String,
    mut progress: ProgressInput,
) -> Result<Option<ReadingUpdate>> {
    let previous = s
        .read(
            "SELECT * FROM reading_progress WHERE book_id=?1",
            [&book],
            progress_row,
        )?
        .pop();
    let time = progress
        .updated_at_ms
        .unwrap_or_else(crate::support::now_ms);
    if previous
        .as_ref()
        .and_then(|previous| previous.updated_at_ms)
        .is_some_and(|old| old > time)
    {
        return Ok(None);
    }
    let (kind, pages) = s
        .read(
            "SELECT content_type,coalesce(page_count,0) FROM books WHERE id=?1",
            [&book],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)),
        )?
        .pop()
        .ok_or_else(|| ResourceError::new("书籍不存在"))?;
    let (chapter, read) = if kind == "pdf" {
        if progress.chapter_index >= pages {
            return Err(ResourceError::new("阅读位置超出 PDF 页数"));
        }
        (
            format!("page-{}", progress.chapter_index + 1),
            progress.chapter_index + 1,
        )
    } else {
        s.read(
            "SELECT id,read_position FROM chapter_positions WHERE book_id=?1 AND position=?2",
            params![book, progress.chapter_index],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)),
        )?
        .pop()
        .ok_or_else(|| ResourceError::new("章节不存在"))?
    };
    if progress
        .chapter_id
        .as_ref()
        .is_some_and(|id| *id != chapter)
    {
        return Err(ResourceError::new("章节 ID 与顺序不一致"));
    }
    progress.chapter_id = (kind != "pdf").then_some(chapter);
    progress.updated_at_ms = Some(time);
    let chapter_changed = previous.as_ref().is_none_or(|old| {
        old.chapter_index != progress.chapter_index || old.chapter_id != progress.chapter_id
    });
    s.write("INSERT INTO reading_progress(book_id,chapter_id,chapter_index,offset,page_id,anchor,quality,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
        ON CONFLICT(book_id) DO UPDATE SET chapter_id=excluded.chapter_id,chapter_index=excluded.chapter_index,offset=excluded.offset,page_id=excluded.page_id,anchor=excluded.anchor,quality=excluded.quality,updated_at_ms=excluded.updated_at_ms",
        params![book,progress.chapter_id,progress.chapter_index,progress.offset,progress.page_id,progress.anchor,progress.quality,time])?;
    Ok(Some(ReadingUpdate {
        book_id: book,
        progress,
        read,
        chapter_changed,
    }))
}
fn save_bookmark(s: &Session<'_>, book: &str, b: crate::reading_tools::Bookmark) -> Result<()> {
    s.write("INSERT INTO bookmarks(id,book_id,position,chapter_id,chapter_index,chapter_title,offset,page_id,anchor,note,orphaned,created_at_ms,updated_at_ms)
        VALUES(?1,?2,coalesce((SELECT position FROM bookmarks WHERE id=?1),(SELECT coalesce(max(position),-1)+1 FROM bookmarks WHERE book_id=?2)),?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
        ON CONFLICT(id) DO UPDATE SET book_id=excluded.book_id,chapter_id=excluded.chapter_id,chapter_index=excluded.chapter_index,chapter_title=excluded.chapter_title,offset=excluded.offset,page_id=excluded.page_id,anchor=excluded.anchor,note=excluded.note,orphaned=excluded.orphaned,updated_at_ms=excluded.updated_at_ms",
        params![b.id,book,b.chapter_id,b.chapter_index as i64,b.chapter_title,b.offset,b.page_id,b.anchor,b.note,b.orphaned,b.created_at_ms,b.updated_at_ms])?;
    Ok(())
}
fn save_shelf(s: &Session<'_>, shelf: crate::database::ShelfDocument) -> Result<()> {
    s.write("DELETE FROM library_books", [])?;
    s.write("DELETE FROM library_groups", [])?;
    for (position, group) in shelf.groups.into_iter().enumerate() {
        s.write(
            "INSERT INTO library_groups(name,position) VALUES(?1,?2)",
            params![group, position as i64],
        )?;
    }
    for (position, book) in shelf.books.into_iter().enumerate() {
        s.write(
            "INSERT INTO library_books(book_id,position) VALUES(?1,?2)",
            params![book.id, position as i64],
        )?;
        for group in book.groups {
            s.write(
                "INSERT INTO library_group_books(group_name,book_id) VALUES(?1,?2)",
                params![group, book.id],
            )?;
        }
    }
    Ok(())
}

/// 原生插件的固定键值协议，查询不修改过期记录，删除直接返回实际删除的键。
pub(crate) fn storage(
    request: crate::source_storage::SourceStorageRequest,
    directory: &Path,
) -> Result<(crate::source_storage::SourceStorageResponse, Vec<String>)> {
    use crate::source_storage::{required_key, SourceStorageResponse};
    let root = if directory
        .file_name()
        .is_some_and(|name| name == "source-engine")
    {
        directory.parent().unwrap_or(directory)
    } else {
        directory
    };
    let database = Database {
        root: Arc::new(root.to_owned()),
    };
    let now = crate::support::now_ms().min(i64::MAX as u64) as i64;
    if request.operation == "get" {
        let value = if !database.file_path().is_file() {
            None
        } else {
            db::read(&database, |s| {
                match request.key {
                    Some(key)=>Ok(s.read("SELECT value FROM source_kv WHERE namespace=?1 AND key=?2 AND (expires_at_ms IS NULL OR expires_at_ms>?3)",params![request.namespace,key,now],|r|r.get(0))?.pop()),
                    None=>Ok(None),
                }
            })?
        };
        return Ok((SourceStorageResponse { value }, Vec::new()));
    }
    let scopes=db::write(&database,WriteMode::Transaction,move |s| {
        let mut scopes=Vec::new();
        match request.operation.as_str() {
            "put" => {
                let key=required_key(&request).map_err(ResourceError::new)?;
                let value=request.value.as_deref().ok_or_else(||ResourceError::new("键值写入缺少值"))?;
                let expiry=request.ttl_seconds.filter(|seconds|*seconds>0).map(|seconds|now.saturating_add(seconds.saturating_mul(1000).min(i64::MAX as u64) as i64));
                s.write("INSERT INTO source_kv(namespace,key,value,expires_at_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,expires_at_ms=excluded.expires_at_ms WHERE source_kv.value!=excluded.value OR source_kv.expires_at_ms IS NOT excluded.expires_at_ms",params![request.namespace,key,value,expiry])?;
            }
            "delete" => {
                let key=required_key(&request).map_err(ResourceError::new)?;
                let deleted=s.read("DELETE FROM source_kv WHERE namespace=?1 AND key=?2 RETURNING key",params![request.namespace,key],|r|r.get::<_,String>(0))?;
                if request.namespace=="cookies" {scopes.extend(deleted);}
            }
            "clear"|"deletePrefixes"|"deleteContains" => {
                let (mode,patterns)=match request.operation.as_str() {
                    "deletePrefixes"=>(1,request.key_prefixes.ok_or_else(||ResourceError::new("缺少前缀"))?),
                    "deleteContains"=>(2,vec![request.contains.ok_or_else(||ResourceError::new("缺少匹配文本"))?]),
                    _=>(0,vec![String::new()]),
                };
                for pattern in patterns {
                    let deleted=s.read("DELETE FROM source_kv WHERE namespace=?1 AND (?2=0 OR (?2=1 AND substr(key,1,length(?3))=?3) OR (?2=2 AND instr(key,?3)>0)) RETURNING key",params![request.namespace,mode,pattern],|r|r.get::<_,String>(0))?;
                    if request.namespace=="cookies" {scopes.extend(deleted);}
                }
            }
            _=>return Err(ResourceError::new("未知键值操作")),
        }
        Ok(scopes)
    })?.wait()?;
    Ok((SourceStorageResponse { value: None }, scopes))
}
