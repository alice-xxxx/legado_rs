//! 后台搜索、目录刷新与章节下载任务的执行过程。

use super::*;

const SEARCH_SOURCE_BATCH_SIZE: usize = 9;

/// Resolve worker termination against a later persisted control request.
/// Cancellation wins even if the worker finished its last source/chapter;
/// pausing a nearly completed task preserves its resumable final checkpoint.
/// Once a task is terminal, an older worker must not rewrite its outcome.
fn reconcile_task_completion<'a>(current: &str, requested: &'a str) -> Option<&'a str> {
    if matches!(current, "completed" | "failed" | "cancelled" | "recoveryRequired") {
        None
    } else if current == "cancelling" {
        Some("cancelled")
    } else if matches!(current, "pausing" | "paused") && requested == "completed" {
        Some("paused")
    } else {
        Some(requested)
    }
}

/// Live search memory can be ahead of the task counter when saving the task fails.
/// Align that same round before reporting failure; process restart discards it.
fn reconciled_source_progress(
    task_completed: usize, task_total: usize,
    document_completed: u64, document_total: u64,
) -> Result<Option<usize>, String> {
    if document_total != task_total as u64
        || document_completed > document_total
        || document_completed < task_completed as u64
    {
        return Err("Search document progress disagrees with the persisted task".into());
    }
    Ok((document_completed > task_completed as u64).then_some(document_completed as usize))
}

impl ApplicationService {
    /// Supervise the Tokio worker separately so a panic cannot bypass its
    /// cleanup and permanently strand a queued/running task behind a live
    /// control sender. All worker entry points use this same launch path.
    pub(in crate::application) fn spawn_task_worker(
        &self,
        task_id: String,
        signal: tokio::sync::watch::Receiver<TaskSignal>,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            let lease = signal.clone();
            let worker_service = service.clone();
            let worker_id = task_id.clone();
            let join = tokio::spawn(async move {
                worker_service.run_task(worker_id, signal).await;
            });
            if let Err(error) = join.await {
                let detail = format!("Worker Tokio task exited unexpectedly: {error}");
                eprintln!("Task {task_id}: {detail}");
                service
                    .notify_task_execution_recovery_required(&task_id, &lease, &detail)
                    .await;
                service.release_task_worker(&task_id, &lease).await;
            }
        });
    }

    pub(super) async fn run_task(
        &self,
        task_id: String,
        mut signal: tokio::sync::watch::Receiver<TaskSignal>,
    ) {
        let outcome = async {
            let operation = self.operation_gate.clone().read_owned().await;
            if !self.task_worker_is_current(&task_id, &signal).await {
                return Ok(());
            }
            let task = self.task_summary_record(&task_id).await?;
            drop(operation);
            // Every task runner begins with task_checkpoint: it owns the
            // durable queued -> running/paused/cancelled transition. Avoid an
            // unconditional running publication that can erase a stop request
            // issued before Tokio first schedules this worker.
            let result = match task.kind.as_str() {
                "chapterDownload" => self.run_download_task(&task_id, &mut signal).await,
                "search" | "bookSourceCandidates" => {
                    self.run_search_task(&task_id, &mut signal).await
                }
                "refreshChapters" | "checkNewChapters" => {
                    self.run_refresh_task(&task_id, &mut signal).await
                }
                _ => Err(format!("Unsupported task kind '{}'", task.kind)),
            };
            match result {
                Ok(Some(_)) => {
                    let _operation = self.operation_gate.clone().read_owned().await;
                    if !self.task_worker_is_current(&task_id, &signal).await {
                        return Ok(());
                    }
                    self.finish_task(&task_id, "completed", None).await
                }
                Ok(None) => Ok(()),
                Err(error) => {
                    if self
                        .task_summary_record(&task_id)
                        .await
                        .is_ok_and(|task| task.status == "recoveryRequired")
                    {
                        return Ok(());
                    }
                    if matches!(task.kind.as_str(), "search" | "bookSourceCandidates") {
                        let round_is_current = {
                            let registry = self.tasks.lock().await;
                            task.search_id.as_deref().is_some_and(|search_id| {
                                registry.search_documents.get(search_id).is_some_and(|document| {
                                    document.get("taskId").and_then(Value::as_str)
                                        == Some(task_id.as_str())
                                }) && (task.kind != "bookSourceCandidates"
                                    || registry.search_replacement_contexts.contains_key(search_id))
                                    && (task.kind != "search"
                                        || registry.active_search_task_id.as_deref()
                                            == Some(task_id.as_str()))
                            })
                        };
                        if signal.borrow().cancelled || !round_is_current {
                            self.finish_task(&task_id, "cancelled", None).await?;
                            return Ok(());
                        }
                    }
                    // The search document can be one source ahead of tasks.json
                    // if the worker failed between their separate commits.
                    if matches!(task.kind.as_str(), "search" | "bookSourceCandidates") {
                        self.reconcile_search_document_progress(&task_id, &signal).await?;
                    }
                    let _operation = self.operation_gate.clone().read_owned().await;
                    if !self.task_worker_is_current(&task_id, &signal).await {
                        return Ok(());
                    }
                    self.finish_task(&task_id, "failed", Some(error)).await
                }
            }
        }
        .await;
        if let Err(error) = outcome {
            eprintln!("Task {task_id} stopped with an error: {error}");
            // The worker has no durable terminal result. Surface an explicit
            // in-memory recovery lock instead of leaving a ghost "running"
            // task with no worker and no way to resume.
            self.notify_task_execution_recovery_required(&task_id, &signal, &error).await;
        }
        self.release_task_worker(&task_id, &signal).await;
    }

    async fn release_task_worker(
        &self,
        task_id: &str,
        signal: &tokio::sync::watch::Receiver<TaskSignal>,
    ) {
        let mut registry = self.tasks.lock().await;
        if registry.owns_worker(task_id, signal) {
            registry.signals.remove(task_id);
            if registry.active_search_task_id.as_deref() == Some(task_id) {
                registry.active_search_task_id = None;
            }
        }
    }

    /// Task IDs may reappear after importing a backup. The control channel
    /// is the worker's actual lease; an old worker must not update a restored
    /// record merely because its opaque ID matches.
    async fn task_worker_is_current(
        &self,
        task_id: &str,
        signal: &tokio::sync::watch::Receiver<TaskSignal>,
    ) -> bool {
        let registry = self.tasks.lock().await;
        registry.owns_worker(task_id, signal)
    }

    async fn task_summary_record(&self, task_id: &str) -> Result<AppTask, String> {
        let registry = self.tasks.lock().await;
        registry
            .records
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| format!("Unknown task '{task_id}'"))
    }

    async fn finish_task(
        &self,
        task_id: &str,
        status: &str,
        error: Option<String>,
    ) -> Result<(), String> {
        // Keep the candidate snapshot and terminal task result in one critical
        // section. A task-resource reader must never see a terminal candidate
        // without the immutable resource descriptor that owns its results.
        let task = {
            let mut registry = self.tasks.lock().await;
            let position = registry.records.iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            let is_candidate = registry.records[position].kind == "bookSourceCandidates";
            let candidate_state_active = !is_candidate
                || registry.records[position].search_id.as_deref().is_some_and(|search_id| {
                    registry.search_replacement_contexts.contains_key(search_id)
                        && registry.search_documents.get(search_id).is_some_and(|document| {
                            document.get("taskId").and_then(Value::as_str)
                                == Some(registry.records[position].id.as_str())
                        })
                });
            let requested_status = if !candidate_state_active {
                "cancelled"
            } else {
                status
            };
            let Some(final_status) = reconcile_task_completion(
                &registry.records[position].status,
                requested_status,
            ) else {
                // Terminal and recovery-required states are immutable. Avoid
                // writing a no-op task JSON while a crash journal may still
                // prohibit writes.
                return Ok(());
            };
            let previous = registry.records[position].clone();
            let previous_completed = previous.completed;
            let previous_total = previous.total;
            let search_id = registry.records[position].search_id.clone();
            let mut previous_search_document = None;
            let mut candidate_result = None;
            if is_candidate
                && candidate_state_active
                && matches!(final_status, "completed" | "failed" | "cancelled")
            {
                let search_id = search_id.as_deref()
                    .ok_or_else(|| "Candidate search has no search resource".to_owned())?;
                let document = registry.search_documents.get_mut(search_id)
                    .ok_or_else(|| "Replacement search is no longer active; search again".to_owned())?;
                previous_search_document = Some(document.clone());
                document["complete"] = json!(true);
                document["status"] = json!(final_status);
                document["completedSources"] = json!(previous_completed);
                document["totalSources"] = json!(previous_total);
                if final_status == "cancelled" {
                    document["cancelled"] = json!(true);
                } else if let Some(object) = document.as_object_mut() {
                    object.remove("cancelled");
                }
                let final_error = if final_status == status { error.as_deref() } else { None };
                if let Some(error) = final_error {
                    document["error"] = json!(error);
                } else if let Some(object) = document.as_object_mut() {
                    object.remove("error");
                }
                let snapshot = document.clone();
                match self.write_search_snapshot(&snapshot).await {
                    Ok((_, descriptor)) => candidate_result = Some(json!({ "resource": descriptor })),
                    Err(error) => {
                        registry.search_documents.insert(
                            search_id.to_owned(),
                            previous_search_document.take().unwrap_or_else(|| snapshot.clone()),
                        );
                        return Err(error);
                    }
                }
            }
            registry.records[position].status = final_status.to_owned();
            registry.records[position].error =
                if final_status == status { error } else { None };
            if let Some(result) = candidate_result {
                registry.records[position].result = Some(result);
            }
            registry.records[position].updated_at_ms = now_ms();
            if let Err(error) = self.persist_tasks_locked(&registry).await {
                registry.records[position] = previous;
                if let (Some(search_id), Some(document)) = (search_id, previous_search_document) {
                    registry.search_documents.insert(search_id, document);
                }
                return Err(error);
            }
            registry.records[position].clone()
        };
        if task.kind == "search" {
            let publish = self.publish_search_task_state(
                &task,
                &task.status,
                task.error.as_deref(),
            ).await;
            if let Err(error) = publish {
                self.notify_task_publication_recovery_required(task_id, &task.status, &error)
                    .await;
                return Err(error);
            }
            self.clear_search_task_claim(task_id).await;
        }
        self.notify_task(&task).await;
        Ok(())
    }

    async fn task_checkpoint(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<bool, String> {
        loop {
            if !self.task_worker_is_current(task_id, signal).await {
                return Ok(false);
            }
            let current = *signal.borrow_and_update();
            if current.cancelled {
                let _operation = self.operation_gate.clone().read_owned().await;
                if !self.task_worker_is_current(task_id, signal).await {
                    return Ok(false);
                }
                self.finish_task(task_id, "cancelled", None).await?;
                return Ok(false);
            }
            if current.paused {
                if self.task_summary_record(task_id).await?.status != "paused" {
                    let _operation = self.operation_gate.clone().read_owned().await;
                    if !self.task_worker_is_current(task_id, signal).await {
                        return Ok(false);
                    }
                    let paused = self.update_task(task_id, |task| {
                        let pending = *signal.borrow();
                        // A later cancel/resume request wins. Never overwrite
                        // the durably recorded cancelling/running state with
                        // an acknowledgement of an older pause signal.
                        if pending.paused && !pending.cancelled
                            && matches!(task.status.as_str(), "pausing" | "running" | "queued")
                        {
                            task.status = "paused".into();
                        }
                    }).await?;
                    if paused.status == "paused" && paused.kind == "search" {
                        self.publish_search_task_state(&paused, "paused", None).await?;
                    }
                }
                // changed() immediately observes an unread resume/cancel and
                // also reports a closed channel. A separate has_changed()
                // poll would spin forever when all senders disappear.
                if signal.changed().await.is_err() {
                    return Ok(false);
                }
                continue;
            }
            if self.task_summary_record(task_id).await?.status != "running" {
                let _operation = self.operation_gate.clone().read_owned().await;
                if !self.task_worker_is_current(task_id, signal).await {
                    return Ok(false);
                }
                // A pause/cancel may have been persisted after this loop read
                // its watch signal. Recheck against the latest record while
                // holding the task registry lock in update_task, so an older
                // worker cannot overwrite "pausing" or "cancelling" with
                // "running" before acknowledging that control request.
                let running = self.update_task(task_id, |task| {
                    let pending = *signal.borrow();
                    if !pending.cancelled
                        && !pending.paused
                        && matches!(task.status.as_str(), "queued" | "interrupted" | "running")
                    {
                        task.status = "running".into();
                    }
                }).await?;
                if matches!(running.status.as_str(), "completed" | "failed" | "cancelled" | "recoveryRequired") {
                    return Ok(false);
                }
                if running.status != "running" {
                    continue;
                }
                if running.kind == "search" {
                    self.publish_search_task_state(&running, "running", None).await?;
                }
            }
            // A control request might arrive during the last state write.
            // Let the next checkpoint observe it before new work proceeds.
            if signal.borrow().cancelled || signal.borrow().paused {
                continue;
            }
            return Ok(true);
        }
    }

    async fn run_download_task(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<Option<String>, String> {
        loop {
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let task = self.task_summary_record(task_id).await?;
            let chapter_id = task
                .chapter_ids
                .as_ref()
                .filter(|ids| {
                    task.total > 0
                        && ids.len() == task.total
                        && task.completed <= ids.len()
                        && ids.iter().all(|id| !id.is_empty())
                })
                .and_then(|ids| ids.get(task.completed))
                .cloned();
            if task.completed >= task.total {
                if task.chapter_ids.as_ref().is_none_or(|ids| {
                    task.total == 0 || ids.len() != task.total || ids.iter().any(|id| id.is_empty())
                }) {
                    return Err("该旧下载任务缺少有效章节快照，请取消后重新下载。".to_owned());
                }
                return Ok(Some(String::new()));
            }
            let book_id = task
                .book_id
                .as_deref()
                .ok_or_else(|| "Download task has no bookId".to_owned())?;
            let chapter_id = chapter_id
                .ok_or_else(|| "该旧下载任务缺少有效章节快照，请取消后重新下载。".to_owned())?;
            let permit = tokio::select! {
                permit = self.task_slots.clone().acquire_owned() => Some(permit.map_err(|error| error.to_string())?),
                changed = signal.changed() => {
                    changed.map_err(|_| "Task control channel closed".to_owned())?;
                    None
                }
            };
            let Some(permit) = permit else {
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            };
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let _operation = self.operation_gate.clone().read_owned().await;
            if !self.task_worker_is_current(task_id, signal).await {
                drop(permit);
                return Ok(None);
            }
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(_operation);
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let index = task.from_index.saturating_add(task.completed);
            self.prepare_chapter_ids_unlocked(book_id, index, vec![chapter_id.clone()])
                .await
                .map_err(|error| {
                    format!(
                        "准备快照章节 {chapter_id} 失败（已完成 {}/{} 章）：{error}。重试会复核当前目录并继续仍存在的快照章节，不会按目录位置替换内容。",
                        task.completed,
                        task.total,
                    )
                })?;
            self.update_task(task_id, |task| {
                task.completed = task.completed.saturating_add(1)
            })
            .await?;
            drop(permit);
        }
    }

    async fn reconcile_search_document_progress(
        &self,
        task_id: &str,
        signal: &tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<(), String> {
        let _operation = self.operation_read().await;
        if !self.task_worker_is_current(task_id, signal).await {
            return Ok(());
        }
        let task = self.task_summary_record(task_id).await?;
        let document = self.read_search_task_state(&task).await?;
        let completed = document.get("completedSources").and_then(Value::as_u64)
            .ok_or_else(|| "Search document has no committed source progress".to_owned())?;
        let total = document.get("totalSources").and_then(Value::as_u64)
            .ok_or_else(|| "Search document has no total source count".to_owned())?;
        if let Some(completed) = reconciled_source_progress(
            task.completed, task.total, completed, total,
        )? {
            self.update_task(task_id, |current| {
                if current.search_id == task.search_id && current.completed < completed {
                    current.completed = completed;
                }
            }).await?;
        }
        Ok(())
    }

    async fn run_search_task(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<Option<String>, String> {
        // Reconcile before the first checkpoint can overwrite a newer
        // source count in this round's memory.
        self.reconcile_search_document_progress(task_id, signal).await?;
        let mut first_checkpoint = true;
        loop {
            // The initial queued -> running transition publishes its own
            // snapshot in task_checkpoint. Only a worker whose record was
            // already marked running (e.g. after resume) needs a separate
            // initial publication. Do not generate two immutable snapshots
            // for every new search.
            let already_running = first_checkpoint
                && self.task_summary_record(task_id).await?.status == "running";
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let task = self.task_summary_record(task_id).await?;
            if already_running && task.kind == "search" {
                self.publish_search_task_state(&task, "running", None).await?;
            }
            first_checkpoint = false;
            if task.completed >= task.total {
                let _operation = self.operation_gate.clone().read_owned().await;
                if !self.task_worker_is_current(task_id, signal).await {
                    return Ok(None);
                }
                let control = *signal.borrow_and_update();
                if control.paused || control.cancelled {
                    drop(_operation);
                    if !self.task_checkpoint(task_id, signal).await? {
                        return Ok(None);
                    }
                    continue;
                }
                return Ok(Some(String::new()));
            }
            let permit = tokio::select! {
                permit = self.task_slots.clone().acquire_owned() => Some(permit.map_err(|error| error.to_string())?),
                changed = signal.changed() => {
                    changed.map_err(|_| "Task control channel closed".to_owned())?;
                    None
                }
            };
            let Some(permit) = permit else {
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            };
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let _operation = self.operation_gate.clone().read_owned().await;
            if !self.task_worker_is_current(task_id, signal).await {
                drop(permit);
                return Ok(None);
            }
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(_operation);
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let source_ids = task
                .source_ids
                .as_ref()
                .ok_or_else(|| "Search task source list is incomplete".to_owned())?;
            let pending_source_ids = source_ids
                .iter()
                .skip(task.completed)
                .take(SEARCH_SOURCE_BATCH_SIZE)
                .cloned()
                .collect::<Vec<_>>();
            if pending_source_ids.is_empty() {
                return Err("Search task source list is incomplete".to_owned());
            }
            let source_slots = {
                let _sources = self.sources_lock.lock().await;
                let current_sources = self.read_sources().await?;
                let mut slots = Vec::with_capacity(pending_source_ids.len());
                for source_id in &pending_source_ids {
                    let Some(source) = current_sources.iter().find(|source| &source.id == source_id).cloned() else {
                        slots.push((
                            source_id.clone(),
                            None,
                            Some(format!("Search source '{source_id}' was removed")),
                        ));
                        continue;
                    };
                    if !source.enabled || crate::source_metadata::is_rss_source_metadata(&source.source) {
                        slots.push((
                            source_id.clone(),
                            None,
                            Some("Search source is no longer enabled for book search".to_owned()),
                        ));
                        continue;
                    }
                    let revision = self.source_revision(source_id).await?;
                    slots.push((source_id.clone(), Some((source, revision)), None));
                }
                slots
            };
            let sources = source_slots
                .iter()
                .filter_map(|(_, source, _)| source.clone())
                .collect::<Vec<_>>();
            let keyword = task
                .keyword
                .as_deref()
                .ok_or_else(|| "Search task has no keyword".to_owned())?;
            let result = if sources.is_empty() {
                Ok(json!({ "results": [] }))
            } else {
                let request_sources =
                    Value::Array(sources.iter().map(|(source, _)| source.source.clone()).collect());
                self.executor
                    .execute(engine_request(
                        "searchBatch",
                        &request_sources,
                        Some(keyword.to_owned()),
                        Some(task.page as i32),
                        None,
                        None,
                        None,
                    ))
                    .await
            };
            let responses = match result {
                Ok(value) => {
                    let batch_results = value.get("results").and_then(Value::as_array);
                    match batch_results.filter(|items| items.len() == sources.len()) {
                        Some(items) => items
                            .iter()
                            .map(|item| {
                                if let Some(error) = item.get("error").and_then(Value::as_str) {
                                    Err(error.to_owned())
                                } else if item.get("books").and_then(Value::as_array).is_some() {
                                    Ok(item.clone())
                                } else {
                                    Err("Source engine returned an invalid search result".to_owned())
                                }
                            })
                            .collect::<Vec<_>>(),
                        None => vec![
                            Err("Source engine returned an invalid search batch".to_owned());
                            sources.len()
                        ],
                    }
                }
                Err(error) => std::iter::repeat_with(|| Err(error.clone()))
                    .take(sources.len())
                    .collect(),
            };
            drop(permit);
            drop(_operation);
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let mut live_responses = responses.into_iter();
            let batch_result_count = source_slots.len();
            for (index, (source_id, source, slot_error)) in source_slots.into_iter().enumerate() {
                let response = if source.is_some() {
                    live_responses.next().unwrap_or_else(|| {
                        Err("Source engine returned an incomplete search batch".to_owned())
                    })
                } else {
                    Err(slot_error.unwrap_or_else(|| "Search source is unavailable".to_owned()))
                };
                loop {
                    if !self.task_checkpoint(task_id, signal).await? {
                        return Ok(None);
                    }
                    let _operation = self.operation_gate.clone().read_owned().await;
                    if !self.task_worker_is_current(task_id, signal).await {
                        return Ok(None);
                    }
                    let control = *signal.borrow_and_update();
                    if control.paused || control.cancelled {
                        drop(_operation);
                        continue;
                    }
                    let committed_task = self.task_summary_record(task_id).await?;
                    if let Some((source, source_revision)) = source {
                        self.append_search_source(
                            &committed_task,
                            &source,
                            source_revision,
                            response,
                            index + 1 == batch_result_count,
                        )
                        .await?;
                    } else {
                        let error = response.err().unwrap_or_else(|| "Search source is unavailable".to_owned());
                        let errors = vec![json!({
                            "sourceId": source_id,
                            "message": error,
                        })];
                        if committed_task.kind == "search" {
                            self.append_and_publish_search_snapshot(
                                &committed_task,
                                Vec::new(),
                                errors,
                                index + 1 == batch_result_count,
                            ).await?;
                        } else {
                            self.append_candidate_search_results(&committed_task, Vec::new(), errors)
                                .await?;
                        }
                    }
                    self.update_task(task_id, |task| {
                        task.completed = task.completed.saturating_add(1)
                    })
                    .await?;
                    break;
                }
            }
        }
    }

    async fn append_search_source(
        &self,
        task: &AppTask,
        source: &SourceRecord,
        source_revision: u64,
        response: Result<Value, String>,
        publish_search_snapshot: bool,
    ) -> Result<(), String> {
        let search_id = task
            .search_id
            .as_deref()
            .ok_or_else(|| "Search task has no search resource".to_owned())?;
        let _sources = self.sources_lock.lock().await;
        let current_source = self
            .read_sources()
            .await?
            .into_iter()
            .find(|current| current.id == source.id);
        let current_revision = self.source_revision(&source.id).await?;
        let source_is_current = current_source.is_some_and(|current| {
            current.enabled
                && current.source == source.source
                && current_revision == source_revision
        });
        let response = if source_is_current {
            response
        } else {
            Err("Source changed while search was running; search again".to_owned())
        };
        let mut results = Vec::new();
        let mut errors = Vec::new();
        let replacement_context = if task.kind == "bookSourceCandidates" {
            Some(self.read_search_replacement_context(search_id).await?)
        } else {
            None
        };
        match response {
            Ok(value) => {
                let books = value
                    .get("books")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let invalid_book_count = books.iter().filter(|book| !book.is_object()).count();
                if invalid_book_count > 0 {
                    errors.push(json!({
                        "sourceId": source.id,
                        "sourceName": source.name,
                        "message": format!("Source returned {invalid_book_count} invalid book entries"),
                    }));
                }
                for book in books.into_iter().filter(|book| book.is_object()) {
                    let replacement = if let Some(context) = &replacement_context {
                        let Some(requires_identity_confirmation) =
                            candidate_identity_match(context, &book)
                        else {
                            continue;
                        };
                        Some((context, requires_identity_confirmation))
                    } else {
                        None
                    };
                    let result_id = format!("result-{}", uuid::Uuid::new_v4().simple());
                    let source_fingerprint = source_definition_fingerprint(&source.source)?;
                    let mut private_result = json!({
                        "searchId": search_id,
                        "sourceId": source.id,
                        "sourceRevision": source_revision,
                        "sourceDefinitionFingerprint": source_fingerprint,
                        "book": book.clone(),
                    });
                    let mut projected = project_search_result(&result_id, source, &book);
                    if let Some((context, requires_confirmation)) = replacement {
                        projected.requires_identity_confirmation = Some(requires_confirmation);
                        private_result["replacementContext"] = json!({
                            "targetBookId": context["targetBookId"],
                            "originalSourceId": context["originalSourceId"],
                            "originalSourcePresent": context["originalSourcePresent"],
                            "originalSourceFingerprint": context["originalSourceFingerprint"],
                            "originalSourceRevision": context["originalSourceRevision"],
                            "catalogFingerprint": context["catalogFingerprint"],
                            "catalogGeneration": context["catalogGeneration"],
                            "bookInstanceId": context["bookInstanceId"],
                            "createdAtMs": context["createdAtMs"],
                            "expiresAtMs": context["expiresAtMs"],
                            "searchId": search_id,
                            "candidateSourceFingerprint": source_fingerprint,
                            "requiresIdentityConfirmation": requires_confirmation,
                        });
                    }
                    results.push((projected, private_result));
                }
            }
            Err(error) => errors.push(
                json!({ "sourceId": source.id, "sourceName": source.name, "message": error }),
            ),
        }
        if task.kind == "search" {
            self.append_and_publish_search_snapshot(task, results, errors, publish_search_snapshot)
                .await?;
        } else {
            self.append_candidate_search_results(task, results, errors).await?;
        }
        Ok(())
    }

    async fn run_refresh_task(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<Option<String>, String> {
        loop {
            if !self.task_checkpoint(task_id, signal).await? {
                return Ok(None);
            }
            let task = self.task_summary_record(task_id).await?;
            let book_id = task
                .book_id
                .as_deref()
                .ok_or_else(|| "Refresh task has no bookId".to_owned())?;
            let permit = tokio::select! {
                permit = self.task_slots.clone().acquire_owned() => Some(permit.map_err(|error| error.to_string())?),
                changed = signal.changed() => {
                    changed.map_err(|_| "Task control channel closed".to_owned())?;
                    None
                }
            };
            let Some(permit) = permit else {
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            };
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            let _operation = self.operation_gate.clone().read_owned().await;
            if !self.task_worker_is_current(task_id, signal).await {
                drop(permit);
                return Ok(None);
            }
            let control = *signal.borrow_and_update();
            if control.paused || control.cancelled {
                drop(_operation);
                drop(permit);
                if !self.task_checkpoint(task_id, signal).await? {
                    return Ok(None);
                }
                continue;
            }
            if task.completed >= task.total {
                return Ok(Some(String::new()));
            }
            let result = self
                .refresh_catalog_unlocked(task_id, book_id, !task.check_only)
                .await?;
            self.update_task(task_id, |task| {
                task.completed = 1;
                task.result = Some(result);
            })
            .await?;
            drop(permit);
            return Ok(Some(String::new()));
        }
    }

    async fn refresh_catalog_unlocked(
        &self,
        task_id: &str,
        book_id: &str,
        commit: bool,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let private_path = Path::new("books").join(format!("{book_id}.json"));
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;

        // Capture a coherent source/catalog snapshot while following the
        // global lock order (source metadata, then book). The source engine
        // call deliberately happens after both locks are released.
        let (source, engine_book, old_raw, old_chapters, catalog_generation) = {
            let _sources_lock = self.sources_lock.lock().await;
            let source_id = {
                let private = self.read_private_json(&private_path).await?;
                private["sourceId"]
                    .as_str()
                    .ok_or_else(|| "Private book is missing sourceId".to_owned())?
                    .to_owned()
            };
            let source = self
                .read_sources()
                .await?
                .into_iter()
                .find(|source| source.id == source_id)
                .ok_or_else(|| format!("Book source '{source_id}' is no longer imported"))?;
            let _book_lock = self.book_lock(book_id).await;
            let private = self.read_private_json(&private_path).await?;
            if private["sourceId"].as_str() != Some(source.id.as_str()) {
                return Err("Book source changed while refreshing its catalog".to_owned());
            }
            let engine_book = private
                .get("book")
                .cloned()
                .ok_or_else(|| "Private book is missing engine metadata".to_owned())?;
            let book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| "Book was removed while refreshing its catalog".to_owned())?;
            if book.get("id").and_then(Value::as_str) != Some(book_id) {
                return Err("Book resource ID does not match its catalog path".to_owned());
            }
            let raw = private
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| "Private book is missing its source chapter catalog".to_owned())?;
            let chapters = serde_json::from_value::<Vec<ChapterDescriptor>>(
                book.get("chapters")
                    .cloned()
                    .ok_or_else(|| "Book chapter directory is missing".to_owned())?,
            )
            .map_err(|error| format!("Cannot read processed chapter directory: {error}"))?;
            if raw.len() != chapters.len() {
                return Err("Book chapter directory is out of sync; keeping existing data".into());
            }
            let generation = private
                .get("catalogGeneration")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (source, engine_book, raw, chapters, generation)
        };

        let operation = if crate::rss::is_legacy_rss_source(&source.source) {
            "rssChapters"
        } else {
            "chapters"
        };
        let response = self
            .executor
            .execute(engine_request(
                operation,
                &source.source,
                None,
                None,
                Some(engine_book.clone()),
                None,
                None,
            ))
            .await?;
        let new_raw = response
            .as_array()
            .cloned()
            .ok_or_else(|| "Source engine returned an invalid chapter list".to_owned())?;

        // Reacquire locks in the same order and reject stale responses. A
        // delete or another refresh may finish while the source engine runs;
        // neither result is allowed to recreate or overwrite newer data.
        let result = {
            let _sources_lock = self.sources_lock.lock().await;
            let current_source = self
                .read_sources()
                .await?
                .into_iter()
                .find(|candidate| candidate.id == source.id)
                .ok_or_else(|| "Book source was removed while refreshing its catalog".to_owned())?;
            if current_source.source != source.source {
                return Err("Book source changed while refreshing its catalog".to_owned());
            }
            let _book_lock = self.book_lock(book_id).await;
            let current_private = self
                .read_private_json(&private_path)
                .await
                .map_err(|_| "Book was removed while refreshing its catalog".to_owned())?;
            if current_private["sourceId"].as_str() != Some(source.id.as_str())
                || current_private.get("book") != Some(&engine_book)
            {
                return Err("Book metadata changed while refreshing its catalog".to_owned());
            }
            let current_raw = current_private
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| "Private book is missing its source chapter catalog".to_owned())?;
            let current_catalog_generation = current_private
                .get("catalogGeneration")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if current_raw != old_raw || current_catalog_generation != catalog_generation {
                return Err(
                    "A newer chapter catalog was committed; discard this stale refresh".into(),
                );
            }
            let writer_guard = match self.store.transaction_writer_guard().await {
                Ok(guard) => guard,
                Err(error) => {
                    let warning = format_catalog_refresh_transaction_error(
                        crate::resource_transactions::CommitState::NotCommitted,
                        true,
                        &format!("Cannot acquire the resource transaction writer: {error}"),
                    );
                    self.notify_task_recovery_required(
                        task_id,
                        crate::resource_transactions::CommitState::NotCommitted,
                        warning.clone(),
                    )
                    .await;
                    return Err(warning);
                }
            };
            let mut current_book = writer_guard
                .read_json_ref(&book_ref)
                .map_err(|_| "Book was removed while refreshing its catalog".to_owned())?;
            let current_chapters = serde_json::from_value::<Vec<ChapterDescriptor>>(
                current_book
                    .get("chapters")
                    .cloned()
                    .ok_or_else(|| "Book chapter directory is missing".to_owned())?,
            )
            .map_err(|error| format!("Cannot read processed chapter directory: {error}"))?;
            if current_chapters.len() != old_chapters.len()
                || current_chapters
                    .iter()
                    .zip(&old_chapters)
                    .any(|(current, old)| {
                        current.id != old.id
                            || current.index != old.index
                            || current.title != old.title
                    })
            {
                return Err(
                    "A newer chapter directory was committed; discard this stale refresh".into(),
                );
            }

            let summary_progress = serde_json::from_value::<ProgressSummary>(
                current_book
                    .get("progress")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .map_err(|error| format!("Cannot read saved book progress: {error}"))?;
            let progress_ref = self
                .store
                .progress_ref(book_id)
                .map_err(|error| error.to_string())?;
            let progress_document = writer_guard
                .read_optional_json_ref(&progress_ref)
                .map_err(|error| error.to_string())?
                .and_then(|value| serde_json::from_value::<ProgressDocument>(value).ok())
                .filter(|progress| progress.book_id == book_id);
            // save_progress writes the book summary before its mirror. On a
            // timestamp tie the summary therefore wins; a newer mirror wins
            // when recovering from an interrupted older write.
            let progress = progress_document
                .filter(|document| document.updated_at_ms > summary_progress.updated_at_ms)
                .map(|document| ProgressSummary {
                    chapter_id: document.chapter_id,
                    chapter_index: document.chapter_index,
                    offset: document.offset,
                    updated_at_ms: document.updated_at_ms,
                })
                .unwrap_or(summary_progress);
            let mut cached_ids = HashSet::new();
            for chapter in &current_chapters {
                let Some(reference) = chapter.resource.as_ref().map(|resource| &resource.resource_id) else {
                    continue;
                };
                let scoped = scoped_chapter_cache_ref(book_id, reference.as_str())?;
                if self.cached_chapter_format(&scoped).await?.is_some() {
                    cached_ids.insert(chapter.id.clone());
                }
            }
            let refreshed_at_ms = now_ms().max(progress.updated_at_ms.saturating_add(1));
            let mut plan = crate::catalog::reconcile_catalog(
                book_id,
                &old_raw,
                &current_chapters,
                &new_raw,
                &progress,
                &cached_ids,
                refreshed_at_ms,
            )?;
            if let Some(generation) = catalog_generation.as_deref() {
                let old_ids = current_chapters
                    .iter()
                    .map(|chapter| chapter.id.as_str())
                    .collect::<HashSet<_>>();
                for (index, chapter) in plan.chapters.iter_mut().enumerate() {
                    if !old_ids.contains(chapter.id.as_str()) {
                        let stable_url = text_at(&new_raw[index], &["url", "chapterUrl"])
                            .unwrap_or_else(|| format!("@{index}"));
                        chapter.id = chapter_id_for_generation(book_id, generation, &stable_url);
                        chapter.resource = None;
                    }
                }
                if let Some(chapter) = plan.chapters.get(plan.progress.chapter_index) {
                    plan.progress.chapter_id = Some(chapter.id.clone());
                }
            }
            if !commit {
                return Ok(json!({
                    "bookResourceId": book_ref.as_str(),
                    "addedCount": plan.added_count,
                    "matchedCount": plan.matched_count,
                    "movedProgress": plan.progress_relocated,
                    "progressRelocated": plan.progress_relocated,
                    "committed": false,
                }));
            }

            let mut next_private = current_private.clone();
            next_private["chapters"] = json!(new_raw);
            current_book["chapters"] =
                serde_json::to_value(&plan.chapters).map_err(|error| error.to_string())?;
            current_book["chapterCount"] = json!(plan.chapters.len());
            current_book["latestChapter"] = json!(plan.latest_chapter);
            current_book["progress"] =
                serde_json::to_value(&plan.progress).map_err(|error| error.to_string())?;
            let next_progress = ProgressDocument {
                schema_version: CURRENT_SCHEMA_VERSION,
                book_id: book_id.to_owned(),
                chapter_id: plan.progress.chapter_id.clone(),
                chapter_index: plan.progress.chapter_index,
                offset: plan.progress.offset,
                updated_at_ms: plan.progress.updated_at_ms,
            };
            let next_progress =
                serde_json::to_value(next_progress).map_err(|error| error.to_string())?;

            let shelf_ref = self.store.shelf_ref();
            let mut next_shelf = writer_guard
                .read_json_ref(&shelf_ref)
                .map_err(|error| error.to_string())?;
            crate::book_commit::upsert_shelf_entry(&mut next_shelf, book_id, &current_book)?;

            let private_bytes = serde_json::to_vec_pretty(&next_private)
                .map_err(|error| format!("Cannot encode refreshed private book: {error}"))?;
            let replacements = vec![
                crate::resource_transactions::Replacement::private_book_json(
                    book_id,
                    private_bytes,
                )
                .map_err(|error| error.to_string())?,
                writer_guard
                    .public_json_replacement(&book_ref, &current_book)
                    .map_err(|error| error.to_string())?,
                writer_guard
                    .public_json_replacement(&progress_ref, &next_progress)
                    .map_err(|error| error.to_string())?,
                writer_guard
                    .public_json_replacement(&shelf_ref, &next_shelf)
                    .map_err(|error| error.to_string())?,
            ];
            let transaction_root = writer_guard.data_root().to_path_buf();
            let transaction_result = tokio::task::spawn_blocking(move || {
                let transaction = crate::resource_transactions::FileTransaction::prepare(
                    &transaction_root,
                    "refresh-catalog",
                    replacements,
                    Vec::new(),
                    &writer_guard,
                )?;
                transaction.commit()
            })
            .await;
            match transaction_result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    let commit_state = error.commit_state();
                    let recovery_required = error.recovery_required_flag();
                    let warning = format_catalog_refresh_transaction_error(
                        commit_state,
                        recovery_required,
                        &error.to_string(),
                    );
                    if recovery_required
                        || commit_state != crate::resource_transactions::CommitState::NotCommitted
                    {
                        self.notify_task_recovery_required(task_id, commit_state, warning.clone())
                            .await;
                    }
                    return Err(warning);
                }
                Err(error) => {
                    let warning = format!(
                        "Catalog refresh commit status is unknown because its file worker stopped before reporting a result. Do not retry yet; restart the application to recover the transaction first. Details: {error}"
                    );
                    self.notify_task_recovery_required(
                        task_id,
                        crate::resource_transactions::CommitState::Indeterminate,
                        warning.clone(),
                    )
                    .await;
                    return Err(warning);
                }
            }
            let result = json!({
                "bookResourceId": book_ref.as_str(),
                "addedCount": plan.added_count,
                "matchedCount": plan.matched_count,
                "movedProgress": plan.progress_relocated,
                "progressRelocated": plan.progress_relocated,
                "committed": true,
            });
            result
        };
        Ok(result)
    }
}
