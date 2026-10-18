//! 后台搜索、目录刷新与章节下载任务的执行过程。

use super::*;
use futures_util::{stream::FuturesUnordered, StreamExt};

// Each source can run JS, parse HTML and create a browser. Leave mobile CPU
// capacity for foreground content instead of admitting sixteen rule engines.
pub(super) const SEARCH_CONCURRENCY: usize = if cfg!(any(target_os = "ios", target_os = "android")) { 4 } else { 8 };
const SEARCH_PUBLICATION_INTERVAL: Duration = Duration::from_millis(250);
const SEARCH_SOURCE_TIMEOUT: Duration = Duration::from_secs(10);

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
        if status == "completed" {
            let task = self.task_summary_record(task_id).await?;
            if task.kind == "bookSourceCandidates"
                && reconcile_task_completion(&task.status, status) == Some("completed")
            {
                let search_id = task.search_id.as_deref().ok_or_else(|| "Candidate task has no search".to_owned())?;
                self.cache_source_candidates(search_id).await?;
            }
        }
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
                candidate_result = Some(json!({ "search": snapshot }));
            }
            registry.records[position].status = final_status.to_owned();
            registry.records[position].error =
                if final_status == status { error } else { None };
            if let Some(result) = candidate_result {
                registry.records[position].result = Some(result);
            }
            registry.records[position].updated_at_ms = now_ms();
            if let Err(error) = self.persist_task_changes(None, &[&registry.records[position]]).await {
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
        if task.kind == "bookSourceCandidates" {
            let notifier = self.search_notifiers.write().ok().and_then(|mut channels| channels.remove(task_id));
            if let Some(notifier) = notifier {
                notifier(json!({"taskId":task.id,"status":task.status,"complete":true,"incremental":true,
                    "results":[],"errors":[],"completedSources":task.completed,"totalSources":task.total,"error":task.error}));
            }
        }
        Ok(())
    }

    async fn task_checkpoint(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<bool, String> {
        self.task_checkpoint_with_cancel(task_id, signal, || {}).await
    }

    async fn task_checkpoint_with_cancel(
        &self,
        task_id: &str,
        signal: &mut tokio::sync::watch::Receiver<TaskSignal>,
        mut before_cancel: impl FnMut(),
    ) -> Result<bool, String> {
        loop {
            let status = {
                let registry = self.tasks.lock().await;
                if !registry.owns_worker(task_id, signal) { return Ok(false); }
                registry.records.iter().find(|task| task.id == task_id)
                    .ok_or_else(|| format!("Unknown task '{task_id}'"))?.status.clone()
            };
            let current = *signal.borrow_and_update();
            if current.cancelled {
                before_cancel();
                let _operation = self.operation_gate.clone().read_owned().await;
                if !self.task_worker_is_current(task_id, signal).await {
                    return Ok(false);
                }
                self.finish_task(task_id, "cancelled", None).await?;
                return Ok(false);
            }
            if current.paused {
                if status != "paused" {
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
            if status != "running" {
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
            if !self.cache_chapter_files_unlocked(book_id, &chapter_id, signal).await? {
                drop(_operation); drop(permit);
                continue;
            }
            if let Ok(notifier) = self.book_notifier.read() {
                if let Some(notifier) = notifier.as_ref() {
                    let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
                    notifier(json!({"requestId": 0, "resource": self.resource_descriptor(&reference)}));
                }
            }
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
        self.reconcile_search_document_progress(task_id, signal).await?;
        let already_running = self.task_summary_record(task_id).await?.status == "running";
        if !self.task_checkpoint(task_id, signal).await? { return Ok(None); }
        let mut task = self.task_summary_record(task_id).await?;
        if already_running && task.kind == "search" {
            self.publish_search_task_state(&task, "running", None).await?;
        }
        // Use the selected definitions for this round. Source edits take effect
        // when the user starts another search, not while results are arriving.
        let exhausted = if task.kind == "search" {
            self.read_search_task_state(&task).await?["exhaustedSourceIds"].as_array()
                .into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect::<HashSet<_>>()
        } else { HashSet::new() };
        let slots = {
            let _operation = self.operation_read().await;
            let records = self.read_sources().await?;
            let by_id = records.iter().map(|source| (source.id.as_str(), source)).collect::<HashMap<_, _>>();
            let mut slots = Vec::new();
            for id in task.source_ids.as_ref().ok_or("Search task source list is incomplete")?.iter().skip(task.completed) {
                let source = by_id.get(id.as_str()).copied().filter(|source| source.enabled
                    && !crate::source_metadata::is_rss_source_metadata(&source.source)
                    && task.source_type.as_deref().is_none_or(|kind| crate::source_metadata::matches_search_type(&source.source, kind)));
                let snapshot = match source {
                    Some(source) if !exhausted.contains(id) => Some(source.clone()),
                    _ => None,
                };
                slots.push((id.clone(), snapshot));
            }
            slots
        };
        let cancellation = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let mut pending = slots.into_iter();
        let mut running = FuturesUnordered::new();
        let mut last_progress_notification = Instant::now();
        let mut last_result_publication = None::<Instant>;
        let mut unpublished_results = false;
        // Search uses one background-task slot, with its own bounded network concurrency.
        let _permit = loop {
            tokio::select! {
                biased;
                changed = signal.changed() => {
                    if changed.is_err() || !self.task_checkpoint_with_cancel(task_id, signal, || { cancellation.cancel(); running.clear(); }).await? { return Ok(None); }
                }
                permit = self.task_slots.clone().acquire_owned() => break permit.map_err(|error| error.to_string())?,
            }
        };
        loop {
            if !self.task_checkpoint_with_cancel(task_id, signal, || { cancellation.cancel(); running.clear(); }).await? { return Ok(None); }
            let foreground_loading = self.detail_preparation.lock().unwrap_or_else(|error| error.into_inner())
                .1.as_ref().is_some_and(|token| !token.is_cancelled());
            let concurrency = if foreground_loading { 1 } else { SEARCH_CONCURRENCY };
            while running.len() < concurrency {
                if signal.borrow().paused || signal.borrow().cancelled { break; }
                let Some((id, source)) = pending.next() else { break; };
                let executor = self.executor.clone();
                let keyword = task.keyword.clone().ok_or("Search task has no keyword")?;
                let page = task.page;
                let token = cancellation.child_token();
                let operation_gate = self.operation_gate.clone();
                let search_slots = self.search_slots.clone();
                running.push(async move {
                    let _cancel_on_drop = token.clone().drop_guard();
                    let _operation = operation_gate.read_owned().await;
                    let response = if let Some(source) = source.as_ref() {
                        let mut request = engine_request("search", &source.source, Some(keyword), Some(page as i32), None, None, None);
                        request.cancellation = Some(token.clone());
                        tokio::select! {
                            biased;
                            _ = token.cancelled() => Err("Source search cancelled".into()),
                            result = async {
                                // Discovery and source-switch searches share the same CPU/network
                                // budget. Waiting for admission does not consume a source timeout.
                                let _slot = search_slots.acquire_owned().await.map_err(|error| error.to_string())?;
                                tokio::time::timeout(SEARCH_SOURCE_TIMEOUT, executor.execute(request)).await
                                    .unwrap_or_else(|_| Err("Source search timed out after 10000 ms".into()))
                            } => result,
                        }
                    } else { Err("Search source is unavailable or is not enabled".into()) };
                    (id, source, response)
                });
            }
            if running.is_empty() {
                if signal.borrow().paused || signal.borrow().cancelled { continue; }
                return Ok(Some(String::new()));
            }
            let completed = tokio::select! {
                biased;
                changed = signal.changed() => {
                    if changed.is_err() { return Ok(None); }
                    continue;
                }
                _ = tokio::time::sleep_until((last_result_publication.unwrap_or_else(Instant::now)
                    + SEARCH_PUBLICATION_INTERVAL).into()), if unpublished_results && task.kind == "search" => {
                    // Flush fast sources even if the remaining network requests stall.
                    self.publish_search_task_state(&task, "running", None).await?;
                    last_result_publication = Some(Instant::now());
                    unpublished_results = false;
                    continue;
                }
                completed = running.next() => completed,
            };
            let Some(completed) = completed else { continue; };
            let (id, source, response) = completed;
            if !self.task_checkpoint_with_cancel(task_id, signal, || { cancellation.cancel(); running.clear(); }).await? { return Ok(None); }
            let _operation = self.operation_gate.clone().read_owned().await;
            if !self.task_worker_is_current(task_id, signal).await { return Ok(None); }
            let publish = last_result_publication.is_none_or(|last| last.elapsed() >= SEARCH_PUBLICATION_INTERVAL);
            if let Some(source) = source {
                self.append_search_source(&task, &source, response, publish).await?;
            } else {
                let errors = if exhausted.contains(&id) { Vec::new() } else {
                    vec![json!({"sourceId": id, "message": response.err().unwrap_or_default()})]
                };
                if task.kind == "search" {
                    self.append_and_publish_search_snapshot(&task, Vec::new(), errors, publish, None).await?;
                } else {
                    self.append_candidate_search_results(&task, Vec::new(), errors).await?;
                }
            }
            if publish { last_result_publication = Some(Instant::now()); }
            unpublished_results = !publish;
            task.completed = task.completed.saturating_add(1);
            // Search documents and progress are session state. Lifecycle transitions
            // persist the latest progress; a result must not rewrite the task file.
            let mut registry = self.tasks.lock().await;
            if !registry.owns_worker(task_id, signal) { return Ok(None); }
            let progress = if let Some(task) = registry.records.iter_mut().find(|task| task.id == task_id) {
                task.completed = task.completed.saturating_add(1);
                task.updated_at_ms = now_ms();
                (last_progress_notification.elapsed() >= Duration::from_millis(250)).then(|| task.clone())
            } else { None };
            drop(registry);
            if last_progress_notification.elapsed() >= Duration::from_millis(250) {
                if let Some(progress) = progress { self.notify_task(&progress).await; }
                last_progress_notification = Instant::now();
            }
        }
    }

    async fn append_search_source(
        &self,
        task: &AppTask,
        source: &SourceRecord,
        response: Result<Value, String>,
        publish_search_snapshot: bool,
    ) -> Result<(), String> {
        let search_id = task
            .search_id
            .as_deref()
            .ok_or_else(|| "Search task has no search resource".to_owned())?;
        let response = response.and_then(|value| {
            if value.get("books").and_then(Value::as_array).is_some() { Ok(value) }
            else { Err("Source engine returned an invalid search result".into()) }
        });
        let mut results = Vec::new();
        let mut errors = Vec::new();
        let mut has_next_page = None;
        let replacement_context = if task.kind == "bookSourceCandidates" {
            Some(self.read_search_replacement_context(search_id).await?)
        } else {
            None
        };
        match response {
            Ok(value) => {
                has_next_page = value.get("hasNextPage").and_then(Value::as_bool);
                let books = value
                    .get("books")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let invalid_book_count = books.iter().filter(|book| !book.is_object()).count();
                if invalid_book_count > 0 {
                    errors.push(json!({
                        "sourceId": source.id,
                        "sourceName": source.name,
                        "message": format!("Source returned {invalid_book_count} invalid book entries"),
                    }));
                }
                for book in books.iter().filter(|book| book.is_object()) {
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
                    let mut private_result = json!({
                        "searchId": search_id,
                        "sourceId": source.id,
                        "book": book.clone(),
                    });
                    let mut projected = project_search_result(&result_id, source, &book);
                    if let Some((context, requires_confirmation)) = replacement {
                        projected.requires_identity_confirmation = Some(requires_confirmation);
                        private_result["replacementContext"] = json!({
                            "targetBookId": context["targetBookId"],
                            "originalSourceId": context["originalSourceId"],
                            "catalogGeneration": context["catalogGeneration"],
                            "bookInstanceId": context["bookInstanceId"],
                            "createdAtMs": context["createdAtMs"],
                            "expiresAtMs": context["expiresAtMs"],
                            "searchId": search_id,
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
            self.append_and_publish_search_snapshot(task, results, errors, publish_search_snapshot,
                has_next_page.map(|has_next| (source.id.as_str(), has_next)))
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
        let private_path = self.book_engine_path(book_id)?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;

        // Capture the book once; source edits do not invalidate this request.
        let (source, engine_book, old_raw, old_chapters, catalog_generation) = {
            let _book_lock = self.book_lock(book_id).await;
            let private = self.read_internal_json(&private_path).await?;
            let source_id = private["sourceId"].as_str()
                .ok_or_else(|| "Private book is missing sourceId".to_owned())?;
            let source = self
                .lookup_source(&source_id).await?
                .ok_or_else(|| format!("Book source '{source_id}' is no longer imported"))?;
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

        // Keep a catalog refresh from overwriting a newer catalog or deleted book.
        let result = {
            let _book_lock = self.book_lock(book_id).await;
            let current_private = self
                .read_internal_json(&private_path)
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
            if current_catalog_generation != catalog_generation
                || (catalog_generation.is_none() && current_raw != old_raw) {
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

            let progress = serde_json::from_value::<ProgressSummary>(
                current_book
                    .get("progress")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .map_err(|error| format!("Cannot read saved book progress: {error}"))?;
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

            if current_raw != new_raw { current_book["updatedAtMs"] = json!(refreshed_at_ms); }
            let mut next_private = current_private.clone();
            next_private["chapters"] = json!(new_raw);
            let mut chapters = serde_json::to_value(&plan.chapters).map_err(|error| error.to_string())?;
            for (index, chapter) in chapters.as_array_mut().into_iter().flatten().enumerate() {
                if let Some(previous) = current_book["chapters"].as_array().into_iter().flatten().find(|old| old["id"] == chapter["id"]) {
                    let previous_index = previous["index"].as_u64().unwrap_or(0) as usize;
                    if text_at(&current_raw[previous_index], &["url", "chapterUrl"]) != text_at(&new_raw[index], &["url", "chapterUrl"]) { continue; }
                    for key in ["pages", "sources", "protocol", "files", "url", "media", "volume"] {
                        if let Some(value) = previous.get(key) { chapter[key] = value.clone(); }
                    }
                    if previous["media"].is_object() { chapter["resource"] = previous["resource"].clone(); }
                }
            }
            current_book["chapters"] = chapters;
            current_book["chapterCount"] = json!(plan.chapters.len());
            current_book["latestChapter"] = json!(plan.latest_chapter);
            current_book["progress"] =
                serde_json::to_value(&plan.progress).map_err(|error| error.to_string())?;
            let replacements = vec![writer_guard.content_replacement(&book_ref, &current_book, &next_private)
                .map_err(|error| error.to_string())?];
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

#[cfg(test)]
mod search_task_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct PendingExecutor {
        entered: tokio::sync::Notify,
        dropped: Arc<AtomicBool>,
        token: std::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
    }
    struct DropNotice(Arc<AtomicBool>);
    impl Drop for DropNotice {
        fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); }
    }
    impl SourceExecutor for PendingExecutor {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                *self.token.lock().unwrap() = request.cancellation;
                let _drop = DropNotice(self.dropped.clone());
                self.entered.notify_one();
                std::future::pending().await
            })
        }
    }

    fn source(id: &str, source_type: u8) -> SourceRecord {
        SourceRecord {
            id: id.into(), name: id.into(), group: None, enabled: true,
            source: json!({"bookSourceName": id, "bookSourceUrl": format!("https://fixture/{id}"), "bookSourceType": source_type, "enabled": true}),
        }
    }

    async fn wait_for_terminal(service: &ApplicationService, id: &str) -> AppTask {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let task = service.task_summary_record(id).await.unwrap();
                if matches!(task.status.as_str(), "completed" | "cancelled" | "failed") { return task; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("Search should finish without waiting for a stuck source request")
    }

    #[tokio::test]
    async fn search_task_cancel_and_release_interrupt_pending_execution() {
        for release in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let executor = Arc::new(PendingExecutor {
                entered: tokio::sync::Notify::new(), dropped: Arc::new(AtomicBool::new(false)),
                token: std::sync::Mutex::new(None),
            });
            let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
            service.write_sources(&[source("text-1", 0)]).await.unwrap();
            let response = service.start_search("text", &["text-1".into()], "凡人", 1, true, None).await.unwrap();
            let id = response["taskId"].as_str().unwrap();
            tokio::time::timeout(Duration::from_secs(2), executor.entered.notified()).await.unwrap();
            if release { service.release_search_results(id).await.unwrap(); }
            else { service.cancel_task(id).await.unwrap(); }
            let terminal = wait_for_terminal(&service, id).await;
            assert_eq!(terminal.status, "cancelled");
            assert_eq!(terminal.completed, 0);
            assert!(executor.dropped.load(Ordering::SeqCst));
            assert!(executor.token.lock().unwrap().as_ref().unwrap().is_cancelled());
            // Cancel-after-finish and repeat cancellation are both harmless.
            service.cancel_task(id).await.unwrap();
            if release { assert!(service.get_search_task_results(id).await.is_err()); }
        }
    }

    #[derive(Default)]
    struct PagingExecutor(std::sync::Mutex<Vec<(i32, Vec<String>)>>);
    impl SourceExecutor for PagingExecutor {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                let page = request.page.unwrap();
                let source = &request.source;
                assert_eq!(request.operation, "search");
                assert_eq!(source["bookSourceType"], 0);
                self.0.lock().unwrap().push((page, vec![source["bookSourceName"].as_str().unwrap().to_owned()]));
                Ok(json!({
                    "books": [{"name": "凡人", "author": "作者", "bookUrl": format!("{}/book", source["bookSourceUrl"].as_str().unwrap())}],
                    "hasNextPage": source["bookSourceName"] == "text-2" && page == 1,
                }))
            })
        }
    }

    #[tokio::test]
    async fn search_task_paging_skips_exhausted_and_other_type_sources() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(PagingExecutor::default());
        let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
        service.write_sources(&[source("text-1", 0), source("text-2", 0), source("image-1", 2), source("audio-1", 1)]).await.unwrap();
        let ids = vec!["text-1".into(), "text-2".into(), "image-1".into(), "audio-1".into()];
        let first = service.start_search("text", &ids, "凡人", 1, true, None).await.unwrap();
        let first_id = first["taskId"].as_str().unwrap();
        assert_eq!(wait_for_terminal(&service, first_id).await.status, "completed");
        assert_eq!(service.get_search_task_results(first_id).await.unwrap()["hasMore"], true);
        let second = service.start_search("text", &ids, "凡人", 2, true, None).await.unwrap();
        let second_id = second["taskId"].as_str().unwrap();
        assert_eq!(wait_for_terminal(&service, second_id).await.status, "completed");
        let document = service.get_search_task_results(second_id).await.unwrap();
        assert_eq!(document["hasMore"], false);
        assert_eq!(document["results"].as_array().unwrap().len(), 1);
        assert_eq!(document["results"][0]["sources"].as_array().unwrap().len(), 2);
        let mut calls = executor.0.lock().unwrap().clone();
        calls.sort();
        assert_eq!(calls, vec![(1, vec!["text-1".to_owned()]), (1, vec!["text-2".to_owned()]), (2, vec!["text-2".to_owned()])]);
        assert!(service.start_search("text", &ids, "凡人", 3, true, None).await.is_err());
        // A different precision policy cannot inherit a previous page's results.
        assert!(service.start_search("text", &ids, "凡人", 2, false, None).await.is_err());
    }

    struct StreamingExecutor {
        release: tokio::sync::Notify,
        started: std::sync::atomic::AtomicUsize,
    }
    impl SourceExecutor for StreamingExecutor {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                self.started.fetch_add(1, Ordering::SeqCst);
                if request.source["bookSourceName"] == "slow" { self.release.notified().await; }
                Ok(json!({"books": [{"name": request.source["bookSourceName"], "bookUrl": "https://fixture/book"}], "hasNextPage": false}))
            })
        }
    }

    #[tokio::test]
    async fn search_delivers_every_result_before_terminal_ipc_even_for_a_fast_round() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(StreamingExecutor { release: tokio::sync::Notify::new(), started: 0.into() });
        let service = ApplicationService::open_with_executor(directory.path(), executor).await.unwrap();
        service.write_sources(&(0..30).map(|i| source(&format!("fast-{i}"), 0)).collect::<Vec<_>>()).await.unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let response = service.start_search("text", &[], "fixture", 1, false,
            Some(Arc::new(move |batch| { let _ = sender.send(batch); }))).await.unwrap();
        let mut delivered = HashSet::new();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let batch = receiver.recv().await.expect("Search channel must remain live until completion");
                for result in batch["results"].as_array().into_iter().flatten() {
                    delivered.insert(result["resultId"].as_str().unwrap().to_owned());
                }
                if batch["complete"] == true {
                    assert_eq!(batch["status"], "completed");
                    break;
                }
            }
        }).await.unwrap();
        assert_eq!(delivered.len(), 30);
        assert_eq!(service.get_search_task_results(response["taskId"].as_str().unwrap()).await.unwrap()["results"].as_array().unwrap().len(), 30);
    }

    #[tokio::test]
    async fn search_keeps_results_when_native_execution_scope_finishes() {
        struct ScopedExecutor;
        impl SourceExecutor for ScopedExecutor {
            fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
                Box::pin(async move {
                    // Desktop JNI and mobile plugin calls both hold this scope until return.
                    tokio::task::spawn_blocking(move || {
                        let mut wire = json!({"operation": request.operation});
                        let _execution = crate::source_host_ffi::ExecutionScope::enter(
                            &mut wire, PathBuf::new(), request.cancellation,
                        )?;
                        Ok(json!({"books": [{"name": "fixture", "bookUrl": "https://fixture/book"}], "hasNextPage": false}))
                    }).await.map_err(|error| error.to_string())?
                })
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let service = ApplicationService::open_with_executor(directory.path(), Arc::new(ScopedExecutor)).await.unwrap();
        service.write_sources(&[source("text-1", 0)]).await.unwrap();
        let response = service.start_search("text", &[], "fixture", 1, true, None).await.unwrap();
        let id = response["taskId"].as_str().unwrap();
        assert_eq!(wait_for_terminal(&service, id).await.status, "completed");
        let document = service.get_search_task_results(id).await.unwrap();
        assert_eq!(document["errors"], json!([]));
        assert_eq!(document["results"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn search_streams_and_refills_while_first_source_is_pending_without_progress_disk_writes() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(StreamingExecutor { release: tokio::sync::Notify::new(), started: 0.into() });
        let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
        let sources = std::iter::once(source("slow", 0))
            .chain((1..20).map(|index| source(&format!("fast-{index}"), 0))).collect::<Vec<_>>();
        service.write_sources(&sources).await.unwrap();
        let batches = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let output = batches.clone();
        let response = service.start_search("text", &[], "fixture", 1, false,
            Some(Arc::new(move |batch| output.lock().unwrap().push(batch)))).await.unwrap();
        let id = response["taskId"].as_str().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if service.task_summary_record(id).await.unwrap().completed == 19 { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("Fast sources, including sources past the first 16, must finish before slow source");
        assert_eq!(executor.started.load(Ordering::SeqCst), 20);
        assert_eq!(service.get_search_task_results(id).await.unwrap()["results"].as_array().unwrap().len(), 19);
        assert!(batches.lock().unwrap().iter().any(|batch| batch["completedSources"] == 1 && !batch["results"].as_array().unwrap().is_empty()));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if batches.lock().unwrap().iter().any(|batch| batch["completedSources"] == 19) { break; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("Buffered fast-source results must publish while the slow source is pending");
        let disk = service.store.read_raw_document(&service.store.document_ref("tasks").unwrap()).unwrap();
        let persisted = disk["tasks"].as_array().unwrap().iter().find(|task| task["id"] == id).unwrap();
        assert_eq!(persisted["completed"], 0, "Progress stays in memory until a lifecycle transition");
        let live = service.list_tasks().await.unwrap();
        let current = live["snapshot"]["tasks"].as_array().unwrap().iter().find(|task| task["id"] == id).unwrap();
        assert_eq!(current["completed"], 19, "Task center reads live progress without writing it to disk");
        executor.release.notify_one();
        assert_eq!(wait_for_terminal(&service, id).await.completed, 20);
    }

    #[tokio::test]
    async fn source_edits_do_not_block_or_discard_search_results() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(StreamingExecutor { release: tokio::sync::Notify::new(), started: 0.into() });
        let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
        let original = source("slow", 0);
        service.write_sources(&[original.clone()]).await.unwrap();
        let response = service.start_search("text", &[], "fixture", 1, false, None).await.unwrap();
        let id = response["taskId"].as_str().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while executor.started.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        let mut edited = original;
        edited.source["searchUrl"] = json!("https://fixture/changed");
        edited.source["bookSourceName"] = json!("edited");
        service.write_sources(&[edited]).await.unwrap();
        // A source editor may still own its guard while this search finishes.
        let sources_guard = service.sources_lock.lock().await;
        executor.release.notify_one();
        assert_eq!(wait_for_terminal(&service, id).await.status, "completed");
        let document = service.get_search_task_results(id).await.unwrap();
        assert_eq!(document["results"].as_array().unwrap().len(), 1);
        assert_eq!(document["results"][0]["title"], "slow");
        assert_eq!(document["errors"], json!([]));
        let registry = service.tasks.lock().await;
        for cache in registry.search_result_documents.values() {
            assert!(cache.get("sourceRevision").is_none());
            assert!(cache.get("sourceDefinitionFingerprint").is_none());
        }
        drop(registry);
        drop(sources_guard);
        let next = service.start_search("text", &[], "fixture", 1, false, None).await.unwrap();
        let next_id = next["taskId"].as_str().unwrap();
        assert_eq!(wait_for_terminal(&service, next_id).await.status, "completed");
        assert_eq!(service.get_search_task_results(next_id).await.unwrap()["results"][0]["title"], "edited");
    }

    struct CapacityExecutor {
        permits: tokio::sync::Semaphore,
        started: std::sync::atomic::AtomicUsize,
    }
    impl SourceExecutor for CapacityExecutor {
        fn execute<'a>(&'a self, _request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                self.started.fetch_add(1, Ordering::SeqCst);
                self.permits.acquire().await.unwrap().forget();
                Ok(json!({"books": [], "hasNextPage": false}))
            })
        }
    }

    #[tokio::test]
    async fn search_bounds_concurrency_and_replaces_one_finished_source() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(CapacityExecutor { permits: tokio::sync::Semaphore::new(0), started: 0.into() });
        let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
        service.write_sources(&(0..40).map(|i| source(&format!("source-{i}"), 0)).collect::<Vec<_>>()).await.unwrap();
        let response = service.start_search("text", &[], "fixture", 1, false, None).await.unwrap();
        let id = response["taskId"].as_str().unwrap();
        for expected in [SEARCH_CONCURRENCY, SEARCH_CONCURRENCY + 1] {
            tokio::time::timeout(Duration::from_secs(2), async {
                while executor.started.load(Ordering::SeqCst) < expected { tokio::task::yield_now().await; }
            }).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(executor.started.load(Ordering::SeqCst), expected);
            if expected == SEARCH_CONCURRENCY { executor.permits.add_permits(1); }
        }
        service.cancel_task(id).await.unwrap();
        assert_eq!(wait_for_terminal(&service, id).await.completed, 1);
    }

    #[tokio::test]
    async fn foreground_details_reduce_search_admission_until_the_request_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(CapacityExecutor { permits: tokio::sync::Semaphore::new(0), started: 0.into() });
        let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
        service.write_sources(&(0..40).map(|i| source(&format!("source-{i}"), 0)).collect::<Vec<_>>()).await.unwrap();
        let foreground = tokio_util::sync::CancellationToken::new();
        *service.detail_preparation.lock().unwrap() = (1, Some(foreground.clone()));
        let response = service.start_search("text", &[], "fixture", 1, false, None).await.unwrap();
        let id = response["taskId"].as_str().unwrap();
        for expected in [1, 2, SEARCH_CONCURRENCY + 2] {
            tokio::time::timeout(Duration::from_secs(2), async {
                while executor.started.load(Ordering::SeqCst) < expected { tokio::task::yield_now().await; }
            }).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(executor.started.load(Ordering::SeqCst), expected);
            if expected == 2 { foreground.cancel(); }
            if expected < SEARCH_CONCURRENCY + 2 { executor.permits.add_permits(1); }
        }
        service.cancel_task(id).await.unwrap();
        assert_eq!(wait_for_terminal(&service, id).await.completed, 2);
    }

    #[tokio::test]
    async fn search_source_is_terminated_after_ten_seconds() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(PendingExecutor { entered: tokio::sync::Notify::new(),
            dropped: Arc::new(AtomicBool::new(false)), token: std::sync::Mutex::new(None) });
        let service = ApplicationService::open_with_executor(directory.path(), executor.clone()).await.unwrap();
        service.write_sources(&[source("pending", 0)]).await.unwrap();
        let response = service.start_search("text", &[], "fixture", 1, false, None).await.unwrap();
        let id = response["taskId"].as_str().unwrap();
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                if service.task_summary_record(id).await.unwrap().status == "completed" { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        assert!(executor.dropped.load(Ordering::SeqCst));
        assert!(executor.token.lock().unwrap().as_ref().unwrap().is_cancelled());
        assert!(service.get_search_task_results(id).await.unwrap()["errors"][0]["message"].as_str().unwrap().contains("10000 ms"));
    }
}
