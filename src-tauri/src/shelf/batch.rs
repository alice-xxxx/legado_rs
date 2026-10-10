//! 将多个书架修改作为一次可恢复的批量变更处理。
//! Atomic batch operations for the JSON-backed bookshelf.

use serde_json::{Value, json};

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use tauri::{Emitter, State};

/// Remove distinct selected books one at a time, preserving typed commit state
/// so callers never have to infer whether a failed delete changed the library.
pub async fn remove_books_batch(
    service: &crate::application::ApplicationService,
    book_ids: Vec<String>,
) -> Result<Value, String> {
    let book_ids = crate::reading_tools::normalize_batch_book_ids(book_ids)?;
    let shelf = service.resource_descriptor(&service.resource_store().shelf_ref());
    remove_books_batch_with_outcomes(book_ids, shelf, |book_id| async move {
        service.remove_book_with_outcome(&book_id).await
    })
    .await
}

async fn remove_books_batch_with_outcomes<F, Fut>(
    book_ids: Vec<String>,
    mut shelf: Value,
    mut remove_book: F,
) -> Result<Value, String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<
            Output = Result<
                crate::application::BookRemovalOutcome,
                crate::application::BookRemovalError,
            >,
        >,
{
    let mut completed_ids = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut not_attempted_ids = Vec::new();
    let mut recovery_required = false;

    // Each service call acquires its own operation guard. Holding one around
    // this loop would re-enter the non-reentrant application gate.
    for (index, book_id) in book_ids.iter().enumerate() {
        match remove_book(book_id.clone()).await {
            Ok(outcome) => {
                completed_ids.push(outcome.book_id);
                shelf = outcome.shelf;
                if let Some(message) = outcome.warning {
                    warnings.push(json!({
                        "bookId": book_id,
                        "outcome": "committed",
                        "recoveryRequired": outcome.recovery_required,
                        "message": message,
                    }));
                }
                if outcome.recovery_required {
                    recovery_required = true;
                    not_attempted_ids.extend(book_ids[index + 1..].iter().cloned());
                    break;
                }
            }
            Err(error) => {
                use crate::resource_transactions::CommitState;

                match error.commit_state {
                    CommitState::NotCommitted => {
                        errors.push(json!({
                            "bookId": book_id,
                            "outcome": "notCommitted",
                            "commitState": "notCommitted",
                            "recoveryRequired": error.recovery_required,
                            "message": error.detail,
                        }));
                        if error.recovery_required {
                            recovery_required = true;
                            not_attempted_ids.extend(book_ids[index + 1..].iter().cloned());
                            break;
                        }
                    }
                    CommitState::Committed => {
                        completed_ids.push(book_id.clone());
                        warnings.push(json!({
                            "bookId": book_id,
                            "outcome": "committed",
                            "commitState": "committed",
                            "recoveryRequired": error.recovery_required,
                            "message": error.detail,
                        }));
                        if error.recovery_required {
                            recovery_required = true;
                            not_attempted_ids.extend(book_ids[index + 1..].iter().cloned());
                            break;
                        }
                    }
                    CommitState::Indeterminate => {
                        errors.push(json!({
                            "bookId": book_id,
                            "outcome": "unknown",
                            "commitState": "indeterminate",
                            "recoveryRequired": true,
                            "message": error.detail,
                        }));
                        recovery_required = true;
                        not_attempted_ids.extend(book_ids[index + 1..].iter().cloned());
                        break;
                    }
                }
            }
        }
    }

    Ok(json!({
        "shelf": shelf,
        "completedIds": completed_ids,
        "errors": errors,
        "warnings": warnings,
        "recoveryRequired": recovery_required,
        "notAttemptedIds": not_attempted_ids,
    }))
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn batch_set_book_groups(
    app: tauri::AppHandle,
    book_ids: Vec<String>,
    groups: Vec<String>,
    service: State<'_, crate::application::ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    let resource =
        crate::reading_tools::set_book_groups_batch(service.resource_store(), book_ids, groups)
            .await?;
    let descriptor = service.resource_descriptor(&resource);
    let _ = app.emit("shelf-updated", descriptor.clone());
    Ok(descriptor)
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn batch_remove_books(
    app: tauri::AppHandle,
    book_ids: Vec<String>,
    service: State<'_, crate::application::ApplicationService>,
) -> Result<Value, String> {
    let result = remove_books_batch(&service, book_ids).await?;
    if result["completedIds"]
        .as_array()
        .is_some_and(|completed_ids| !completed_ids.is_empty())
    {
        let _ = app.emit("shelf-updated", result["shelf"].clone());
    }
    Ok(result)
}
