//! 阅读进度与相邻章节预读；正文请求统一调用 main_text。

#[cfg(test)]
use crate::support::validate_id;
use crate::{main_text::MainText, settings::preload_count};
use serde_json::{json, Value};

impl MainText {
    pub(crate) async fn prepare_reading_chapters(
        &self,
        database: &crate::database::Database,
        resources: &crate::resources::Resources,
        book_id: &str,
        chapter_index: usize,
        epoch: u64,
        updated: impl Fn(Value),
    ) -> Result<(), String> {
        let _operation = self.backup.operation_read().await;
        let settings = resources.settings.read().await.ok();
        let count = preload_count(settings);
        let book = crate::book_files::read_book(database, &book_id)
            .await
            .map_err(|error| error.to_string())?;
        let end = chapter_index
            .saturating_add(count + 1)
            .min(book["chapters"].as_array().map_or(0, Vec::len));
        for index in chapter_index..end {
            if self
                .backup
                .restore_epoch
                .load(std::sync::atomic::Ordering::Acquire)
                != epoch
            {
                break;
            }
            let current = crate::book_files::read_book(database, book_id)
                .await
                .map_err(|error| error.to_string())?;
            if current["progress"]["chapterIndex"].as_u64() != Some(chapter_index as u64) {
                break;
            }
            if current["chapters"][index]["volume"] == true {
                continue;
            }
            if crate::book_files::indexed_chapter_ready(resources, &current, index).await? {
                continue;
            }
            let chapter_id = current["chapters"][index]["id"]
                .as_str()
                .ok_or("Reading chapter ID is missing")?;
            self.main_text(database, resources, book_id, chapter_id, false)
                .await?;
            updated(json!({"bookId":book_id}));
        }
        Ok(())
    }

    #[cfg(test)]
    pub async fn save_progress(
        &self,
        database: &crate::database::Database,
        book_id: &str,
        progress: Value,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let updates = database
            .save_reading_progress(
                book_id.to_owned(),
                serde_json::from_value(progress).map_err(|error| error.to_string())?,
            )
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        Ok(json!({"bookId":book_id,"readingUpdate":updates}))
    }
}

#[cfg(test)]
#[path = "tests/performance.rs"]
mod performance_tests;
#[cfg(test)]
#[path = "tests/progress.rs"]
mod tests;
