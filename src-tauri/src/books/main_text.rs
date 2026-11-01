//! 正文统一入口：请求后入库，缓存没有就新增，有就更新。
use std::sync::Arc;

use crate::support::validate_id;
use tauri_plugin_source_engine::{BookRequest, ContentRequest};

pub(crate) enum ChapterPreparation {
    Updated,
    Cached,
    Stale,
}

pub(crate) struct MainText {
    pub(crate) sources: Arc<crate::sources::Catalog>,
    pub(crate) executor: Arc<dyn tauri_plugin_source_engine::SourceExecutor>,
    pub(crate) text_cancel: tokio::sync::broadcast::Sender<Option<(String, String)>>,
    pub(crate) backup: Arc<crate::backup::state::Operations>,
}
impl MainText {
    pub(crate) fn new(
        sources: Arc<crate::sources::Catalog>,
        executor: Arc<dyn tauri_plugin_source_engine::SourceExecutor>,
        backup: Arc<crate::backup::state::Operations>,
    ) -> Self {
        Self {
            sources,
            executor,
            backup,
            text_cancel: tokio::sync::broadcast::channel(64).0,
        }
    }
}

/// 文本始终缓存；cache 仅决定是否下载完整图片和音视频。Vue 等待完成后读取缓存。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn main_text(
    database: tauri::State<'_, crate::database::Database>,
    resources: tauri::State<'_, Arc<crate::resources::Resources>>,
    book_id: String,
    chapter_id: String,
    cache: bool,
    service: tauri::State<'_, Arc<MainText>>,
) -> Result<(), String> {
    service
        .main_text(&database, &resources, &book_id, &chapter_id, cache)
        .await
}

/// 按书籍和章节取消，不影响其他章节或后续请求。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) fn cancel_main_text(
    book_id: String,
    chapter_id: String,
    service: tauri::State<'_, Arc<MainText>>,
) {
    service.cancel_main_text(&book_id, &chapter_id);
}

impl MainText {
    pub(crate) fn cancel_main_text(&self, book_id: &str, chapter_id: &str) {
        let _ = self
            .text_cancel
            .send(Some((book_id.into(), chapter_id.into())));
    }

    pub(crate) async fn main_text(
        &self,
        database: &crate::database::Database,
        resources: &crate::resources::Resources,
        book_id: &str,
        chapter_id: &str,
        cache: bool,
    ) -> Result<(), String> {
        self.prepare_chapter(database, resources, book_id, chapter_id, cache, None)
            .await
            .map(|_| ())
    }

    pub(crate) async fn prepare_chapter(
        &self,
        database: &crate::database::Database,
        resources: &crate::resources::Resources,
        book_id: &str,
        chapter_id: &str,
        cache: bool,
        reading_index: Option<usize>,
    ) -> Result<ChapterPreparation, String> {
        let token = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = token.clone().drop_guard();
        let mut cancellation = self.text_cancel.subscribe();
        let cancelled = async {
            loop {
                match cancellation.recv().await {
                    Ok(None) => break,
                    Ok(Some((book, chapter))) if book == book_id && chapter == chapter_id => break,
                    _ => {}
                }
            }
        };
        let operation = async {
            validate_id(book_id, "bookId")?;
            validate_id(chapter_id, "chapterId")?;
            let db = database.clone();
            let (book, chapter) = (book_id.to_owned(), chapter_id.to_owned());
            let context = tokio::task::spawn_blocking(move || db.chapter_context(&book, &chapter))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?
                .ok_or("Chapter is no longer in the book directory")?;
            if reading_index.is_some_and(|index| context.progress_index != Some(index)) {
                return Ok(ChapterPreparation::Stale);
            }
            if context.chapter.volume {
                return Ok(ChapterPreparation::Cached);
            }
            if (reading_index.is_some() || context.source_id.is_none())
                && crate::book_files::chapter_ready(resources, &context).await?
            {
                return Ok(ChapterPreparation::Cached);
            }
            let Some(source_id) = context.source_id.as_deref() else {
                return Err(
                    "Local chapter resource is missing; re-import the original file".into(),
                );
            };
            let source = self.sources.get(source_id).await?;
            if !source.enabled {
                return Err("Book source is disabled".into());
            }
            let book = BookRequest::new((&source.source).try_into()?, context.engine.ok_or("Source book context is missing")?);
            let request = ContentRequest {
                source: book.source, book: book.book,
                chapter: context.chapter.engine_data.clone(),
                next_chapter_url: context.next_chapter_url,
            };
            let content = self.executor.content(request.clone(), token.clone()).await?.content;

            let mut chapter = crate::chapter_resources::chapter_content(
                database,
                resources,
                self.executor.as_ref(),
                book_id,
                chapter_id,
                &request,
                content,
                serde_json::to_value(context.chapter).map_err(|error| error.to_string())?,
                token.clone(),
            )
            .await?;
            if cache {
                chapter = crate::chapter_resources::cache_chapter_files(
                    database, resources, book_id, chapter_id, chapter,
                )
                .await?;
            }
            // 只提交这一章的缓存字段，不重写目录和进度。
            let database = database.clone();
            let (book_id, chapter_id, token) =
                (book_id.to_owned(), chapter_id.to_owned(), token.clone());
            database
                .save_main_text(
                    book_id,
                    chapter_id,
                    serde_json::from_value(chapter).map_err(|error| error.to_string())?,
                    token,
                )
                .map_err(|e| e.to_string())?
                .await
                .map_err(|e| e.to_string())?;
            Ok(ChapterPreparation::Updated)
        };
        tokio::select! {
            biased;
            _ = cancelled => Err("Chapter content cancelled".into()),
            result = operation => result,
        }
    }
}

impl MainText {
    pub(crate) fn cancel_all(&self) {
        let _ = self.text_cancel.send(None);
    }
}
