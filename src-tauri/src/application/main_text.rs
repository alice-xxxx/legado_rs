//! 正文统一入口：请求后入库，缓存没有就新增，有就更新。
use super::*;

#[cfg(test)]
#[path = "main_text_tests.rs"]
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
pub fn cancel_main_text(book_id: String, chapter_id: String, service: tauri::State<'_, ApplicationService>) {
    service.cancel_main_text(&book_id, &chapter_id);
}

impl ApplicationService {
    pub(crate) fn cancel_main_text(&self, book_id: &str, chapter_id: &str) {
        let _ = self.text_cancel.send(Some((book_id.into(), chapter_id.into())));
    }

    pub(crate) async fn main_text(&self, book_id: &str, chapter_id: &str, cache: bool) -> Result<(), String> {
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
            let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
            let document = self
                .store
                .read_raw_document(&reference)
                .map_err(|error| error.to_string())?;
            let kind = self.store.book_type(book_id).map_err(|error| error.to_string())?;
            let book =
                crate::resources::content::book_view(&document, &kind).map_err(|error| error.to_string())?;
            let state = crate::resources::content::source_state(&document);
            let index = book["chapters"]
                .as_array()
                .and_then(|chapters| chapters.iter().position(|chapter| chapter["id"] == chapter_id))
                .ok_or("Chapter is no longer in the book directory")?;
            if book["chapters"][index]["volume"] == true {
                return Ok(());
            }
            let Some(source_id) = state["sourceId"].as_str() else {
                return if self.indexed_chapter_ready(&book, index).await? {
                    Ok(())
                } else {
                    Err("Local chapter resource is missing; re-import the original file".into())
                };
            };
            let source = self.find_source(source_id).await?;
            if !source.enabled {
                return Err("Book source is disabled".into());
            }
            let mut request = engine_request(
                if crate::rss::is_legacy_rss_source(&source.source) {
                    "rssContent"
                } else {
                    "content"
                },
                &source.source,
                None,
                None,
                Some(state["book"].clone()),
                Some(state["chapters"][index].clone()),
                state["chapters"]
                    .get(index + 1)
                    .and_then(|chapter| text_at(chapter, &["url", "chapterUrl"])),
            );
            request.cancellation = Some(token.clone());
            #[cfg(test)]
            let content = self.executor.execute(request.clone()).await?;
            #[cfg(all(not(test), any(target_os = "android", target_os = "ios")))]
            let content = {
                use tauri_plugin_source_engine::{SourceEngineCall, SourceEngineExt};
                let app = self.app.clone().ok_or("KMP plugin is unavailable")?;
                let data_dir = self.root.join("source-engine");
                let request = request.clone();
                tokio::task::spawn_blocking(move || {
                    let cancellation = request.cancellation.clone();
                    let mut wire = json!(request);
                    let _execution =
                        crate::source_host_ffi::ExecutionScope::enter(&mut wire, data_dir, cancellation)?;
                    let response = app
                        .source_engine()
                        .execute(SourceEngineCall {
                            request_json: wire.to_string(),
                        })
                        .map_err(|error| error.to_string())?;
                    serde_json::from_str::<Value>(&response.result_json).map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| error.to_string())??
            };
            #[cfg(all(not(test), not(any(target_os = "android", target_os = "ios"))))]
            let content = {
                #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
                let resource_dir = self.app.as_ref().and_then(|app| {
                    use tauri::Manager;
                    app.path().resource_dir().ok()
                });
                #[cfg(not(any(feature = "desktop", feature = "mobile-runtime")))]
                let resource_dir = None;
                crate::source_engine::execute_with_resource_dir(
                    request.clone(),
                    self.root.join("source-engine"),
                    resource_dir,
                )
                .await?
            };

            let mut chapter = self
                .chapter_content(
                    book_id,
                    chapter_id,
                    &request,
                    content,
                    book["chapters"][index].clone(),
                )
                .await?;
            if cache {
                chapter = self.cache_main_text_files(book_id, chapter_id, chapter).await?;
            }
            // 只提交这一章的缓存字段，不重写目录和进度。
            let database = self.store.database().clone();
            let (book_id, chapter_id, token) = (book_id.to_owned(), chapter_id.to_owned(), token.clone());
            tokio::task::spawn_blocking(move || database.save_main_text(book_id, chapter_id, chapter, token))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())
        };
        tokio::select! {
            biased;
            _ = cancelled => Err("Chapter content cancelled".into()),
            result = operation => result,
        }
    }

    /// 阅读预加载先检查缓存；显式正文请求则直接刷新。
    pub(super) async fn indexed_chapter_ready(&self, book: &Value, index: usize) -> Result<bool, String> {
        let chapter = &book["chapters"][index];
        let kind = book["contentType"].as_str().unwrap_or("text");
        let addresses = match kind {
            "image" => {
                let Some(pages) = chapter["pages"].as_array().filter(|pages| !pages.is_empty()) else {
                    return Ok(false);
                };
                pages
                    .iter()
                    .map(|page| page["src"].as_str())
                    .collect::<Option<Vec<_>>>()
            }
            "audio" | "video" => chapter["media"]["src"].as_str().map(|src| vec![src]),
            "pdf" => chapter["pdfPage"]["src"].as_str().map(|src| vec![src]),
            _ => chapter["resource"]["resourceId"].as_str().map(|src| vec![src]),
        };
        let Some(addresses) = addresses else {
            return Ok(false);
        };
        for address in addresses {
            let reference = ResourceRef::new(address).map_err(|error| error.to_string())?;
            let exists = if let Some(id) = reference.path().strip_prefix("media/") {
                self.server.contains_media(id).await
            } else {
                self.store
                    .regular_file_exists(&reference)
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
