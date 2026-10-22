//! 正文统一入口：请求后入库，缓存没有就新增，有就更新。
use super::ApplicationService;
use crate::{
    resources::ResourceRef,
    source_engine::SourceEngineRequest,
    support::{text_at, validate_id},
};
use serde_json::Value;

#[cfg(test)]
#[path = "tests/main_text_tests.rs"]
mod tests;

/// 文本始终缓存；cache 仅决定是否下载完整图片和音视频。Vue 等待完成后读取缓存。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn main_text(
    book_id: String,
    chapter_id: String,
    cache: bool,
    service: tauri::State<'_, ApplicationService>,
) -> Result<(), String> {
    service.main_text(&book_id, &chapter_id, cache).await
}

/// 按书籍和章节取消，不影响其他章节或后续请求。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub fn cancel_main_text(
    book_id: String,
    chapter_id: String,
    service: tauri::State<'_, ApplicationService>,
) {
    service.cancel_main_text(&book_id, &chapter_id);
}

impl ApplicationService {
    pub(crate) fn cancel_main_text(&self, book_id: &str, chapter_id: &str) {
        let _ = self
            .text_cancel
            .send(Some((book_id.into(), chapter_id.into())));
    }

    pub(crate) async fn main_text(
        &self,
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
            let book = self
                .read_book(book_id)
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
                return if self.indexed_chapter_ready(&book, index).await? {
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
                self,
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
                    self, book_id, chapter_id, chapter,
                )
                .await?;
            }
            // 只提交这一章的缓存字段，不重写目录和进度。
            let database = self.database.clone();
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

    /// 阅读预加载先检查缓存；显式正文请求则直接刷新。
    pub(crate) async fn indexed_chapter_ready(
        &self,
        book: &Value,
        index: usize,
    ) -> Result<bool, String> {
        let chapter = &book["chapters"][index];
        let kind = book["contentType"].as_str().unwrap_or("text");
        let addresses = match kind {
            "image" => {
                let Some(pages) = chapter["pages"]
                    .as_array()
                    .filter(|pages| !pages.is_empty())
                else {
                    return Ok(false);
                };
                pages
                    .iter()
                    .map(|page| page["url"].as_str())
                    .collect::<Option<Vec<_>>>()
            }
            "pdf" => book["url"].as_str().map(|src| vec![src]),
            _ => chapter["url"].as_str().map(|src| vec![src]),
        };
        let Some(addresses) = addresses else {
            return Ok(false);
        };
        for address in addresses {
            let reference = ResourceRef::new(address).map_err(|error| error.to_string())?;
            let exists = if let Some(id) = reference.path().strip_prefix("proxy/") {
                self.resources.proxy.contains(id)
            } else {
                self.resources
                    .cache
                    .contains(&reference)
                    .await
                    .map_err(|error| error.to_string())?
            };
            if !exists {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
