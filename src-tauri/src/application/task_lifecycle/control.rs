//! 后台任务的暂停、恢复和取消控制。
use super::*;

impl ApplicationService {
    pub async fn pause_task(&self, task_id: &str) -> Result<Value, String> {
        self.control_task(task_id, "pause").await
    }

    pub async fn resume_task(&self, task_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let mut restart = None;
        let (task, candidates_to_notify) = {
            let mut registry = self.tasks.lock().await;
            let position = registry
                .records
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            let signal = registry.signals.get(task_id).cloned();
            let previous = registry.records[position].clone();
            let task_search_id = previous.search_id.clone();
            if previous.kind == "search" && task_search_id.is_none() {
                return Err("Search task has no search resource".into());
            }
            let candidate_state_active = previous.kind != "bookSourceCandidates"
                || task_search_id.as_deref().is_some_and(|search_id| {
                    registry.search_replacement_contexts.contains_key(search_id)
                        && registry.search_documents.get(search_id).is_some_and(|document| {
                            document.get("taskId").and_then(Value::as_str) == Some(task_id)
                        })
                });
            if !candidate_state_active {
                return Err("Candidate search state expired; search this book again".into());
            }
            let reset_search_document = previous.kind == "search" && signal.is_none();
            let (task, new_signal) = {
                let task = &mut registry.records[position];
                if matches!(
                    task.status.as_str(),
                    "completed" | "failed" | "cancelled" | "cancelling" | "recoveryRequired"
                ) {
                    return Err(format!("Task cannot resume from status '{}'", task.status));
                }
                if task.kind == "chapterDownload"
                    && (task.total == 0
                        || task.completed > task.total
                        || task.chapter_ids.as_ref().is_none_or(|ids| {
                            ids.len() != task.total
                                || ids.iter().any(|id| id.is_empty())
                                || ids.iter().collect::<HashSet<_>>().len() != ids.len()
                        }))
                {
                    return Err("该旧下载任务缺少有效章节快照，请取消后重新下载。".to_owned());
                }
                if signal.is_some() {
                    // An existing worker, not this UI command, owns the
                    // queued -> running transition and its search snapshot.
                    // Keep a genuinely running worker unchanged on duplicate
                    // resume, but make paused/pausing work wait for its next
                    // checkpoint before claiming to be running.
                    if task.status != "running" {
                        task.status = "queued".into();
                    }
                    task.updated_at_ms = now_ms();
                    (task.clone(), None)
                } else if matches!(
                    task.status.as_str(),
                    "paused" | "pausing" | "interrupted" | "queued" | "running"
                ) {
                    // A task may have lost its worker after a failed status
                    // write. No active control channel means an old "running"
                    // record is not evidence of live execution. A user-issued
                    // resume replaces it with a newly owned queued worker.
                    task.status = "queued".into();
                    if reset_search_document {
                        task.completed = 0;
                    }
                    task.updated_at_ms = now_ms();
                    let (sender, receiver) = tokio::sync::watch::channel(TaskSignal::default());
                    restart = Some(receiver);
                    (task.clone(), Some(sender))
                } else {
                    return Err(format!("Task cannot resume from status '{}'", task.status));
                }
            };
            let mut candidate_backups = Vec::new();
            let mut candidates_to_notify = Vec::new();
            if reset_search_document {
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
            }
            if let Err(error) = self.persist_tasks_locked(&registry).await {
                registry.records[position] = previous;
                for (position, backup) in candidate_backups {
                    registry.records[position] = backup;
                }
                return Err(error);
            }
            if reset_search_document {
                let reset_task = registry.records[position].clone();
                let old_round_ids = registry.search_documents.keys().cloned().collect::<HashSet<_>>();
                registry.search_documents.clear();
                registry.search_result_documents.retain(|_, cache| {
                    !cache.get("searchId").and_then(Value::as_str)
                        .is_some_and(|search_id| old_round_ids.contains(search_id))
                });
                registry.search_replacement_contexts.clear();
                registry.search_book_group_roots.clear();
                if let Some(search_id) = reset_task.search_id.as_deref() {
                    registry.search_documents.insert(
                        search_id.to_owned(),
                        empty_search_task_document(&reset_task),
                    );
                }
                for candidate in &candidates_to_notify {
                    if let Some(candidate_signal) = registry.signals.get(&candidate.id) {
                        candidate_signal.send_modify(|state| state.cancelled = true);
                    }
                }
            }
            if let Some(sender) = new_signal {
                registry.signals.insert(task_id.to_owned(), sender);
            }
            // Do not wake a paused worker until its resumed state is durable.
            if let Some(sender) = signal {
                sender.send_modify(|state| state.paused = false);
            }
            (task, candidates_to_notify)
        };
        for candidate in candidates_to_notify {
            self.notify_task(&candidate).await;
        }
        if task.kind == "search" && restart.is_some() {
            if let Err(error) = self.claim_search_task(&task.id).await {
                // No worker has been spawned yet. A failed publication claim
                // must not leave a queued task with an unused signal forever.
                let recovered = {
                    let mut registry = self.tasks.lock().await;
                    registry.signals.remove(&task.id);
                    if let Some(position) = registry.records.iter().position(|record| record.id == task.id) {
                        let previous = registry.records[position].clone();
                        let record = &mut registry.records[position];
                        record.status = match record.status.as_str() {
                            "queued" | "running" => "interrupted".into(),
                            "pausing" => "paused".into(),
                            "cancelling" => "cancelled".into(),
                            _ => record.status.clone(),
                        };
                        record.updated_at_ms = now_ms();
                        if let Err(recovery_error) = self.persist_tasks_locked(&registry).await {
                            registry.records[position] = previous;
                            return Err(format!(
                                "Cannot claim resumed search: {error}; failed to persist interrupted task: {recovery_error}"
                            ));
                        }
                        Some(registry.records[position].clone())
                    } else {
                        None
                    }
                };
                if let Some(recovered) = recovered {
                    self.notify_task(&recovered).await;
                }
                return Err(error);
            }
        }
        self.notify_task(&task).await;
        if let Some(receiver) = restart {
            self.spawn_task_worker(task.id.clone(), receiver);
        }
        Ok(json!({ "task": task_summary(&task), "resource": self.task_resource_descriptor()? }))
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<Value, String> {
        self.control_task(task_id, "cancel").await
    }

    async fn control_task(&self, task_id: &str, operation: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        let task = {
            let mut registry = self.tasks.lock().await;
            let position = registry
                .records
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| format!("Unknown task '{task_id}'"))?;
            let signal = registry.signals.get(task_id).cloned();
            let previous = registry.records[position].clone();
            let task = &mut registry.records[position];
            if matches!(
                task.status.as_str(),
                "completed" | "failed" | "cancelled" | "cancelling" | "recoveryRequired"
            ) {
                return Err(format!(
                    "Task cannot {operation} from status '{}'",
                    task.status
                ));
            }
            match operation {
                "pause" => {
                    if signal.is_some() {
                        if task.status != "paused" {
                            task.status = "pausing".into();
                        }
                    } else {
                        // A queued or interrupted task has no running worker to
                        // acknowledge pausing. Persist the terminal pause now.
                        task.status = "paused".into();
                    }
                }
                "cancel" => {
                    task.status = if signal.is_some() {
                        "cancelling".into()
                    } else {
                        "cancelled".into()
                    };
                }
                _ => unreachable!(),
            }
            task.updated_at_ms = now_ms();
            if let Err(error) = self.persist_tasks_locked(&registry).await {
                registry.records[position] = previous;
                return Err(error);
            }
            // Only signal the worker once its control request has durable
            // ownership. A failed write cannot pause or cancel a running task.
            if let Some(sender) = signal {
                match operation {
                    "pause" => sender.send_modify(|state| state.paused = true),
                    "cancel" => sender.send_modify(|state| state.cancelled = true),
                    _ => unreachable!(),
                }
            }
            registry.records[position].clone()
        };
        self.notify_task(&task).await;
        Ok(json!({ "task": task_summary(&task), "resource": self.task_resource_descriptor()? }))
    }
}
