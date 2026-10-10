//! 后台任务查询、清理、重试和书目刷新调度。
use super::*;

impl ApplicationService {
    pub async fn list_tasks(&self) -> Result<Value, String> {
        Ok(json!({ "resource": self.task_resource_descriptor()? }))
    }

    pub async fn tasks_resource(&self) -> Result<Value, String> {
        self.list_tasks().await
    }

    pub async fn clear_finished_tasks(&self) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let (removed_count, in_use_count) = {
            let mut registry = self.tasks.lock().await;
            let in_use_ids = registry.signals.keys().cloned().collect::<HashSet<_>>();
            let mut removed_count = 0;
            let mut in_use_count = 0;
            let retained_records = registry
                .records
                .iter()
                .filter(|task| {
                    if !matches!(task.status.as_str(), "completed" | "failed" | "cancelled") {
                        return true;
                    }
                    if in_use_ids.contains(&task.id) {
                        in_use_count += 1;
                        true
                    } else {
                        removed_count += 1;
                        false
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            if removed_count > 0 {
                self.persist_task_records_locked(&retained_records).await?;
                registry.records = retained_records;
            }
            (removed_count, in_use_count)
        };
        Ok(json!({
            "removedCount": removed_count,
            "inUseCount": in_use_count,
            "resource": self.task_resource_descriptor()?,
        }))
    }

    /// Retry a failed or cancelled persisted task by creating a new task from
    /// its stored request fields. The old record remains terminal and intact
    /// for task history; interrupted/running tasks continue to use resume.
    pub async fn retry_task(&self, task_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(task_id, "taskId")?;
        let task = {
            let registry = self.tasks.lock().await;
            registry
                .records
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?
        };
        if !matches!(task.status.as_str(), "failed" | "cancelled") {
            return Err(format!(
                "Only failed or cancelled tasks can be retried; task is '{}'",
                task.status
            ));
        }
        // Keep the selected historical request and the newly queued task in
        // the same restore session. Reacquiring a read guard in the public
        // start_* methods can deadlock behind a pending exclusive restore.
        match task.kind.as_str() {
            "chapterDownload" => {
                let book_id = task
                    .book_id
                    .as_deref()
                    .filter(|book_id| !book_id.is_empty())
                    .ok_or_else(|| {
                        "Task retry is unsupported because the download request has no bookId"
                            .to_owned()
                    })?;
                if task.total == 0 || task.completed > task.total {
                    return Err(
                        "Task retry is unsupported because the download progress is invalid".into(),
                    );
                }
                let chapter_ids = task
                    .chapter_ids
                    .as_ref()
                    .filter(|ids| {
                        task.total > 0
                            && ids.len() == task.total
                            && ids.iter().all(|id| !id.is_empty())
                            && ids.iter().collect::<HashSet<_>>().len() == ids.len()
                    })
                    .ok_or_else(|| "该旧下载任务缺少有效章节快照，请取消后重新下载。".to_owned())?;
                let remaining = task.total - task.completed;
                if remaining == 0 {
                    let warning = "此下载任务的所有快照章节均已处理完成，无需再次下载。".to_owned();
                    let task_warning = warning.clone();
                    self.update_task(&task.id, move |current| {
                        current.status = "completed".into();
                        current.error = None;
                        current.result = Some(json!({ "warning": task_warning }));
                    })
                    .await?;
                    return Err(warning);
                }

                let (remaining_ids, current_from_index, skipped_count) = {
                    let _book_lock = self.book_lock(book_id).await;
                    let book_ref = self
                        .store
                        .book_ref(book_id)
                        .map_err(|error| error.to_string())?;
                    let book = self
                        .store
                        .read_json_ref(&book_ref)
                        .await
                        .map_err(|_| "对应书籍已从书架移除，无法继续旧下载任务。".to_owned())?;
                    let chapters = book
                        .get("chapters")
                        .and_then(Value::as_array)
                        .ok_or_else(|| "书籍目录缺失，无法继续旧下载任务。".to_owned())?;
                    let current_ids = chapters
                        .iter()
                        .filter_map(|chapter| chapter.get("id").and_then(Value::as_str))
                        .collect::<HashSet<_>>();
                    let remaining_ids = chapter_ids[task.completed..]
                        .iter()
                        .filter(|chapter_id| current_ids.contains(chapter_id.as_str()))
                        .cloned()
                        .collect::<Vec<_>>();
                    let skipped_count = remaining - remaining_ids.len();
                    let current_from_index = remaining_ids
                        .first()
                        .and_then(|first_id| {
                            chapters.iter().position(|chapter| {
                                chapter.get("id").and_then(Value::as_str) == Some(first_id.as_str())
                            })
                        })
                        .unwrap_or(task.from_index.saturating_add(task.completed));
                    (remaining_ids, current_from_index, skipped_count)
                };

                if remaining_ids.is_empty() {
                    let warning = format!(
                        "目录已变化：剩余 {} 个快照章节均已移除；此前已处理 {} 个章节。没有可继续下载的旧章节，请从当前目录启动新下载；不会按序号映射到新章节。",
                        skipped_count, task.completed,
                    );
                    let task_warning = warning.clone();
                    self.update_task(&task.id, move |current| {
                        current.status = "completed".into();
                        current.error = None;
                        current.result = Some(json!({ "warning": task_warning }));
                    })
                    .await?;
                    return Err(warning);
                }

                let warning = (skipped_count > 0).then(|| format!(
                    "目录已变化：跳过 {skipped_count} 个已移除的快照章节，继续 {remaining_ids_len} 个仍存在的快照章节。此前已处理 {} 个章节；不会按序号映射到新章节。",
                    task.completed,
                    remaining_ids_len = remaining_ids.len(),
                ));
                self.start_chapter_download_ids_unlocked(book_id, current_from_index, remaining_ids, warning)
                    .await
            }
            "refreshChapters" | "checkNewChapters" => {
                let check_only = task.kind == "checkNewChapters";
                if task.check_only != check_only {
                    return Err(
                        "Task retry is unsupported because its catalog request is inconsistent"
                            .into(),
                    );
                }
                let book_id = task
                    .book_id
                    .as_deref()
                    .filter(|book_id| !book_id.is_empty())
                    .ok_or_else(|| {
                        "Task retry is unsupported because the catalog request has no bookId"
                            .to_owned()
                    })?;
                self.start_catalog_refresh_unlocked(book_id, check_only).await
            }
            "search" => {
                let source_ids = task.source_ids.as_deref().ok_or_else(|| {
                    "Task retry is unsupported because the search request has no sourceIds"
                        .to_owned()
                })?;
                let keyword = task.keyword.as_deref().ok_or_else(|| {
                    "Task retry is unsupported because the search request has no keyword".to_owned()
                })?;
                if task.page == 0 {
                    return Err(
                        "Task retry is unsupported because the search request has no page".into(),
                    );
                }
                self.start_search_unlocked(source_ids, keyword, task.page).await
            }
            "bookSourceCandidates" => {
                let book_id = task
                    .book_id
                    .as_deref()
                    .filter(|book_id| !book_id.is_empty())
                    .ok_or_else(|| {
                        "Task retry is unsupported because the source-candidate request has no bookId".to_owned()
                    })?;
                let source_ids = task.source_ids.as_deref().ok_or_else(|| {
                    "Task retry is unsupported because the source-candidate request has no sourceIds".to_owned()
                })?;
                let keyword = task.keyword.as_deref().ok_or_else(|| {
                    "Task retry is unsupported because the source-candidate request has no keyword".to_owned()
                })?;
                if task.page == 0 {
                    return Err(
                        "Task retry is unsupported because the source-candidate request has no page".into(),
                    );
                }
                self.search_book_source_candidates_unlocked(book_id, source_ids, Some(keyword), task.page)
                    .await
            }
            kind => Err(format!("Task retry is unsupported for kind '{kind}'")),
        }
    }

    pub async fn refresh_chapters(&self, book_id: &str) -> Result<Value, String> {
        self.start_catalog_refresh(book_id, false).await
    }

    pub async fn check_new_chapters(&self, book_id: &str) -> Result<Value, String> {
        self.start_catalog_refresh(book_id, true).await
    }
}
