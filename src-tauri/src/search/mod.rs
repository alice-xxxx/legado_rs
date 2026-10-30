//! 统一搜索入口：所有书源搜索都在此请求 KMP，并使用相同结果结构向 Vue emit。

use crate::{
    sources::SourceRecord,
    support::{now_ms, text_at},
};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri_plugin_source_engine::SearchRequest;

pub(crate) struct Search {
    pub(crate) sources: Arc<crate::sources::Catalog>,
    pub(crate) executor: Arc<dyn tauri_plugin_source_engine::SourceExecutor>,
    pub(crate) searches: tokio::sync::Mutex<SearchRegistry>,
    search_slots: tokio::sync::Semaphore,
    pub(crate) backup: Arc<crate::backup::state::Operations>,
}
impl Search {
    pub(crate) fn new(
        sources: Arc<crate::sources::Catalog>,
        executor: Arc<dyn tauri_plugin_source_engine::SourceExecutor>,
        backup: Arc<crate::backup::state::Operations>,
    ) -> Self {
        Self {
            sources,
            executor,
            backup,
            searches: Default::default(),
            search_slots: tokio::sync::Semaphore::new(SEARCH_CONCURRENCY),
        }
    }
}

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
pub(crate) struct SearchRegistry {
    runs: HashMap<String, tokio_util::sync::CancellationToken>,
    pub(crate) search_result_documents: HashMap<String, Value>,
}

pub(crate) const SEARCH_CONCURRENCY: usize = if cfg!(any(target_os = "ios", target_os = "android"))
{
    4
} else {
    8
};

impl Search {
    /// 唯一搜索入口：直接请求 KMP、按精度过滤，每个源完成后立即发送结果。
    /// 通知只携带本次增量；累计、合并、排序和翻页均由 Vue 处理。
    pub(crate) async fn search(
        &self,
        database: &crate::database::Database,
        source_ids: &[String],
        keyword: &str,
        page: u32,
        precision: SearchPrecision,
        emit: Arc<dyn Fn(Value) + Send + Sync>,
    ) -> Result<String, String> {
        let operation = self.backup.operation_read().await;
        let keyword = keyword.trim();
        if keyword.is_empty() || page == 0 || page > i32::MAX as u32 {
            return Err("Invalid search keyword or page".into());
        }
        let sources = self
            .sources
            .all()
            .await?
            .iter()
            .filter(|source| {
                source.enabled
                    && source_ids.contains(&source.id)
                    && !crate::rss::is_rss_source(&source.source)
            })
            .cloned()
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return Err("No enabled book sources are selected".into());
        }
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let cancellation = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        database
            .record_search(keyword.to_owned(), now_ms())
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
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
                async move {
                    let _cancel_on_drop = token.clone().drop_guard();
                    let mut response_time_ms = None;
                    let response = async {
                        let request = SearchRequest {
                            source: (&source.source).try_into()?,
                            keyword: keyword.to_owned(),
                            page: page as i32,
                        };
                        let _slot = self
                            .search_slots
                            .acquire()
                            .await
                            .map_err(|error| error.to_string())?;
                        // 从取得执行名额后计时，排除本地并发排队时间。
                        let started = Instant::now();
                        let timeout =
                            Duration::from_secs(crate::source_http::network_timeout_seconds());
                        let response = tokio::time::timeout(
                            timeout,
                            self.executor.search(request, token.clone()),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            Err(format!(
                                "Source search timed out after {} ms",
                                timeout.as_millis()
                            ))
                        })
                        .and_then(|page| {
                            serde_json::to_value(page).map_err(|error| error.to_string())
                        });
                        response_time_ms = Some(started.elapsed().as_millis() as u64);
                        response
                    }
                    .await;
                    (source, response, response_time_ms)
                }
            })
            .buffer_unordered(SEARCH_CONCURRENCY);
        loop {
            let response = tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                response = responses.next() => response,
            };
            let Some((source, response, response_time_ms)) = response else {
                break;
            };
            let _operation = tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                operation = self.backup.operation_read() => operation,
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
                        let mut leaf = project_search_result(&result_id, &source, book);
                        leaf.response_time_ms = response_time_ms;
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
                        registry.search_result_documents.insert(
                            result_id,
                            json!({
                                "searchId": search_id, "book": book
                            }),
                        );
                    }
                }
                response => {
                    update["error"] = json!(response.err().unwrap_or_else(|| {
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
    pub(crate) async fn stop_search(&self, search_id: &str) -> Result<(), String> {
        if let Some(token) = self.searches.lock().await.runs.get(search_id) {
            token.cancel();
        }
        Ok(())
    }

    /// 关闭界面时释放会话及私有引擎结果；旧请求不能重新发布已释放的数据。
    pub(crate) async fn release_search_results(&self, search_id: &str) -> Result<(), String> {
        let mut registry = self.searches.lock().await;
        if let Some(token) = registry.runs.remove(search_id) {
            token.cancel();
        }
        registry
            .search_result_documents
            .retain(|_, cache| cache["searchId"].as_str() != Some(search_id));
        Ok(())
    }

    pub(crate) async fn clear_searches(&self) {
        let mut registry = self.searches.lock().await;
        for token in registry.runs.values() {
            token.cancel();
        }
        *registry = SearchRegistry::default();
    }
}

impl Search {
    pub(crate) async fn read_search_result_cache(&self, result_id: &str) -> Result<Value, String> {
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
pub(crate) async fn search(
    database: tauri::State<'_, crate::database::Database>,
    app: tauri::AppHandle,
    source_ids: Vec<String>,
    keyword: String,
    page: u32,
    precision: SearchPrecision,
    service: tauri::State<'_, Arc<Search>>,
) -> Result<String, String> {
    use tauri::Emitter;
    let emitter = app.clone();
    let notify = Arc::new(move |update: Value| {
        let _ = emitter.emit("search-updated", update);
    });
    let result = service
        .search(&database, &source_ids, &keyword, page, precision, notify)
        .await?;
    let _ = app.emit("history-updated", json!({"part":"search"}));
    Ok(result)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn stop_search(
    search_id: String,
    service: tauri::State<'_, Arc<Search>>,
) -> Result<(), String> {
    service.stop_search(&search_id).await
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn release_search_results(
    search_id: String,
    service: tauri::State<'_, Arc<Search>>,
) -> Result<(), String> {
    service.release_search_results(&search_id).await
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchResultSource {
    /// 保存可换来源时保留原始书籍上下文，避免详情规则丢失自定义字段。
    pub(crate) engine: Value,
    pub(crate) result_id: String,
    pub(crate) source_id: String,
    pub(crate) source_name: String,
    #[serde(default)]
    pub(crate) content_type: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) cover_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) latest_chapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) response_time_ms: Option<u64>,
    pub(crate) book_url: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) kind: Option<String>,
}

pub(crate) fn project_search_result(
    result_id: &str,
    source: &SourceRecord,
    book: &Value,
) -> SearchResultSource {
    let mut result = SearchResultSource {
        engine: book.clone(),
        result_id: result_id.to_owned(),
        source_id: source.id.clone(),
        source_name: source.name.clone(),
        content_type: crate::sources::content_type(&source.source).to_owned(),
        title: "Untitled".to_owned(),
        author: None,
        cover_src: None,
        intro: None,
        latest_chapter: None,
        response_time_ms: None,
        book_url: text_at(book, &["bookUrl", "url", "origin"]).unwrap_or_default(),
        media_type: None,
        kind: None,
    };
    // 音视频标记直接复用阅读类型，kind 仍是书籍题材。
    if matches!(result.content_type.as_str(), "audio" | "video") {
        result.media_type = Some(result.content_type.clone());
    } else if let Some(kind) = book
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| matches!(*kind, "image" | "manga" | "manhua" | "manhwa" | "漫画"))
    {
        result.kind = Some(kind.to_owned());
    }
    let read = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| book.get(*key).filter(|value| !value.is_null()).cloned())
            .and_then(|value| value.as_str().map(str::to_owned))
    };
    result.title = read(&["name", "title"])
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Untitled".to_owned());
    result.author = read(&["author"]);
    result.cover_src = read(&["coverUrl", "cover", "coverSrc"]);
    result.intro = read(&["intro", "introduction"]);
    result.latest_chapter = read(&["lastChapter", "latestChapter"]);
    result
}

// Keep punctuation significant for keyword matching (e.g. C++ and C#), while
// ignoring surrounding whitespace and letter case across source responses.
pub(crate) fn search_match_rank(title: &str, author: Option<&str>, keyword: &str) -> u8 {
    let title = title.trim().to_lowercase();
    let author = author.unwrap_or_default().trim().to_lowercase();
    if title == keyword || author == keyword {
        0
    } else if title.contains(keyword) || author.contains(keyword) {
        1
    } else {
        2
    }
}

/// 搜索结果即书籍定位，Vue 无需等待详情 API 返回 ID。
pub(crate) fn source_book_id(source: &SourceRecord, book: &Value) -> String {
    let url = text_at(book, &["bookUrl", "url", "origin"]).unwrap_or_else(|| book.to_string());
    let kind = crate::sources::content_type(&source.source);
    crate::database::online_book_id(&source.id, kind, &url)
}

impl Search {
    // 发现与 RSS 的结果仅在此投影和缓存，不在此发起搜索。
    pub(crate) async fn store_processed_results(
        &self,
        keyword: &str,
        page: u32,
        source_results: Vec<(SourceRecord, Value)>,
        errors: Vec<Value>,
    ) -> Result<Value, String> {
        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let mut public_results = Vec::with_capacity(source_results.len());
        let mut private_results = Vec::with_capacity(source_results.len());
        for (source, book) in source_results {
            let result_id = source_book_id(&source, &book);
            let leaf = project_search_result(&result_id, &source, &book);
            let cache = json!({
                "searchId": search_id,
                "sourceId": source.id,
                "book": book,
                "result": leaf
            });
            public_results.push(serde_json::to_value(leaf).map_err(|error| error.to_string())?);
            private_results.push((result_id, cache));
        }
        self.searches
            .lock()
            .await
            .search_result_documents
            .extend(private_results);
        let count = public_results.len();
        let document = json!({
            "searchId": search_id,
            "keyword": keyword,
            "page": page.max(1),
            "results": public_results,
            "errors": errors,
            "complete": true,
        });
        Ok(json!({ "search": document, "bookCount": count, "errors": errors }))
    }
}
