//! 文件只复制到最终缓存一次；格式处理、登记书架和进度通知共用同一条流程。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use crate::file_access::picker_display_name;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};

pub(crate) async fn import_book_file(
    backup: &crate::backup::state::Operations,
    database: &crate::database::Database,
    resources: &crate::resources::Resources,
    file: std::fs::File,
    name: PathBuf,
    options: crate::local_books::LocalImportOptions,
    notify: Arc<dyn Fn(Value) + Send + Sync>,
) -> Result<String, String> {
    let _operation = backup.operation_read().await;
    let filename = name
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let progress_name = filename.clone();
    let progress = notify.clone();
    let report: crate::local_books::ImportProgress = Arc::new(move |stage, completed, total| {
        progress(
            json!({"file":progress_name, "stage":stage, "completed":completed, "total":total}),
        );
    });
    report("read", 0, 0);
    let result = async {
        let book_id = crate::local_books::import_local_book(
            database, resources, file, name, &options, report,
        )
        .await?;
        let database = database.clone();
        let id = book_id.clone();
        database
            .shelve_import(id)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(book_id)
    }
    .await;
    match &result {
        Ok(id) => notify(json!({"file":filename,"stage":"done", "bookId":id})),
        Err(error) => notify(json!({"file":filename,"stage":"error", "error":error})),
    }
    result
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn import_book(
    database: tauri::State<'_, crate::database::Database>,
    resources: tauri::State<'_, std::sync::Arc<crate::resources::Resources>>,
    format: Option<String>,
    options: crate::local_books::LocalImportOptions,
    backup: tauri::State<'_, std::sync::Arc<crate::backup::state::Operations>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    let extensions: &[&str] = match format.as_deref() {
        None => &["txt", "epub", "cbz", "pdf"],
        Some("txt") => &["txt"],
        Some("epub") => &["epub"],
        Some("cbz") => &["cbz"],
        Some("pdf") => &["pdf"],
        Some(_) => return Err("Unsupported book format".into()),
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Books", extensions)
        .pick_file(move |file| {
            let _ = sender.send(file);
        });
    if let Some(file) = receiver.await.map_err(|error| error.to_string())? {
        import_selected_book(&backup, &database, &resources, &app, file, options).await?;
    }
    Ok(())
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) async fn import_selected_book<R: tauri::Runtime>(
    backup: &crate::backup::state::Operations,
    database: &crate::database::Database,
    resources: &crate::resources::Resources,
    app: &tauri::AppHandle<R>,
    selected: tauri_plugin_dialog::FilePath,
    options: crate::local_books::LocalImportOptions,
) -> Result<(), String> {
    use tauri_plugin_fs::FsExt;
    #[cfg(target_os = "ios")]
    let _scope = crate::file_access::SecurityScopedFileAccess {
        app,
        selected: selected.clone(),
    };
    let uri = selected.to_string();
    let name = picker_display_name(&uri, None).unwrap_or_else(|| "book".into());
    let mut open = tauri_plugin_fs::OpenOptions::new();
    open.read(true);
    let file = app
        .fs()
        .open(selected, open)
        .map_err(|error| error.to_string())?;
    use tauri::Emitter;
    let emitter = app.clone();
    let notify = Arc::new(move |value: Value| {
        if value["stage"] == "done" {
            let _ = emitter.emit("shelf-updated", ());
        }
        let _ = emitter.emit("book-import", value);
    });
    import_book_file(
        backup,
        database,
        resources,
        file,
        name.into(),
        options,
        notify,
    )
    .await
    .map(|_| ())
}
