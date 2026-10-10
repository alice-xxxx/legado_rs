//! 任务注册表、持久化和状态通知。
use super::*;

impl TaskRegistry {
    // Task IDs can be restored from a backup. The watch channel, not the ID,
    // is the worker lease; check it under the same registry lock as mutations.
    pub(in crate::application) fn owns_worker(
        &self,
        task_id: &str,
        signal: &tokio::sync::watch::Receiver<TaskSignal>,
    ) -> bool {
        self.signals.get(task_id).is_some_and(|sender| {
            sender.subscribe().same_channel(signal)
        })
    }
}

impl ApplicationService {
    pub(in crate::application) async fn create_task(
        &self,
        task: AppTask,
    ) -> Result<tokio::sync::watch::Receiver<TaskSignal>, String> {
        if task.kind == "search" {
            return self.create_search_task(task).await;
        }
        let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
        {
            let mut registry = self.tasks.lock().await;
            registry.records.push(task.clone());
            registry.signals.insert(task.id.clone(), sender);
            if let Err(error) = self.persist_tasks_locked(&registry).await {
                registry.signals.remove(&task.id);
                registry.records.pop();
                return Err(error);
            }
        }
        self.notify_task(&task).await;
        Ok(receiver)
    }

    pub(in crate::application) async fn create_candidate_search_task(
        &self,
        task: AppTask,
        document: Value,
        context: Value,
        reused_results: Vec<(SearchResultSource, Value)>,
    ) -> Result<tokio::sync::watch::Receiver<TaskSignal>, String> {
        if task.kind != "bookSourceCandidates" {
            return Err("Candidate search task has an invalid kind".into());
        }
        let search_id = task.search_id.as_deref()
            .ok_or_else(|| "Candidate search has no search resource".to_owned())?
            .to_owned();
        if document.get("taskId").and_then(Value::as_str) != Some(task.id.as_str())
            || document.get("searchId").and_then(Value::as_str) != Some(search_id.as_str())
        {
            return Err("Candidate search document does not match its task".into());
        }
        let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
        {
            let mut registry = self.tasks.lock().await;
            if reused_results.iter().any(|(_, cache)| {
                cache.get("originSearchId").and_then(Value::as_str)
                    .is_none_or(|origin_id| !registry.search_documents.contains_key(origin_id))
            }) {
                return Err("Discovery results expired while source candidates were starting; search again".into());
            }
            registry.search_documents.insert(search_id.clone(), document);
            registry.search_replacement_contexts.insert(search_id.clone(), context);
            for (result, cache) in &reused_results {
                let mut cache = cache.clone();
                if let Some(object) = cache.as_object_mut() {
                    object.remove("originSearchId");
                }
                registry.search_result_documents.insert(result.result_id.clone(), cache);
            }
            registry.records.push(task.clone());
            registry.signals.insert(task.id.clone(), sender);
            if let Err(error) = self.persist_tasks_locked(&registry).await {
                registry.signals.remove(&task.id);
                registry.records.pop();
                registry.search_documents.remove(&search_id);
                registry.search_replacement_contexts.remove(&search_id);
                for (result, _) in reused_results {
                    registry.search_result_documents.remove(&result.result_id);
                }
                return Err(error);
            }
        }
        self.notify_task(&task).await;
        Ok(receiver)
    }

    async fn create_search_task(
        &self,
        task: AppTask,
    ) -> Result<tokio::sync::watch::Receiver<TaskSignal>, String> {
        let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
        let (previous_to_notify, candidates_to_notify) = {
            let mut registry = self.tasks.lock().await;
            let previous_id = registry.active_search_task_id.clone();
            let previous_backup = previous_id.as_ref().and_then(|id| {
                registry.records.iter().position(|record| record.id == *id)
                    .map(|position| (position, registry.records[position].clone()))
            });
            let mut previous_to_notify = None;
            let mut candidate_backups = Vec::new();
            let mut candidates_to_notify = Vec::new();

            if let Some((position, _)) = previous_backup.as_ref() {
                let previous_id = registry.records[*position].id.clone();
                let has_worker = registry.signals.contains_key(&previous_id);
                let previous = &mut registry.records[*position];
                if !matches!(previous.status.as_str(), "completed" | "failed" | "cancelled" | "recoveryRequired") {
                    previous.status = if has_worker { "cancelling".into() } else { "cancelled".into() };
                    previous.updated_at_ms = now_ms();
                    previous_to_notify = Some(previous.clone());
                }
            }

            for position in 0..registry.records.len() {
                if registry.records[position].kind != "bookSourceCandidates"
                    || matches!(registry.records[position].status.as_str(), "completed" | "failed" | "cancelled" | "recoveryRequired")
                {
                    continue;
                }
                let backup = registry.records[position].clone();
                let candidate_id = registry.records[position].id.clone();
                let has_worker = registry.signals.contains_key(&candidate_id);
                {
                    let candidate = &mut registry.records[position];
                    candidate.status = if has_worker { "cancelling" } else { "cancelled" }.into();
                    candidate.error = if has_worker {
                        None
                    } else {
                        Some("Candidate results expired after a new search; search this book again".into())
                    };
                    candidate.updated_at_ms = now_ms();
                }
                candidate_backups.push((position, backup));
                candidates_to_notify.push(registry.records[position].clone());
            }

            registry.records.push(task.clone());
            registry.signals.insert(task.id.clone(), sender);
            registry.active_search_task_id = Some(task.id.clone());
            if let Err(error) = self.persist_tasks_locked(&registry).await {
                registry.signals.remove(&task.id);
                registry.records.pop();
                registry.active_search_task_id = previous_id.clone();
                if let Some((position, backup)) = previous_backup {
                    registry.records[position] = backup;
                }
                for (position, backup) in candidate_backups {
                    registry.records[position] = backup;
                }
                return Err(error);
            }

            // Search data belongs to one discovery round. A new search drops
            // prior round results and candidate contexts from memory, while
            // leaving unrelated home/debug result caches available.
            let old_round_ids = registry.search_documents.keys().cloned().collect::<HashSet<_>>();
            registry.search_documents.clear();
            registry.search_result_documents.retain(|_, cache| {
                !cache.get("searchId").and_then(Value::as_str)
                    .is_some_and(|search_id| old_round_ids.contains(search_id))
            });
            registry.search_replacement_contexts.clear();
            registry.search_book_group_roots.clear();
            if let Some(search_id) = task.search_id.as_deref() {
                registry
                    .search_documents
                    .insert(search_id.to_owned(), empty_search_task_document(&task));
            }

            for candidate in &candidates_to_notify {
                if let Some(signal) = registry.signals.get(&candidate.id) {
                    signal.send_modify(|state| state.cancelled = true);
                }
            }

            if let Some(previous_id) = previous_id {
                if let Some(previous_signal) = registry.signals.get(&previous_id) {
                    previous_signal.send_modify(|state| state.cancelled = true);
                }
            }
            (previous_to_notify, candidates_to_notify)
        };
        if let Some(previous) = previous_to_notify {
            self.notify_task(&previous).await;
        }
        for candidate in candidates_to_notify {
            self.notify_task(&candidate).await;
        }
        self.notify_task(&task).await;
        Ok(receiver)
    }

    pub(in crate::application) async fn claim_search_task(&self, task_id: &str) -> Result<(), String> {
        let previous_to_notify = {
            let mut registry = self.tasks.lock().await;
            let target = registry.records.iter().find(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            if target.kind != "search" {
                return Err("Only a search task can own search publication".into());
            }
            // A resumed search can be paused or cancelled between creating its
            // worker signal and claiming the single search publisher slot.
            // Do not evict another running search for a request that will
            // never actually start.
            if !matches!(target.status.as_str(), "queued" | "running")
                || !registry.signals.contains_key(task_id)
            {
                return Err("Search task is no longer eligible to publish results".into());
            }
            let target_search_id = target.search_id.as_deref()
                .ok_or_else(|| "Search task has no search resource".to_owned())?;
            if !registry.search_documents.get(target_search_id).is_some_and(|document| {
                document.get("taskId").and_then(Value::as_str) == Some(task_id)
            }) {
                return Err("Search round was superseded; start a new search".into());
            }
            if registry.active_search_task_id.as_deref() == Some(task_id) {
                return Ok(());
            }

            let previous_id = registry.active_search_task_id.clone();
            let previous_backup = previous_id.as_ref().and_then(|id| {
                registry.records.iter().position(|record| record.id == *id)
                    .map(|position| (position, registry.records[position].clone()))
            });
            let mut previous_to_notify = None;
            let mut previous_changed = false;
            if let Some((position, _)) = previous_backup.as_ref() {
                let previous_id = registry.records[*position].id.clone();
                let has_worker = registry.signals.contains_key(&previous_id);
                let previous = &mut registry.records[*position];
                if !matches!(previous.status.as_str(), "completed" | "failed" | "cancelled" | "recoveryRequired") {
                    previous.status = if has_worker { "cancelling".into() } else { "cancelled".into() };
                    previous.updated_at_ms = now_ms();
                    previous_to_notify = Some(previous.clone());
                    previous_changed = true;
                }
            }

            registry.active_search_task_id = Some(task_id.to_owned());
            if previous_changed {
                if let Err(error) = self.persist_tasks_locked(&registry).await {
                    registry.active_search_task_id = previous_id.clone();
                    if let Some((position, backup)) = previous_backup {
                        registry.records[position] = backup;
                    }
                    return Err(error);
                }
            }
            if let Some(previous_id) = previous_id {
                if let Some(previous_signal) = registry.signals.get(&previous_id) {
                    previous_signal.send_modify(|state| state.cancelled = true);
                }
            }
            previous_to_notify
        };
        if let Some(previous) = previous_to_notify {
            self.notify_task(&previous).await;
        }
        Ok(())
    }

    pub(in crate::application) async fn search_task_is_publisher(&self, task_id: &str) -> bool {
        self.tasks.lock().await.active_search_task_id.as_deref() == Some(task_id)
    }

    pub(in crate::application) async fn clear_search_task_claim(&self, task_id: &str) {
        let mut registry = self.tasks.lock().await;
        if registry.active_search_task_id.as_deref() == Some(task_id) {
            registry.active_search_task_id = None;
        }
    }

    pub(super) async fn create_catalog_task(
        &self,
        task: AppTask,
    ) -> Result<
        (
            AppTask,
            Option<tokio::sync::watch::Receiver<TaskSignal>>,
            bool,
        ),
        String,
    > {
        let mut registry = self.tasks.lock().await;
        if let Some(existing) = registry
            .records
            .iter()
            .find(|existing| {
                existing.book_id == task.book_id
                    && matches!(
                        existing.kind.as_str(),
                        "refreshChapters" | "checkNewChapters"
                    )
                    && matches!(
                        existing.status.as_str(),
                        "queued"
                            | "running"
                            | "pausing"
                            | "paused"
                            | "cancelling"
                            | "interrupted"
                            | "recoveryRequired"
                    )
            })
            .cloned()
        {
            return Ok((existing, None, true));
        }

        let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
        registry.records.push(task.clone());
        registry.signals.insert(task.id.clone(), sender);
        if let Err(error) = self.persist_tasks_locked(&registry).await {
            registry.signals.remove(&task.id);
            registry.records.pop();
            return Err(error);
        }
        drop(registry);
        self.notify_task(&task).await;
        Ok((task, Some(receiver), false))
    }

    pub(super) async fn create_download_task(
        &self,
        task: AppTask,
    ) -> Result<
        (
            AppTask,
            Option<tokio::sync::watch::Receiver<TaskSignal>>,
            bool,
        ),
        String,
    > {
        let requested_ids = task
            .chapter_ids
            .as_deref()
            .ok_or_else(|| "Download task has no chapterIds snapshot".to_owned())?;
        let requested_set = requested_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let mut registry = self.tasks.lock().await;
        if let Some(existing) = registry
            .records
            .iter()
            .find(|existing| {
                if existing.book_id != task.book_id
                    || existing.kind != "chapterDownload"
                    || !matches!(
                        existing.status.as_str(),
                        "queued"
                            | "running"
                            | "pausing"
                            | "paused"
                            | "cancelling"
                            | "interrupted"
                            | "recoveryRequired"
                    )
                {
                    return false;
                }
                let Some(existing_ids) = existing
                    .chapter_ids
                    .as_ref()
                    .filter(|ids| ids.len() == existing.total && existing.completed <= ids.len())
                    .map(|ids| &ids[existing.completed..])
                else {
                    return false;
                };
                let remaining_ids = existing_ids.iter().map(String::as_str).collect::<HashSet<_>>();
                !requested_set.is_disjoint(&remaining_ids)
            })
            .cloned()
        {
            let remaining_ids = existing
                .chapter_ids
                .as_ref()
                .map(|ids| &ids[existing.completed..])
                .expect("matching active download has a chapterIds snapshot")
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            if requested_set.is_subset(&remaining_ids) {
                return Ok((existing, None, true));
            }
            return Err(
                "An active chapter download overlaps this request. Wait for it to finish before starting the overlapping chapters.".to_owned(),
            );
        }

        let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
        registry.records.push(task.clone());
        registry.signals.insert(task.id.clone(), sender);
        if let Err(error) = self.persist_tasks_locked(&registry).await {
            registry.signals.remove(&task.id);
            registry.records.pop();
            return Err(error);
        }
        drop(registry);
        self.notify_task(&task).await;
        Ok((task, Some(receiver), false))
    }

    pub(in crate::application) async fn persist_tasks_locked(
        &self,
        registry: &TaskRegistry,
    ) -> Result<(), String> {
        self.persist_task_records_locked(&registry.records).await
    }

    pub(super) async fn persist_task_records_locked(
        &self,
        records: &[AppTask],
    ) -> Result<(), String> {
        let reference = self
            .store
            .reading_ref("tasks")
            .map_err(|error| error.to_string())?;
        let value = json!({ "schemaVersion": CURRENT_SCHEMA_VERSION, "tasks": records });
        self.store
            .write_json_ref(&reference, &value)
            .await
            .map_err(|error| error.to_string())
    }

    pub(in crate::application) fn task_resource_descriptor(&self) -> Result<Value, String> {
        let resource = self
            .store
            .reading_ref("tasks")
            .map_err(|error| error.to_string())?;
        Ok(self.resource_descriptor(&resource))
    }

    pub(in crate::application) async fn task_summary_by_id(
        &self,
        task_id: &str,
    ) -> Result<Value, String> {
        let registry = self.tasks.lock().await;
        let task = registry
            .records
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
        Ok(task_summary(task))
    }

    pub(in crate::application) async fn notify_task(&self, task: &AppTask) {
        let Ok(resource) = self.task_resource_descriptor() else {
            return;
        };
        let event = json!({ "taskId": task.id, "resource": resource });
        if let Ok(notifier) = self.task_notifier.read() {
            if let Some(notifier) = notifier.as_ref() {
                notifier(event);
            }
        }
    }

    /// A worker failed without committing an authoritative terminal task
    /// state (e.g. storage refused the state write). Keep the task locked in
    /// memory and notify the UI explicitly: the task JSON on disk must not
    /// be misrepresented as having committed a failure.
    pub(in crate::application) async fn notify_task_execution_recovery_required(
        &self,
        task_id: &str,
        signal: &tokio::sync::watch::Receiver<TaskSignal>,
        error: &str,
    ) {
        let task = {
            let mut registry = self.tasks.lock().await;
            if !registry.owns_worker(task_id, signal) {
                return;
            }
            let Some(task) = registry.records.iter_mut().find(|task| task.id == task_id) else {
                return;
            };
            if matches!(
                task.status.as_str(),
                "completed" | "failed" | "cancelled" | "recoveryRequired"
            ) {
                return;
            }
            task.status = "recoveryRequired".into();
            task.error = Some(format!(
                "Task worker stopped without a durable terminal state: {error}. Restart the app to recover."
            ));
            task.updated_at_ms = now_ms();
            task.clone()
        };
        // This is explicitly a transient exception event. No ordinary task
        // resource update may claim this in-memory state was persisted.
        if let Ok(notifier) = self.task_recovery_notifier.read() {
            if let Some(notifier) = notifier.as_ref() {
                notifier(task_summary(&task));
            }
        }
    }

    /// The task status was already committed to tasks.json, but its search
    /// document/snapshot publication failed afterwards. Present a transient
    /// exception without changing or falsely rewriting the durable result.
    pub(in crate::application) async fn notify_task_publication_recovery_required(
        &self,
        task_id: &str,
        persisted_status: &str,
        error: &str,
    ) {
        let task = {
            let mut registry = self.tasks.lock().await;
            let Some(task) = registry.records.iter_mut().find(|task| task.id == task_id) else {
                return;
            };
            if task.status != persisted_status || task.status == "recoveryRequired" {
                return;
            }
            task.status = "recoveryRequired".into();
            task.error = Some(format!(
                "Task state was committed as '{persisted_status}', but its search results could not be published: {error}. Restart and repeat the search if results are missing."
            ));
            let result = task.result.get_or_insert_with(|| json!({}));
            if !result.is_object() {
                *result = json!({});
            }
            result["publicationFailed"] = json!(true);
            result["persistedTaskStatus"] = json!(persisted_status);
            result["recoveryRequired"] = json!(true);
            task.updated_at_ms = now_ms();
            task.clone()
        };
        if let Ok(notifier) = self.task_recovery_notifier.read() {
            if let Some(notifier) = notifier.as_ref() {
                notifier(task_summary(&task));
            }
        }
    }

    pub(in crate::application) async fn notify_task_recovery_required(
        &self,
        task_id: &str,
        commit_state: crate::resource_transactions::CommitState,
        warning: String,
    ) {
        let commit_state = match commit_state {
            crate::resource_transactions::CommitState::NotCommitted => "notCommitted",
            crate::resource_transactions::CommitState::Committed => "committed",
            crate::resource_transactions::CommitState::Indeterminate => "indeterminate",
        };
        let task = {
            let mut registry = self.tasks.lock().await;
            let Some(task) = registry.records.iter_mut().find(|task| task.id == task_id) else {
                return;
            };
            task.status = "recoveryRequired".into();
            if commit_state == "committed" {
                task.completed = task.total;
            }
            task.error = Some(warning.clone());
            task.result = Some(json!({
                "commitState": commit_state,
                "recoveryRequired": true,
                "warning": warning,
            }));
            task.updated_at_ms = now_ms();
            task.clone()
        };
        // The pending journal blocks task JSON writes too, so no committed
        // tasks resource can represent this state. Publish this one explicit
        // exceptional in-memory signal instead of mixing it with normal task
        // resource notifications.
        if let Ok(notifier) = self.task_recovery_notifier.read() {
            if let Some(notifier) = notifier.as_ref() {
                notifier(task_summary(&task));
            }
        }
    }

    pub(in crate::application) async fn update_task<F>(
        &self,
        task_id: &str,
        update: F,
    ) -> Result<AppTask, String>
    where
        F: FnOnce(&mut AppTask),
    {
        let task = {
            let mut registry = self.tasks.lock().await;
            let position = registry
                .records
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            let previous = registry.records[position].clone();
            update(&mut registry.records[position]);
            registry.records[position].updated_at_ms = now_ms();
            if let Err(error) = self.persist_tasks_locked(&registry).await {
                registry.records[position] = previous;
                return Err(error);
            }
            registry.records[position].clone()
        };
        self.notify_task(&task).await;
        Ok(task)
    }
}

pub(super) fn task_summary(task: &AppTask) -> Value {
    let mut summary = json!({
        "id": task.id,
        "kind": task.kind,
        "status": task.status,
        "completed": task.completed,
        "total": task.total,
        "createdAtMs": task.created_at_ms,
        "updatedAtMs": task.updated_at_ms,
    });
    if let Some(book_id) = &task.book_id {
        summary["bookId"] = json!(book_id);
    }
    if task.kind == "chapterDownload" {
        let snapshot_valid = task.total > 0
            && task.completed <= task.total
            && task.chapter_ids.as_ref().is_some_and(|ids| {
                ids.len() == task.total
                    && ids.iter().all(|id| !id.is_empty())
                    && ids.iter().collect::<HashSet<_>>().len() == ids.len()
            });
        summary["downloadSnapshotValid"] = json!(snapshot_valid);
    }
    if task.kind == "search" {
        if let Some(search_id) = &task.search_id {
            summary["searchId"] = json!(search_id);
        }
    }
    if let Some(error) = &task.error {
        summary["error"] = json!(error);
    }
    if let Some(result) = &task.result {
        summary["result"] = result.clone();
    }
    summary
}
