//! 唯一详情接口：按源和书籍 URL 请求，取得详情后统一刷新或新建存储。

use super::*;

impl ApplicationService {
    pub(crate) fn cancel_information(&self, source_id: &str, book_url: &str) {
        let _ = self
            .information_cancel
            .send(Some((source_id.into(), book_url.into())));
    }

    pub(crate) async fn more_information(
        &self,
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
            let source = self.find_source(source_id).await?;
            if !source.enabled {
                return Err("Book source is disabled".into());
            }
            let kind = crate::source_metadata::content_type(&source.source);
            let key = source_book_id(&source, &json!({"bookUrl":book_url}));
            // 搜索已提供的书名等参数可供规则使用，没有时只需书籍 URL。
            let cached = self
                .read_search_result_cache(&key)
                .await
                .ok()
                .map(|cache| cache["book"].clone());
            let mut raw_book = if let Some(book) = cached {
                book
            } else {
                let database = self.store.database().clone();
                let source_id = source_id.to_owned();
                let book_url = book_url.to_owned();
                tokio::task::spawn_blocking(move || {
                    database.information_seed(&source_id, &book_url, kind)
                })
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?
                .unwrap_or_else(|| json!({}))
            };
            raw_book["bookUrl"] = json!(book_url);
            raw_book["origin"] = source.source["bookSourceUrl"].clone();
            raw_book["originName"] = json!(source.name);
            let legacy_rss = crate::rss::is_legacy_rss_source(&source.source);
            let execute = |mut request: SourceEngineRequest| {
                request.cancellation = Some(token.clone());
                self.executor.execute(request)
            };
            let info = execute(engine_request(
                if legacy_rss {
                    "rssBookInfo"
                } else {
                    "bookInfo"
                },
                &source.source,
                None,
                None,
                Some(raw_book.clone()),
                None,
                None,
            ))
            .await?;
            if !info.is_object() {
                return Err("Source engine returned invalid book details".into());
            }
            // 详情规则可以只返回部分字段，目录请求仍需保留搜索和已存储的上下文。
            let mut engine_book = raw_book;
            engine_book
                .as_object_mut()
                .ok_or("Invalid book context")?
                .extend(
                    info.as_object()
                        .unwrap()
                        .iter()
                        .filter(|(_, value)| !value.is_null())
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            engine_book["bookUrl"] = json!(book_url);
            // 先取得详情，再按同一来源和 URL 查库；不由调用方指定刷新或新建。
            let database = self.store.database().clone();
            let lookup_source = source_id.to_owned();
            let lookup_url = book_url.to_owned();
            let existing = tokio::task::spawn_blocking(move || {
                database.information_id(&lookup_source, &lookup_url, kind)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
            if token.is_cancelled() {
                return Err("Book details cancelled".into());
            }
            let book_id = existing.unwrap_or(key);
            self.store
                .bind_book_type(&book_id, kind)
                .await
                .map_err(|error| error.to_string())?;
            let chapters = execute(engine_request(
                if legacy_rss {
                    "rssChapters"
                } else {
                    "chapters"
                },
                &source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?;
            let chapters = chapters
                .as_array()
                .ok_or("Source engine returned an invalid chapter catalog")?;
            self.save_information(
                &book_id,
                &source,
                engine_book,
                chapters.to_vec(),
                token.clone(),
            )
            .await?;
            Ok(())
        };
        tokio::select! {
            biased;
            _ = cancelled => Err("Book details cancelled".into()),
            result = operation => result,
        }
    }
}

impl ApplicationService {
    // 新打开和刷新走同一入库逻辑；在事务内读取最新缓存，再合并详情和目录。
    async fn save_information(
        &self,
        book_id: &str,
        source: &SourceRecord,
        response: Value,
        chapters: Vec<Value>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        let database = self.store.database().clone();
        let reference = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let root = self.store.root().to_path_buf();
        let book_id = book_id.to_owned();
        let source = source.clone();
        tokio::task::spawn_blocking(move || {
            database.transaction(move |db| {
                use crate::resources::{content, ResourceError};
                if cancellation.is_cancelled() { return Err(ResourceError::new("Book details cancelled")); }
                let current = crate::database::read(db, reference.path())?.unwrap_or_else(||json!({}));
                let saved = current.get("id").is_some();
                let mut state = content::source_state(&current);
                if saved && (state["sourceId"] != source.id || text_at(&state["book"], &["bookUrl","url"]) != text_at(&response, &["bookUrl","url"])) {
                    return Err(ResourceError::new("Book changed during refresh"));
                }
                if !saved {
                    state["sourceId"] = json!(source.id);
                    state["bookInstanceId"] = json!(uuid::Uuid::new_v4().simple().to_string());
                    state["catalogGeneration"] = json!(uuid::Uuid::new_v4().simple().to_string());
                }
                let engine_book = state["book"].clone();
                let response_fields = response
                    .as_object()
                    .ok_or_else(|| ResourceError::new("Source engine returned invalid book details"))?;
                let mut refreshed_fields = engine_book
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                // 合并部分字段时保留规则上下文；刷新不能改动已有书籍的来源身份。
                const CATALOG_IDENTITY_KEYS: [&str; 5] = ["bookUrl", "url", "origin", "articleId", "id"];
                for (key, value) in response_fields {
                    if (saved && CATALOG_IDENTITY_KEYS.contains(&key.as_str())) || value.is_null() {
                        continue;
                    }
                    let valid = match key.as_str() {
                        "name" | "title" | "author" | "coverUrl" | "cover" | "coverSrc" | "intro"
                        | "introduction" | "lastChapter" | "latestChapter" | "latestChapterTitle" => {
                            value.as_str().is_some_and(|text| !text.trim().is_empty())
                        }
                        "kind" | "category" => {
                            value.as_str().is_some_and(|text| !text.trim().is_empty())
                                || value.as_array().is_some_and(|values| {
                                    values.iter().any(|value| {
                                        value.as_str().is_some_and(|text| !text.trim().is_empty())
                                    })
                                })
                        }
                        "wordCount" | "word_count" => {
                            value.as_number().is_some()
                                || value.as_str().is_some_and(|text| !text.trim().is_empty())
                        }
                        _ => true,
                    };
                    if !valid {
                        continue;
                    }
                    refreshed_fields.insert(key.clone(), value.clone());
                }
                let refreshed_engine_book = Value::Object(refreshed_fields);
                let metadata = crate::book_metadata::project_book_metadata(&refreshed_engine_book, &source);
                let kind = crate::source_metadata::content_type(&source.source);
                let mut view = if saved { content::book_view(&current, kind)? } else {
                    json!({"id":book_id,"title":"Untitled","author":"","chapters":[],"progress":ProgressSummary::default()})
                };
                crate::book_display_metadata::update_base(&mut view, &metadata).map_err(ResourceError::new)?;
                let labels = json!(metadata);
                for key in ["sourceId", "sourceName", "sourceGroup", "wordCount", "contentType"] {
                    if let Some(value) = labels.get(key) { view[key] = value.clone(); }
                }
                if let Some(tag) = metadata.kind.as_ref() { view["tags"] = json!([tag]); }
                let old_raw = state["chapters"]
                    .as_array()
                    .ok_or_else(|| ResourceError::new("Saved catalog is missing"))?;
                let old_chapters: Vec<ChapterDescriptor> =
                    serde_json::from_value(view["chapters"].clone())
                        .map_err(|error| ResourceError::new(error.to_string()))?;
                let progress: ProgressSummary = serde_json::from_value(view["progress"].clone())
                    .map_err(|error| ResourceError::new(error.to_string()))?;
                let cached = old_chapters
                    .iter()
                    .filter(|chapter| {
                        chapter.resource.as_ref().is_some_and(|resource| {
                            resource.resource_id.is_local()
                                && root.join(resource.resource_id.path()).is_file()
                        })
                    })
                    .map(|chapter| chapter.id.clone())
                    .collect();
                let mut plan = crate::catalog::reconcile_catalog(
                    &book_id,
                    old_raw,
                    &old_chapters,
                    &chapters,
                    &progress,
                    &cached,
                    now_ms(),
                )
                .map_err(ResourceError::new)?;
                // 保留旧章节身份；新章节沿用本次目录所属的代次，避免误用换源前的缓存。
                if let Some(generation) = state["catalogGeneration"].as_str().filter(|_|saved) {
                    for (index, chapter) in plan.chapters.iter_mut().enumerate() {
                        if !old_chapters.iter().any(|old| old.id == chapter.id) {
                            let url = text_at(&chapters[index], &["url", "chapterUrl"])
                                .unwrap_or_else(|| format!("@{index}"));
                            chapter.id = format!("chapter-{:016x}", stable_hash(&format!("{book_id}\0{generation}\0{url}")));
                        }
                    }
                    if let Some(chapter) = plan.chapters.get(plan.progress.chapter_index) {
                        plan.progress.chapter_id = Some(chapter.id.clone());
                    }
                }
                let mut next_chapters = json!(plan.chapters);
                for (index, chapter) in next_chapters
                    .as_array_mut()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let Some(old) = view["chapters"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|old| old["id"] == chapter["id"])
                    else {
                        continue;
                    };
                    let old_index = old["index"].as_u64().unwrap_or(0) as usize;
                    if text_at(&old_raw[old_index], &["url", "chapterUrl"])
                        != text_at(&chapters[index], &["url", "chapterUrl"])
                    {
                        continue;
                    }
                    // 图片页、媒体地址和下载文件只在章节 URL 未变化时复用。
                    for key in ["pages", "sources", "protocol", "files", "url", "media"] {
                        if let Some(value) = old.get(key) {
                            chapter[key] = value.clone();
                        }
                    }
                    if old["media"].is_object() {
                        chapter["resource"] = old["resource"].clone();
                    }
                }
                view["chapters"] = next_chapters;
                view["chapterCount"] = json!(plan.chapters.len());
                view["latestChapter"] = json!(plan.latest_chapter);
                // 首次打开不产生阅读记录，刷新才沿用并定位已有阅读进度。
                view["progress"] = if saved { json!(plan.progress) } else { json!(ProgressSummary::default()) };
                if old_raw != &chapters {
                    view["updatedAtMs"] = json!(now_ms());
                }
                state["chapters"] = json!(chapters);
                state["book"] = refreshed_engine_book;
                let encoded = content::encode_book(&view, Some(&state), &current, kind)?;
                if cancellation.is_cancelled() { return Err(ResourceError::new("Book details cancelled")); }
                crate::database::write(db, reference.path(), &encoded)
            })
        }).await.map_err(|error|error.to_string())?.map_err(|error|error.to_string())
    }
}

/// 不返回详情数据，Vue 等待入库后自行读取业务表。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn more_information(
    source_id: String,
    book_url: String,
    service: tauri::State<'_, ApplicationService>,
) -> Result<(), String> {
    service.more_information(&source_id, &book_url).await
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub fn cancel_more_information(
    source_id: String,
    book_url: String,
    service: tauri::State<'_, ApplicationService>,
) {
    service.cancel_information(&source_id, &book_url);
}

#[cfg(test)]
#[path = "information_tests.rs"]
mod tests;
