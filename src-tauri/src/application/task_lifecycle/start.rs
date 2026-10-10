//! 后台任务的创建和启动流程。
use super::*;

impl ApplicationService {
    pub async fn start_search(
        &self,
        source_ids: &[String],
        keyword: &str,
        page: u32,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        self.start_search_unlocked(source_ids, keyword, page).await
    }

    // Caller already holds the restore admission and operation read guards.
    pub(super) async fn start_search_unlocked(
        &self,
        source_ids: &[String],
        keyword: &str,
        page: u32,
    ) -> Result<Value, String> {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return Err("Search keyword cannot be empty".into());
        }
        let page = page.max(1);
        let selected = self
            .read_sources()
            .await?
            .into_iter()
            .filter(|source| {
                source.enabled
                    && !crate::source_metadata::is_rss_source_metadata(&source.source)
                    && (source_ids.is_empty() || source_ids.contains(&source.id))
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("No enabled book sources are selected".into());
        }

        let search_id = format!("search-{}", uuid::Uuid::new_v4().simple());
        let task_id = format!("task-{}", uuid::Uuid::new_v4().simple());
        crate::search_history::record_search(&self.store, &search_id, keyword, now_ms()).await?;

        let now = now_ms();
        let task = AppTask {
            id: task_id.clone(),
            kind: "search".into(),
            status: "queued".into(),
            book_id: None,
            chapter_ids: None,
            source_ids: Some(selected.iter().map(|source| source.id.clone()).collect()),
            keyword: Some(keyword.to_owned()),
            page,
            from_index: 0,
            total: selected.len(),
            completed: 0,
            check_only: false,
            search_id: Some(search_id),
            result: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let receiver = self.create_task(task).await?;
        self.spawn_task_worker(task_id.clone(), receiver);
        Ok(json!({
            "taskId": task_id.clone(),
            "task": self.task_summary_by_id(&task_id).await?,
            "resource": self.task_resource_descriptor()?,
        }))
    }
    pub async fn start_book_download(&self, book_id: &str) -> Result<Value, String> {
        self.start_chapter_download(book_id, 0, usize::MAX).await
    }

    pub async fn start_chapter_download(
        &self,
        book_id: &str,
        from_index: usize,
        count: usize,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let book = self
            .store
            .read_json_ref(
                &self
                    .store
                    .book_ref(book_id)
                    .map_err(|error| error.to_string())?,
            )
            .await
            .map_err(|error| error.to_string())?;
        let chapters = book
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
        if chapters.is_empty() {
            return Err("Book chapter directory is empty".into());
        }
        if from_index >= chapters.len() {
            return Err("Download starts outside the book chapter list".into());
        }
        if count == 0 {
            return Err("Download count must be greater than zero".into());
        }
        let total = count.min(chapters.len() - from_index);
        let chapter_ids = chapters
            .iter()
            .skip(from_index)
            .take(total)
            .map(|chapter| {
                chapter["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        "Book chapter directory contains a missing chapter ID".to_owned()
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.start_chapter_download_ids_unlocked(book_id, from_index, chapter_ids, None)
            .await
    }

    pub(super) async fn start_chapter_download_ids_unlocked(
        &self,
        book_id: &str,
        from_index: usize,
        chapter_ids: Vec<String>,
        warning: Option<String>,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        if chapter_ids.is_empty() {
            return Err("Download count must be greater than zero".into());
        }
        let unique_chapter_ids = chapter_ids.iter().collect::<HashSet<_>>();
        if unique_chapter_ids.len() != chapter_ids.len() {
            return Err("Book chapter directory contains duplicate chapter IDs".into());
        }
        let now = now_ms();
        let retry_warning = warning.clone();
        let task = AppTask {
            id: format!("task-{}", uuid::Uuid::new_v4().simple()),
            kind: "chapterDownload".into(),
            status: "queued".into(),
            book_id: Some(book_id.to_owned()),
            chapter_ids: Some(chapter_ids.clone()),
            source_ids: None,
            keyword: None,
            page: 1,
            from_index,
            total: chapter_ids.len(),
            completed: 0,
            check_only: false,
            search_id: None,
            result: warning.map(|warning| json!({ "warning": warning })),
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let (mut task, receiver, reused) = self.create_download_task(task).await?;
        if reused {
            if let Some(warning) = retry_warning {
                task = self
                    .update_task(&task.id, move |current| {
                        current.result = Some(json!({ "warning": warning }));
                    })
                    .await?;
            }
        }
        if let Some(receiver) = receiver {
            self.spawn_task_worker(task.id.clone(), receiver);
        }
        Ok(json!({
            "taskId": task.id.clone(),
            "task": task_summary(&task),
            "resource": self.task_resource_descriptor()?,
            "reused": reused,
        }))
    }

    pub async fn start_catalog_refresh(
        &self,
        book_id: &str,
        check_only: bool,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        self.start_catalog_refresh_unlocked(book_id, check_only).await
    }

    pub(super) async fn start_catalog_refresh_unlocked(
        &self,
        book_id: &str,
        check_only: bool,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let _ = self
            .read_private_json(Path::new("books").join(format!("{book_id}.json")))
            .await?;
        let now = now_ms();
        let task = AppTask {
            id: format!("task-{}", uuid::Uuid::new_v4().simple()),
            kind: if check_only {
                "checkNewChapters"
            } else {
                "refreshChapters"
            }
            .into(),
            status: "queued".into(),
            book_id: Some(book_id.to_owned()),
            chapter_ids: None,
            source_ids: None,
            keyword: None,
            page: 1,
            from_index: 0,
            total: 1,
            completed: 0,
            check_only,
            search_id: None,
            result: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let (task, receiver, reused) = self.create_catalog_task(task).await?;
        if let Some(receiver) = receiver {
            self.spawn_task_worker(task.id.clone(), receiver);
        }
        Ok(json!({
            "taskId": task.id.clone(),
            "task": task_summary(&task),
            "resource": self.task_resource_descriptor()?,
            "reused": reused,
        }))
    }
}
