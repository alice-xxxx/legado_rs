//! 文件只复制到最终缓存一次；格式处理、登记书架和进度通知共用同一条流程。
use super::ApplicationService;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use crate::file_access::picker_display_name;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};

impl ApplicationService {
    pub(crate) async fn import_book_file(
        &self,
        file: std::fs::File,
        name: PathBuf,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<String, String> {
        let _operation = self.operation_read().await;
        let filename = name
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let progress_name = filename.clone();
        let service = self.clone();
        let report: crate::local_books::ImportProgress = Arc::new(
            move |stage, completed, total| {
                service.import_progress(json!({"file":progress_name, "stage":stage, "completed":completed, "total":total}));
            },
        );
        report("read", 0, 0);
        let result = async {
            let book_id =
                crate::local_books::import_local_book(&self.store, file, name, &options, report)
                    .await?;
            let database = self.store.database().clone();
            let id = book_id.clone();
            database.shelve_import(id).map_err(|e|e.to_string())?.await.map_err(|e|e.to_string())?;
            Ok::<_, String>(book_id)
        }
        .await;
        match &result {
            Ok(id) => self.import_progress(json!({"file":filename,"stage":"done", "bookId":id})),
            Err(error) => {
                self.import_progress(json!({"file":filename,"stage":"error", "error":error}))
            }
        }
        result
    }

    fn import_progress(&self, value: Value) {
        #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
        if let Some(app) = &self.app {
            use tauri::Emitter;
            if value["stage"] == "done" {
                let _ = app.emit("shelf-updated", ());
            }
            let _ = app.emit("book-import", value);
        }
        #[cfg(not(any(feature = "desktop", feature = "mobile-runtime")))]
        let _ = value;
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn import_book(
    format: Option<String>,
    options: crate::local_books::LocalImportOptions,
    service: tauri::State<'_, ApplicationService>,
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
        service.import_selected_book(&app, file, options).await?;
    }
    Ok(())
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl ApplicationService {
    pub async fn import_selected_book<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        selected: tauri_plugin_dialog::FilePath,
        options: crate::local_books::LocalImportOptions,
    ) -> Result<(), String> {
        use tauri_plugin_fs::FsExt;
        #[cfg(target_os = "ios")]
        let _scope = picker::SecurityScopedFileAccess {
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
        self.import_book_file(file, name.into(), options)
            .await
            .map(|_| ())
    }
}
