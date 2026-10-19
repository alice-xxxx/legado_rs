//! 统一搜索入口：所有书源搜索都在此请求 KMP，并使用相同结果结构向 Vue emit。
use super::*;
use futures_util::{stream, StreamExt};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchPrecision {
    /// 书名或作者与整个关键词相等（忽略首尾空白和大小写）。
    Exact,
    #[default]
    /// 书名或作者包含关键词。
    Precise,
    /// 不按关键词过滤引擎结果。
    None,
}

#[derive(Default)]
pub(super) struct SearchRegistry {
    runs: HashMap<String, tokio_util::sync::CancellationToken>,
    pub(super) search_result_documents: HashMap<String, Value>,
}

pub(super) const SEARCH_CONCURRENCY: usize = if cfg!(any(target_os = "ios", target_os = "android"))
{
    4
} else {
    8
};
const SEARCH_SOURCE_TIMEOUT: Duration = Duration::from_secs(10);

impl ApplicationService {
    /// 唯一搜索入口：直接请求 KMP、按精度过滤，每个源完成后立即发送结果。
    /// 通知只携带本次增量；累计、合并、排序和翻页均由 Vue 处理。
    pub async fn search(
        &self,
        source_ids: &[String],
        keyword: &str,
        page: u32,
        precision: SearchPrecision,
        emit: Arc<dyn Fn(Value) + Send + Sync>,
    ) -> Result<String, String> {
        let operation = self.operation_read().await;
        let keyword = keyword.trim();
        if keyword.is_empty() || page == 0 || page > i32::MAX as u32 {
            return Err("Invalid search keyword or page".into());
        }
        let sources = self
            .read_sources()
            .await?
            .iter()
            .filter(|source| {
                source.enabled
                    && source_ids.contains(&source.id)
                    && !crate::source_metadata::is_rss_source_metadata(&source.source)
            })
            .cloned()
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return Err("No enabled book sources are selected".into());
        }
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let cancellation = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        crate::search_history::record_search(&self.store, &search_id, keyword, now_ms()).await?;
        self.searches
            .lock()
            .await
            .runs
            .insert(search_id.clone(), cancellation.clone());
        // 先交出会话编号供 Vue 取消；网络等待期间不持有恢复准入锁。
        drop(operation);
        emit(json!({"searchId": search_id, "results": []}));
        let normalized_keyword = keyword.to_lowercase();
        let rank_limit = match precision {
            SearchPrecision::Exact => 0,
            SearchPrecision::Precise => 1,
            SearchPrecision::None => 2,
        };
        let mut responses = stream::iter(sources)
            .map(|source| {
                let token = cancellation.child_token();
                let mut request = engine_request(
                    "search",
                    &source.source,
                    Some(keyword.to_owned()),
                    Some(page as i32),
                    None,
                    None,
                    None,
                );
                request.cancellation = Some(token.clone());
                async move {
                    let _cancel_on_drop = token.drop_guard();
                    let response = async {
                        let _slot = self
                            .search_slots
                            .acquire()
                            .await
                            .map_err(|error| error.to_string())?;
                        tokio::time::timeout(SEARCH_SOURCE_TIMEOUT, self.executor.execute(request))
                            .await
                            .unwrap_or_else(
                                |_| Err("Source search timed out after 10000 ms".into()),
                            )
                    }
                    .await;
                    (source, response)
                }
            })
            .buffer_unordered(SEARCH_CONCURRENCY);
        loop {
            let response = tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                response = responses.next() => response,
            };
            let Some((source, response)) = response else {
                break;
            };
            let _operation = tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                operation = self.operation_read() => operation,
            };
            let mut registry = self.searches.lock().await;
            if cancellation.is_cancelled() {
                break;
            }
            let mut update = json!({"searchId": search_id, "sourceId": source.id, "results": [], "hasMore": false});
            match response {
                Ok(value) if value["books"].is_array() => {
                    let books = value["books"].as_array().unwrap();
                    update["hasMore"] =
                        json!(value["hasNextPage"].as_bool().unwrap_or(!books.is_empty()));
                    for book in books {
                        if !book.is_object() {
                            update["error"] = json!("Source returned an invalid book entry");
                            continue;
                        }
                        let result_id = source_book_id(&source, book);
                        let leaf = project_search_result(&result_id, &source, book);
                        if search_match_rank(
                            &leaf.title,
                            leaf.author.as_deref(),
                            &normalized_keyword,
                        ) > rank_limit
                        {
                            continue;
                        }
                        let leaf = json!(leaf);
                        update["results"].as_array_mut().unwrap().push(leaf.clone());
                        // 暂存详情请求的引擎上下文；展示数据和可换来源由 Vue 管理。
                        registry.search_result_documents.insert(result_id, json!({
                            "searchId": search_id, "book": book
                        }));
                    }
                }
                response => {
                    update["error"] =
                        json!(response.err().unwrap_or_else(|| {
                            "Source engine returned an invalid search result".into()
                        }))
                }
            }
            drop(registry);
            emit(update);
        }
        self.searches.lock().await.runs.remove(&search_id);
        // 命令返回即代表结束，不重复发送完成通知或全部结果。
        Ok(search_id)
    }

    /// 直接取消在途引擎请求；保留已发布结果供 Vue 继续显示。
    pub async fn stop_search(&self, search_id: &str) -> Result<(), String> {
        if let Some(token) = self.searches.lock().await.runs.get(search_id) {
            token.cancel();
        }
        Ok(())
    }

    /// 关闭界面时释放会话及私有引擎结果；旧请求不能重新发布已释放的数据。
    pub async fn release_search_results(&self, search_id: &str) -> Result<(), String> {
        let mut registry = self.searches.lock().await;
        if let Some(token) = registry.runs.remove(search_id) {
            token.cancel();
        }
        registry
            .search_result_documents
            .retain(|_, cache| cache["searchId"].as_str() != Some(search_id));
        Ok(())
    }

    pub(super) async fn clear_searches(&self) {
        let mut registry = self.searches.lock().await;
        for token in registry.runs.values() {
            token.cancel();
        }
        *registry = SearchRegistry::default();
    }
}

impl ApplicationService {
    pub(in crate::application) async fn read_search_result_cache(
        &self,
        result_id: &str,
    ) -> Result<Value, String> {
        self.searches
            .lock()
            .await
            .search_result_documents
            .get(result_id)
            .cloned()
            .ok_or_else(|| "Search result is unavailable; search again".to_owned())
    }
}

/// Vue 等待命令结束；期间直接 emit 每个源的过滤结果。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn search(
    app: tauri::AppHandle,
    source_ids: Vec<String>,
    keyword: String,
    page: u32,
    precision: SearchPrecision,
    service: tauri::State<'_, ApplicationService>,
) -> Result<String, String> {
    use tauri::Emitter;
    let emitter = app.clone();
    let notify = Arc::new(move |update: Value| {
        let _ = emitter.emit("search-updated", update);
    });
    let result = service
        .search(&source_ids, &keyword, page, precision, notify)
        .await?;
    let _ = app.emit("history-updated", json!({"part":"search"}));
    Ok(result)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn stop_search(
    search_id: String,
    service: tauri::State<'_, ApplicationService>,
) -> Result<(), String> {
    service.stop_search(&search_id).await
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn release_search_results(
    search_id: String,
    service: tauri::State<'_, ApplicationService>,
) -> Result<(), String> {
    service.release_search_results(&search_id).await
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
