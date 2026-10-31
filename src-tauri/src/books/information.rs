//! 唯一详情接口：按源和书籍 URL 请求，取得详情后统一刷新或新建存储。
use std::sync::Arc;

use crate::search::source_book_id;
use tauri_plugin_source_engine::BookRequest;
use serde_json::json;

impl Information {
    pub(crate) fn cancel_information(&self, source_id: &str, book_url: &str) {
        let _ = self
            .information_cancel
            .send(Some((source_id.into(), book_url.into())));
    }

    pub(crate) async fn more_information(
        &self,
        database: &crate::database::Database,
        source_id: &str,
        book_url: &str,
    ) -> Result<(), String> {
        let token = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = token.clone().drop_guard();
        let mut cancellations = self.information_cancel.subscribe();
        // 每个请求独立监听取消；按源和 URL 匹配，恢复备份时取消全部。
        let cancelled = async {
            loop {
                match cancellations.recv().await {
                    Ok(None) => break,
                    Ok(Some((source, url))) if source == source_id && url == book_url => break,
                    _ => {}
                }
            }
        };
        let operation = async {
            if book_url.trim().is_empty() {
                return Err("Book URL is missing".into());
            }
            let source = self.sources.get(source_id).await?;
            if !source.enabled {
                return Err("Book source is disabled".into());
            }
            let kind = crate::sources::content_type(&source.source);
            let key = source_book_id(&source, &json!({"bookUrl":book_url}));
            // 搜索已提供的书名等参数可供规则使用，没有时只需书籍 URL。
            let cached = self
                .search
                .read_search_result_cache(&key)
                .await
                .ok()
                .map(|cache| cache["book"].clone());
            let mut raw_book = if let Some(book) = cached {
                book
            } else {
                let database = database.clone();
                let id = key.clone();
                tokio::task::spawn_blocking(move || database.book_engine(&id))
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .unwrap_or_else(|| json!({}))
            };
            raw_book["bookUrl"] = json!(book_url);
            raw_book["origin"] = source.source["bookSourceUrl"].clone();
            raw_book["originName"] = json!(source.name);
            let info = self.executor.detail(
                BookRequest::new((&source.source).try_into()?, serde_json::from_value(raw_book.clone()).map_err(|e| e.to_string())?), token.clone(),
            ).await?;
            let info = serde_json::to_value(info).map_err(|error| error.to_string())?;
            // 详情规则可以只返回部分字段，目录请求仍需保留搜索和已存储的上下文。
            let mut engine_book = raw_book;
            engine_book
                .as_object_mut()
                .ok_or("Invalid book context")?
                .extend(
                    info.as_object()
                        .ok_or("Invalid book details")?
                        .iter()
                        .filter(|(_, value)| !value.is_null())
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            engine_book["bookUrl"] = json!(book_url);
            if token.is_cancelled() {
                return Err("Book details cancelled".into());
            }
            let book_id = key;
            let chapters = self.executor.chapters(
                BookRequest::new((&source.source).try_into()?, serde_json::from_value(engine_book.clone()).map_err(|e| e.to_string())?), token.clone(),
            ).await?.chapters;
            {
                let database = database.clone();
                let token = token.clone();
                database
                    .save_information(
                        {
                            let metadata =
                                crate::book_metadata::project_book_metadata(&engine_book, &source);
                            crate::database::Book {
                                id: book_id,
                                content_type: kind.into(),
                                source_id: Some(source.id),
                                title: metadata.title,
                                author: metadata.author,
                                intro: metadata.intro,
                                cover: metadata.cover_src.map(|reference| reference.to_string()),
                                word_count: metadata.word_count,
                                latest_chapter_title: metadata.latest_chapter,
                                engine: Some(
                                    serde_json::from_value(engine_book)
                                        .map_err(|error| error.to_string())?,
                                ),
                                ..Default::default()
                            }
                        },
                        chapters,
                        token,
                    )
                    .map_err(|e| e.to_string())?
                    .await
                    .map_err(|e| e.to_string())
            }?;
            Ok(())
        };
        tokio::select! {
            biased;
            _ = cancelled => Err("Book details cancelled".into()),
            result = operation => result,
        }
    }
}

/// 不返回详情数据，Vue 等待入库后自行读取业务表。
pub(crate) struct Information {
    pub(crate) sources: Arc<crate::sources::Catalog>,
    pub(crate) executor: Arc<dyn tauri_plugin_source_engine::SourceExecutor>,
    information_cancel: tokio::sync::broadcast::Sender<Option<(String, String)>>,
    search: Arc<crate::search::Search>,
}
impl Information {
    pub(crate) fn new(
        sources: Arc<crate::sources::Catalog>,
        executor: Arc<dyn tauri_plugin_source_engine::SourceExecutor>,
        search: Arc<crate::search::Search>,
    ) -> Self {
        Self {
            sources,
            executor,
            search,
            information_cancel: tokio::sync::broadcast::channel(64).0,
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn more_information(
    database: tauri::State<'_, crate::database::Database>,
    source_id: String,
    book_url: String,
    service: tauri::State<'_, Arc<Information>>,
) -> Result<(), String> {
    service
        .more_information(&database, &source_id, &book_url)
        .await
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) fn cancel_more_information(
    source_id: String,
    book_url: String,
    service: tauri::State<'_, Arc<Information>>,
) {
    service.cancel_information(&source_id, &book_url);
}

impl Information {
    pub(crate) fn cancel_all(&self) {
        let _ = self.information_cancel.send(None);
    }
}
