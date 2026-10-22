//! 阅读进度与相邻章节预读；正文请求统一调用 main_text。

use crate::{
    main_text::{ChapterPreparation, MainText},
    settings::preload_count,
};
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
        let db = database.clone();
        let id = book_id.to_owned();
        let chapters = tokio::task::spawn_blocking(move || {
            db.reading_chapter_ids(&id, chapter_index, count + 1)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        for chapter_id in chapters {
            if self
                .backup
                .restore_epoch
                .load(std::sync::atomic::Ordering::Acquire)
                != epoch
            {
                break;
            }
            match self
                .prepare_chapter(
                    database,
                    resources,
                    book_id,
                    &chapter_id,
                    false,
                    Some(chapter_index),
                )
                .await?
            {
                ChapterPreparation::Updated => {
                    updated(json!({"bookId":book_id,"chapterId":chapter_id}))
                }
                ChapterPreparation::Cached => (),
                ChapterPreparation::Stale => break,
            }
        }
        Ok(())
    }
}
