//! Rust 后端入口：公开业务模块并启动跨平台 Tauri 宿主。

pub mod application;
// 备份、书籍和书架。
#[path = "backup/mod.rs"]
pub mod backup;
#[path = "books/book_commit.rs"]
pub(crate) mod book_commit;
#[path = "books/book_display_metadata.rs"]
pub(crate) mod book_display_metadata;
#[path = "books/book_metadata.rs"]
pub(crate) mod book_metadata;
#[path = "books/catalog.rs"]
pub(crate) mod catalog;
#[path = "books/local_books.rs"]
pub mod local_books;
#[path = "shelf/batch.rs"]
pub mod shelf_batch;

// 发现页、来源引擎和 RSS 内容。
#[path = "discovery/mod.rs"]
pub mod discovery;
#[path = "rss/mod.rs"]
pub mod rss;
#[path = "sources/source_engine.rs"]
pub mod source_engine;
#[path = "sources/source_host_ffi.rs"]
mod source_host_ffi;
#[path = "sources/source_http.rs"]
mod source_http;
#[path = "sources/source_jni.rs"]
mod source_jni;
#[path = "sources/source_metadata.rs"]
pub(crate) mod source_metadata;
#[path = "sources/source_storage.rs"]
mod source_storage;

// 阅读配置、资源服务和持久化模型。
#[path = "settings/http_tts_config.rs"]
pub(crate) mod http_tts_config;
#[path = "resources/image_ops_host.rs"]
mod image_ops_host;
#[path = "models/mod.rs"]
pub mod models;
pub mod reading_tools;
#[path = "resources/transactions.rs"]
pub(crate) mod resource_transactions;
pub mod resources;
#[path = "search/history.rs"]
pub(crate) mod search_history;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub fn run() {
    use tauri::{Emitter, Manager};
    #[cfg(debug_assertions)]
    use tauri::Listener;

    #[cfg(all(
        feature = "desktop",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    let startup_file_args = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    #[cfg(all(
        feature = "desktop",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    let startup_cwd = std::env::current_dir().unwrap_or_default();

    let external_files = application::PendingExternalFileRegistry::new();
    let builder = tauri::Builder::default().manage(external_files.clone());
    #[cfg(all(
        feature = "desktop",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    let builder = {
        let external_files = external_files.clone();
        builder.plugin(tauri_plugin_single_instance::init(move |app, args, cwd| {
            let cwd = std::path::Path::new(&cwd);
            for argument in args.iter().skip(1) {
                queue_external_file(app, &external_files, std::path::Path::new(argument), cwd);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
    };
    let run_external_files = external_files.clone();
    let builder = builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_source_engine::init())
        .setup(move |app| {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                if let Err(error) = app.deep_link().register_all() {
                    eprintln!("[deep-link-register] {error}");
                }
            }

            #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
            source_engine::install_browser_app(app.handle().clone());

            #[cfg(debug_assertions)]
            app.listen("frontend-runtime-error", |event| {
                const MAX_PAYLOAD_CHARS: usize = 16_384;
                let mut chars = event.payload().chars();
                let payload = chars.by_ref().take(MAX_PAYLOAD_CHARS).collect::<String>();
                let truncated = chars.next().is_some();
                if truncated {
                    eprintln!("[frontend-runtime-error] {payload}… [truncated]");
                } else {
                    eprintln!("[frontend-runtime-error] {payload}");
                }
            });

            let app_handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            let source_engine_dir = data_dir.join("source-engine");
            let resource_dir = app.path().resource_dir().ok();
            let executor = std::sync::Arc::new(application::TauriSourceExecutor::new(
                app_handle,
                source_engine_dir,
                resource_dir,
            ));
            let service = tauri::async_runtime::block_on(
                application::ApplicationService::open_with_executor(data_dir, executor),
            )
            .map_err(std::io::Error::other)?;
            let task_app = app.handle().clone();
            service.set_task_notifier(std::sync::Arc::new(move |payload| {
                let _ = task_app.emit("task-updated", payload);
            }));
            let task_recovery_app = app.handle().clone();
            service.set_task_recovery_notifier(std::sync::Arc::new(move |payload| {
                let _ = task_recovery_app.emit("task-recovery-required", payload);
            }));
            let search_app = app.handle().clone();
            service.set_search_snapshot_notifier(std::sync::Arc::new(move |descriptor| {
                let _ = search_app.emit("search-results-updated", descriptor);
            }));
            let background_service = service.clone();
            let backup_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    match background_service.maybe_upload_webdav_backup_due().await {
                        Ok(Some(result)) => {
                            if let Some(resource) = result.get("settings") {
                                let _ = backup_app.emit(
                                    "resource-updated",
                                    serde_json::json!({ "kind": "backupMetadata", "resource": resource }),
                                );
                            }
                        }
                        Ok(None) => {}
                        Err(error) => eprintln!("[automatic-webdav-backup] {error}"),
                    }
                }
            });
            app.manage(service);

            #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
            for path in &startup_file_args {
                queue_external_file(app.handle(), &external_files, path, &startup_cwd);
            }
            Ok(())
        });
    let invoke_handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        application::app_bootstrap,
        application::mutate_http_tts_configs,
        application::request_http_tts_audio,
        application::release_http_tts_audio,
        application::get_search_history,
        application::delete_search_history,
        application::clear_search_history,
        application::get_source_login_state,
        application::get_source_login_form,
        application::mutate_source,
        application::search_books,
        application::debug_search_result,
        application::start_search,
        application::search_book_source_candidates,
        application::tasks_resource,
        application::clear_finished_tasks,
        application::retry_task,
        application::start_book_download,
        application::start_chapter_download,
        application::refresh_chapters,
        application::check_new_chapters,
        application::pause_task,
        application::resume_task,
        application::cancel_task,
        application::add_book,
        application::prepare_search_result_book,
        application::change_book_source,
        application::change_chapter_source,
        application::import_book_from_picker,
        application::list_pending_external_files,
        application::import_external_file,
        application::discard_external_file,
        application::import_protected_pdf,
        application::cancel_pending_pdf_import,
        application::get_book,
        application::refresh_book_info,
        application::update_book_display_metadata,
        application::pick_book_cover,
        application::pick_reader_background_image,
        application::clear_reader_background_image,
        application::discard_book_cover_asset,
        application::prepare_chapters,
        application::refresh_chapter_content,
        application::remove_book,
        application::save_progress,
        application::reset_book_progress,
        application::clear_book_chapter_cache,
        application::get_chapter_cache_usage,
        application::save_settings,
        application::get_home_config,
        application::save_home_config,
        application::get_rss_state,
        application::set_rss_filter,
        application::set_rss_article_state,
        application::unsubscribe_rss,
        application::mutate_txt_toc_rules,
        application::list_bookmarks,
        application::upsert_bookmark,
        application::delete_bookmark,
        application::reading_history_resource,
        application::get_reading_statistics,
        application::record_reading_session,
        application::delete_reading_history_for_book,
        application::clear_reading_history,
        application::mutate_replacement_rules,
        application::set_book_groups,
        shelf_batch::batch_set_book_groups,
        shelf_batch::batch_remove_books,
        application::create_shelf_group,
        application::rename_shelf_group,
        application::delete_shelf_group,
        application::set_shelf_sort,
        application::set_shelf_order,
        application::create_backup_from_picker,
        application::restore_backup_from_picker,
        application::get_webdav_backup_config,
        application::save_webdav_auto_backup_interval,
        application::save_webdav_backup_config,
        application::upload_webdav_backup,
        application::restore_webdav_backup,
        application::list_discovery_categories,
        application::list_discovery_books,
        application::list_discovery_favorites,
        application::set_discovery_favorite,
        application::list_rss_categories,
        application::list_rss_articles,
        application::open_rss_article,
        application::lookup_dictionary,
    ];
    builder
        .invoke_handler(move |invoke| {
            let label = invoke.message.webview_ref().label();
            if label.starts_with("source-browser-") || label.starts_with("source-login-") {
                invoke
                    .resolver
                    .reject("Private source browser views cannot call application commands");
                true
            } else {
                invoke_handler(invoke)
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |_app, event| {
            #[cfg(all(
                feature = "desktop",
                not(any(target_os = "android", target_os = "ios"))
            ))]
            if let tauri::RunEvent::WebviewEvent {
                label,
                event: tauri::WebviewEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }),
                ..
            } = &event
            {
                if label == "main" {
                    let current_dir = std::env::current_dir().unwrap_or_default();
                    let supported_paths = paths
                        .iter()
                        .filter(|path| {
                            path.is_file()
                                && path
                                    .extension()
                                    .and_then(|extension| extension.to_str())
                                    .is_some_and(|extension| {
                                        matches!(
                                            extension.to_ascii_lowercase().as_str(),
                                            "txt" | "epub" | "cbz" | "pdf"
                                        )
                                    })
                        })
                        .collect::<Vec<_>>();
                    if supported_paths.is_empty() {
                        use tauri_plugin_dialog::DialogExt;
                        _app.dialog()
                            .message("支持的文件格式为 TXT、EPUB、CBZ、PDF；单个文件最多 512 MiB。")
                            .title("无法导入文件")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                            .show(|_| {});
                    }
                    for path in supported_paths {
                        queue_external_file(
                            _app,
                            &run_external_files,
                            path.as_path(),
                            &current_dir,
                        );
                    }
                }
            }

            #[cfg(all(feature = "desktop", target_os = "macos"))]
            if let tauri::RunEvent::Opened { urls } = event {
                for url in urls {
                    if let Ok(path) = url.to_file_path() {
                        queue_external_file(
                            _app,
                            &run_external_files,
                            &path,
                            std::path::Path::new(""),
                        );
                    }
                }
            }

            #[cfg(any(target_os = "android", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = event {
                for url in urls {
                    #[cfg(target_os = "android")]
                    let supported_scheme = url.scheme() == "content";
                    #[cfg(target_os = "ios")]
                    let supported_scheme = url.scheme() == "file";
                    if supported_scheme {
                        queue_external_mobile_url(_app, &run_external_files, url);
                    }
                }
            }
        });
}

#[cfg(all(
    feature = "desktop",
    not(any(target_os = "android", target_os = "ios"))
))]
fn queue_external_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    registry: &application::PendingExternalFileRegistry,
    path: &std::path::Path,
    current_dir: &std::path::Path,
) {
    let selected_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };
    if !selected_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "txt" | "epub" | "cbz" | "pdf"
            )
        })
    {
        return;
    }

    let app = app.clone();
    let registry = registry.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        use tauri_plugin_dialog::{DialogExt, FilePath};

        let queued = match application::stage_external_file(
            &app,
            FilePath::Path(selected_path),
        )
        .await
        {
            Ok(file) => registry.enqueue_picker_file(file),
            Err(error) => Err(error),
        };
        match queued {
            Ok(Some(file)) => {
                application::schedule_pending_external_file_cleanup(
                    registry.clone(),
                    file.token.clone(),
                );
                if let Err(error) = app.emit("external-file-opened", file) {
                    eprintln!("[external-file-open] {error}");
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("[external-file-open] {error}");
                app.dialog()
                    .message("无法读取该书籍，或待处理文件已满。请从应用的本地导入入口重新选择文件；单文件最多 512 MiB。")
                    .title("无法打开书籍")
                    .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                    .show(|_| {});
            }
        }
    });
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn queue_external_mobile_url<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    registry: &application::PendingExternalFileRegistry,
    url: tauri::Url,
) {
    use tauri::Emitter;
    use tauri_plugin_dialog::{DialogExt, FilePath};

    let app = app.clone();
    let registry = registry.clone();
    tauri::async_runtime::spawn(async move {
        let file = match application::stage_external_file(&app, FilePath::Url(url)).await {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[external-file-open] {error}");
                app.dialog()
                    .message("无法读取系统提供的书籍。请从应用的本地导入入口重新选择文件并授权访问；单文件最多 512 MiB。")
                    .title("无法打开书籍")
                    .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                    .show(|_| {});
                return;
            }
        };
        match registry.enqueue_picker_file(file) {
            Ok(Some(file)) => {
                application::schedule_pending_external_file_cleanup(
                    registry.clone(),
                    file.token.clone(),
                );
                if let Err(error) = app.emit("external-file-opened", file) {
                    eprintln!("[external-file-open] {error}");
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("[external-file-open] {error}");
                app.dialog()
                    .message("暂时无法接收该书籍。请处理现有导入请求后，从应用的本地导入入口重新选择文件。")
                    .title("无法打开书籍")
                    .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                    .show(|_| {});
            }
        }
    });
}
