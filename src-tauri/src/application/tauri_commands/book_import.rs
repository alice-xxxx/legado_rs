//! 本地书籍与外部文件导入 Tauri 命令。

use super::*;

#[tauri::command]
pub fn list_pending_external_files(
    pending: State<'_, PendingExternalFileRegistry>,
) -> Result<Value, String> {
    Ok(json!({ "files": pending.list()? }))
}

#[tauri::command]
pub async fn import_external_file(
    app: tauri::AppHandle,
    token: String,
    options: crate::local_books::LocalImportOptions,
    pending: State<'_, PendingExternalFileRegistry>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    validate_id(&token, "token")?;
    let Some(file) = pending.claim(&token)? else {
        return Err("This book file request expired; open the file again".to_owned());
    };
    let mut claim = PendingExternalFileImportClaim::new((*pending).clone(), token.clone());
    let result = service
        .import_local_book_with_challenge(file.file.clone(), options)
        .await;
    match result {
        Ok(result) => {
            claim.finish()?;
            if result.get("passwordRequired").and_then(Value::as_bool) != Some(true) {
                let _ = app.emit("book-added", result["book"].clone());
                let _ = app.emit("shelf-updated", result["shelf"].clone());
            }
            Ok(result)
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub fn discard_external_file(
    token: String,
    pending: State<'_, PendingExternalFileRegistry>,
) -> Result<Value, String> {
    validate_id(&token, "token")?;
    pending.discard(&token)?;
    Ok(json!({ "discarded": true }))
}

#[tauri::command]
pub async fn import_book_from_picker(
    app: tauri::AppHandle,
    options: crate::local_books::LocalImportOptions,
    format: Option<String>,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let selection: &[&str] = match format.as_deref() {
        None => &["txt", "epub", "cbz", "pdf"],
        Some("txt") => &["txt"],
        Some("epub") => &["epub"],
        Some("cbz") => &["cbz"],
        Some("pdf") => &["pdf"],
        Some(_) => return Err("Unsupported local book format filter".to_owned()),
    };
    let Some(path) = pick_file(&app, "Books", selection).await? else {
        return Ok(json!({ "cancelled": true }));
    };
    if let Some(expected) = format.as_deref() {
        let actual = path.path.extension().and_then(|suffix| suffix.to_str());
        if !actual.is_some_and(|suffix| suffix.eq_ignore_ascii_case(expected)) {
            return Err("Selected book extension does not match the requested format".to_owned());
        }
    }
    let result = service
        .import_local_book_with_challenge(path, options)
        .await?;
    if result.get("passwordRequired").and_then(Value::as_bool) == Some(true) {
        return Ok(result);
    }
    let _ = app.emit("book-added", result["book"].clone());
    let _ = app.emit("shelf-updated", result["shelf"].clone());
    Ok(result)
}

#[tauri::command]
pub async fn import_protected_pdf(
    app: tauri::AppHandle,
    import_token: String,
    password: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let result = service
        .retry_pending_pdf_import(&import_token, password)
        .await?;
    let _ = app.emit("book-added", result["book"].clone());
    let _ = app.emit("shelf-updated", result["shelf"].clone());
    Ok(result)
}

#[tauri::command]
pub async fn cancel_pending_pdf_import(
    import_token: String,
    service: State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _ = service.cancel_pending_pdf_import(&import_token).await?;
    Ok(json!({ "cancelled": true }))
}
