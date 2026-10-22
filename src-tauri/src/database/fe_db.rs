//! Vue 唯一写入口：只转发业务操作，并在提交后发送通知。
use super::{Mutation, ReadingUpdate};
use serde::Serialize;
use tauri::{ipc::Channel, Emitter, State};

#[derive(Clone, Serialize)]
pub(crate) struct Completion {
    error: Option<String>,
}

#[tauri::command]
pub(crate) async fn fe_db(
    database: tauri::State<'_, crate::database::Database>,
    resources: tauri::State<'_, std::sync::Arc<crate::resources::Resources>>,
    app: tauri::AppHandle,
    backup: State<'_, std::sync::Arc<crate::backup::state::Operations>>,
    sources: State<'_, std::sync::Arc<crate::source_catalog::SourceCatalog>>,
    text: State<'_, std::sync::Arc<crate::main_text::MainText>>,
    operations: Vec<Mutation>,
    completed: Channel<Completion>,
    prepare_current: Option<bool>,
) -> Result<(), String> {
    let backup = backup.inner().clone();
    let sources = sources.inner().clone();
    let text = text.inner().clone();
    let database = database.inner().clone();
    let resources = resources.inner().clone();
    let operation = backup.operation_read().await;
    let sources_changed = operations
        .iter()
        .any(|operation| matches!(operation, Mutation::SaveSources { .. }));
    let publication = if sources_changed {
        Some(sources.publication.clone().lock_owned().await)
    } else {
        None
    };
    let pending = database
        .submit(operations)
        .map_err(|error| error.to_string())?;
    let epoch = backup
        .restore_epoch
        .load(std::sync::atomic::Ordering::Acquire);
    tauri::async_runtime::spawn(async move {
        let result = pending.await;
        if let Ok(updates) = &result {
            if sources_changed {
                sources.invalidate().await;
                let _ = app.emit("sources-updated", ());
            }
            for update in updates {
                let _ = app.emit("reading-progress-updated", update);
            }
        }
        let _ = completed.send(Completion {
            error: result.as_ref().err().map(ToString::to_string),
        });
        drop(publication);
        drop(operation);
        if let Ok(updates) = result {
            for ReadingUpdate {
                book_id,
                progress,
                chapter_changed,
                ..
            } in updates
            {
                if !chapter_changed && !prepare_current.unwrap_or(false) {
                    continue;
                }
                if let Err(error) = text
                    .prepare_reading_chapters(
                        &database,
                        &resources,
                        &book_id,
                        progress.chapter_index as usize,
                        epoch,
                        |update| {
                            let _ = app.emit("book-updated", update);
                        },
                    )
                    .await
                {
                    #[derive(Clone, Serialize)]
                    #[serde(rename_all = "camelCase")]
                    struct ReadingError<'a> {
                        book_id: &'a str,
                        chapter_index: u64,
                        error: String,
                    }
                    let _ = app.emit(
                        "reading-resource-error",
                        ReadingError {
                            book_id: &book_id,
                            chapter_index: progress.chapter_index,
                            error,
                        },
                    );
                }
            }
        }
    });
    Ok(())
}
