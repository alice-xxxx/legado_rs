pub mod application;
pub mod backup;
pub(crate) mod catalog;
pub mod discovery;
mod image_ops_host;
pub mod local_books;
pub mod models;
pub mod reading_tools;
pub mod resources;
pub mod rss;
pub(crate) mod search_history;
pub mod source_engine;
mod source_host_ffi;
mod source_http;
mod source_jni;
pub(crate) mod source_metadata;
mod source_storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub fn run() {
    use tauri::{Emitter, Manager};

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_source_engine::init())
        .setup(|app| {
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
            app.manage(service);
            Ok(())
        });
    builder
        .invoke_handler(tauri::generate_handler![
            application::app_bootstrap,
            application::list_sources,
            application::get_search_history,
            application::delete_search_history,
            application::clear_search_history,
            application::import_sources,
            application::import_sources_from_picker,
            application::remove_sources,
            application::update_source,
            application::search_books,
            application::start_search,
            application::search_book_source_candidates,
            application::tasks_resource,
            application::start_chapter_download,
            application::refresh_chapters,
            application::check_new_chapters,
            application::pause_task,
            application::resume_task,
            application::cancel_task,
            application::add_book,
            application::change_book_source,
            application::import_book_from_picker,
            application::import_protected_pdf,
            application::cancel_pending_pdf_import,
            application::get_book,
            application::prepare_chapters,
            application::remove_book,
            application::save_progress,
            application::save_settings,
            application::get_home_config,
            application::save_home_config,
            application::get_rss_state,
            application::set_rss_filter,
            application::set_rss_article_state,
            application::unsubscribe_rss,
            application::get_txt_toc_rules,
            application::upsert_txt_toc_rule,
            application::delete_txt_toc_rule,
            application::list_bookmarks,
            application::upsert_bookmark,
            application::delete_bookmark,
            application::reading_history_resource,
            application::record_reading_session,
            application::delete_reading_history_for_book,
            application::clear_reading_history,
            application::list_replacement_rules,
            application::upsert_replacement_rule,
            application::delete_replacement_rule,
            application::set_book_groups,
            application::create_shelf_group,
            application::rename_shelf_group,
            application::delete_shelf_group,
            application::set_shelf_sort,
            application::set_shelf_order,
            application::create_backup_from_picker,
            application::restore_backup_from_picker,
            application::list_discovery_categories,
            application::list_discovery_books,
            application::list_discovery_favorites,
            application::set_discovery_favorite,
            application::list_rss_categories,
            application::list_rss_articles,
            application::open_rss_article,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
