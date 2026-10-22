//! 正文统一入口：请求后入库，缓存没有就新增，有就更新。
use std::sync::Arc;

use crate::{
    source_engine::SourceEngineRequest,
    support::{text_at, validate_id},
};

pub(crate) struct MainText {
    pub(crate) sources: Arc<crate::source_catalog::SourceCatalog>,
    pub(crate) executor: Arc<dyn crate::source_engine::SourceExecutor>,
    pub(crate) text_cancel: tokio::sync::broadcast::Sender<Option<(String, String)>>,
    pub(crate) backup: Arc<crate::backup::state::Operations>,
}
impl MainText {
    pub(crate) fn new(
        sources: Arc<crate::source_catalog::SourceCatalog>,
        executor: Arc<dyn crate::source_engine::SourceExecutor>,
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
            let book = crate::book_files::read_book(database, book_id)
                .await
                .map_err(|error| error.to_string())?;
            let index = book["chapters"]
                .as_array()
                .and_then(|chapters| {
                    chapters
                        .iter()
                        .position(|chapter| chapter["id"] == chapter_id)
                })
                .ok_or("Chapter is no longer in the book directory")?;
            if book["chapters"][index]["volume"] == true {
                return Ok(());
            }
            let Some(source_id) = book["sourceId"].as_str() else {
                return if crate::book_files::indexed_chapter_ready(resources, &book, index).await? {
                    Ok(())
                } else {
                    Err("Local chapter resource is missing; re-import the original file".into())
                };
            };
            let source = self.sources.get(source_id).await?;
            if !source.enabled {
                return Err("Book source is disabled".into());
            }
            let mut request = SourceEngineRequest {
                chapter: Some(book["chapters"][index]["engineData"].clone()),
                next_chapter_url: book["chapters"]
                    .get(index + 1)
                    .and_then(|chapter| text_at(&chapter["engineData"], &["url"])),
                ..SourceEngineRequest::new(
                    if crate::rss::is_legacy_rss_source(&source.source) {
                        "rssContent"
                    } else {
                        "content"
                    },
                    &source.source,
                )
            }
            .with_book(book["engine"].clone());
            request.cancellation = Some(token.clone());
            let content = self.executor.execute(request.clone()).await?;

            let mut chapter = crate::chapter_resources::chapter_content(
                database,
                resources,
                self.executor.as_ref(),
                book_id,
                chapter_id,
                &request,
                content,
                book["chapters"][index].clone(),
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
                .map_err(|e| e.to_string())
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
