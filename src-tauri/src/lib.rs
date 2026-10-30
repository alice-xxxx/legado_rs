//! Rust 后端入口：公开业务模块并启动跨平台 Tauri 宿主。

#[path = "books/import.rs"]
pub(crate) mod book_import;
pub(crate) mod database;
mod file_access;
pub(crate) use tauri_plugin_source_engine::http as http_client;
#[path = "books/information.rs"]
pub(crate) mod information;
#[path = "books/main_text.rs"]
pub(crate) mod main_text;
pub(crate) mod search;
pub(crate) mod sources;

#[path = "settings/mod.rs"]
mod settings;
mod support;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
mod updates;
// 备份、书籍和书架。
#[path = "backup/mod.rs"]
pub mod backup;
#[path = "books/book_metadata.rs"]
pub(crate) mod book_metadata;
#[path = "books/local_books.rs"]
pub mod local_books;

// 发现页、来源引擎和 RSS 内容。
#[path = "discovery/mod.rs"]
pub mod discovery;
#[path = "rss/mod.rs"]
pub mod rss;
pub(crate) use tauri_plugin_source_engine::http as source_http;

// 阅读配置、资源服务和持久化模型。
#[path = "books/files.rs"]
pub(crate) mod book_files;
#[path = "books/chapter_resources.rs"]
pub(crate) mod chapter_resources;
#[path = "settings/http_tts_config.rs"]
pub(crate) mod http_tts_config;
pub mod reading_tools;
pub mod resources;

// 启动入口负责模块初始化和进程锁，业务服务不持有基础设施。
#[cfg(any(test, feature = "desktop", feature = "mobile-runtime"))]
async fn open_data(
    root: &std::path::Path,
) -> Result<
    (
        database::Database,
        std::sync::Arc<resources::Resources>,
        backup::state::AppDataProcessLock,
    ),
    String,
> {
    let lock = backup::state::AppDataProcessLock::acquire(root).map_err(|e| e.to_string())?;
    let root = lock.data_root();
    backup::recover_interrupted_restore(root)?;
    let database = database::Database::new(root).map_err(|e| e.to_string())?;
    let resources = std::sync::Arc::new(
        resources::Resources::open(root, database.clone())
            .await
            .map_err(|e| e.to_string())?,
    );
    let staged = file_access::picker_import_directory(root);
    match tokio::fs::remove_dir_all(staged).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot clear stale picker imports: {error}")),
    }
    Ok((database, resources, lock))
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn app_data_root<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<std::path::PathBuf> {
    use tauri::Manager;
    let root = app.path().app_data_dir()?;
    // Android returns the sandbox root. Keep our data and its sibling lock/restore
    // files inside it, rather than trying to write into /data/user/0.
    #[cfg(target_os = "android")]
    let root = root.join("files").join("foliver");
    Ok(root)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub fn run() {
    #[cfg(debug_assertions)]
    use tauri::Listener;
    use tauri::{Emitter, Manager};

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

    let builder = tauri::Builder::default();
    #[cfg(all(
        feature = "desktop",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    let builder = {
        builder.plugin(tauri_plugin_single_instance::init(move |app, args, cwd| {
            let cwd = std::path::Path::new(&cwd);
            for argument in args.iter().skip(1) {
                import_external_file(app, std::path::Path::new(argument), cwd);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
    };
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(tauri_plugin_update_installer::init());
    let builder = builder
        .manage(updates::Updates::default())
        .plugin(tauri_plugin_screen::init())
        .plugin(tauri_plugin_navigation::init())
        .plugin(tauri_plugin_system_media::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_source_engine::init())
        .plugin(tauri_plugin_sql::Builder::default().build())
        .setup(move |app| {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                if let Err(error) = app.deep_link().register_all() {
                    eprintln!("[deep-link-register] {error}");
                }
            }

            #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
            tauri_plugin_source_engine::install_browser_app(app.handle().clone());

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
            let data_dir = app_data_root(app.handle())?;
            let resource_dir = app.path().resource_dir().ok();
            let (database, resources, process_lock) = tauri::async_runtime::block_on(open_data(&data_dir))
                .map_err(std::io::Error::other)?;
            let executor = std::sync::Arc::new(tauri_plugin_source_engine::Client::new(
                app_handle, tauri_plugin_source_engine::EngineContext {
                    storage: database.source_storage(), data_dir: data_dir.join("source-engine"),
                    resource_dir, cancellation: None,
                },
            ));
            let backup = std::sync::Arc::new(backup::state::Operations::new());
            let sources = std::sync::Arc::new(sources::Catalog::new(database.clone()));
            let search = std::sync::Arc::new(search::Search::new(sources.clone(), executor.clone(), backup.clone()));
            let information = std::sync::Arc::new(information::Information::new(sources.clone(), executor.clone(), search.clone()));
            let text = std::sync::Arc::new(main_text::MainText::new(sources.clone(), executor.clone(), backup.clone()));
            let mut login = sources::login::Login::new(sources.clone(), executor.clone(), backup.clone());
            login.app = Some(app.handle().clone());
            let mut tts = reading_tools::tts::Tts::new(executor);
            tts.app = Some(app.handle().clone());
            app.manage(sources);
            app.manage(search);
            app.manage(information);
            app.manage(text);
            app.manage(std::sync::Arc::new(login));
            app.manage(std::sync::Arc::new(tts));
            tauri::async_runtime::block_on(crate::database::reader::open(
                app.handle(),
            )).map_err(std::io::Error::other)?;
            let background_backup = backup.clone();
            let backup_database = database.clone();
            let backup_resources = resources.clone();
            let backup_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    match background_backup.maybe_upload_webdav_backup_due(&backup_database, &backup_resources).await {
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
            app.manage(process_lock);
            app.manage(database);
            app.manage(resources);
            app.manage(backup);

            #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
            for path in &startup_file_args {
                import_external_file(app.handle(), path, &startup_cwd);
            }
            Ok(())
        });
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let builder = builder.on_web_content_process_terminate(|webview| {
        // WebKit can discard the content process while the app is in the background.
        // Recover only on WebKit's actual process-termination event.
        let webview = webview.clone();
        let label = webview.label().to_owned();
        eprintln!("[web-content] process terminated for {label}; reloading");
        tauri::async_runtime::spawn(async move {
            // A fresh frontend has no active reader yet. Restore native chrome
            // before loading it, without waiting for a dead JavaScript context.
            if label == "main" {
                if let Err(error) = tauri_plugin_screen::reset(webview.clone()).await {
                    eprintln!("[web-content] failed to restore screen controls: {error}");
                }
            }
            if let Err(error) = webview.reload() {
                eprintln!("[web-content] failed to reload {label}: {error}");
            }
        });
    });
    let invoke_handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        resources::app_bootstrap,
        updates::get_app_info,
        updates::check_app_update,
        updates::install_app_update,
        updates::restart_after_update,
        reading_tools::tts::start_tts,
        reading_tools::tts::tts_voices,
        reading_tools::tts::stop_tts,
        reading_tools::tts::submit_tts,
        sources::login::login,
        search::search,
        search::stop_search,
        search::release_search_results,
        information::more_information,
        information::cancel_more_information,
        main_text::main_text,
        main_text::cancel_main_text,
        book_import::import_book,
        database::fe_db::fe_db,
        settings::save_settings,
        settings::appearance::pick_book_cover,
        settings::appearance::pick_reader_background_image,
        settings::appearance::discard_book_cover_asset,
        settings::import::prepare_configuration_import,
        resources::clear_book_chapter_cache,
        resources::get_chapter_cache_usage,
        backup::service::create_backup_from_picker,
        backup::service::restore_backup_from_picker,
        backup::service::upload_webdav_backup,
        backup::service::restore_webdav_backup,
        discovery::list_discovery_categories,
        discovery::list_discovery_books,
        rss::list_rss_categories,
        rss::list_rss_articles,
        rss::open_rss_article,
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
            if let tauri::RunEvent::WindowEvent { label, event, .. } = &event {
                #[cfg(any(target_os = "android", target_os = "ios"))]
                let resume_resources = matches!(event, tauri::WindowEvent::Resumed);
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                let resume_resources = matches!(event, tauri::WindowEvent::Focused(true));
                if label == "main" && resume_resources {
                    if let Some(resources) =
                        _app.try_state::<std::sync::Arc<resources::Resources>>()
                    {
                        if let Err(error) = resources.resume_http() {
                            eprintln!("[resource-listener] foreground resume failed: {error}");
                        }
                    }
                }
            }

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
                        import_external_file(_app, path.as_path(), &current_dir);
                    }
                }
            }

            #[cfg(all(feature = "desktop", target_os = "macos"))]
            if let tauri::RunEvent::Opened { urls } = event {
                for url in urls {
                    if let Ok(path) = url.to_file_path() {
                        import_external_file(_app, &path, std::path::Path::new(""));
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
                        import_external_book(_app, tauri_plugin_dialog::FilePath::Url(url));
                    }
                }
            }
        });
}

#[cfg(all(
    feature = "desktop",
    not(any(target_os = "android", target_os = "ios"))
))]
fn import_external_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
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

    import_external_book(app, tauri_plugin_dialog::FilePath::Path(selected_path));
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
fn import_external_book<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    file: tauri_plugin_dialog::FilePath,
) {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = book_import::import_selected_book(
            &app.state::<std::sync::Arc<backup::state::Operations>>(),
            &app.state::<database::Database>(),
            &app.state::<std::sync::Arc<resources::Resources>>(),
            &app,
            file,
            Default::default(),
        )
        .await
        {
            app.dialog()
                .message(error)
                .title("Import failed")
                .show(|_| {});
        }
    });
}
