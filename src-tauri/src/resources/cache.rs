//! 章节缓存的文件枚举、统计与清理；缓存列更新由数据库事务完成。
use super::ResourceStore;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use super::{appearance::READER_BACKGROUND_BOOK_ID, ResourceRef};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use crate::application::ApplicationService;
use crate::support::validate_id;
use serde_json::{json, Value};

pub(crate) async fn clear_cache(store: &ResourceStore, book_id: &str) -> Result<Value, String> {
    validate_id(book_id, "bookId")?;
    let guard = store
        .transaction_writer_guard()
        .await
        .map_err(|e| e.to_string())?;
    let directory = store.root().join("cache").join(
        store.book_type(book_id).map_err(|e| e.to_string())?,
    ).join(book_id);
    let database = store.database().clone();
    let root = store.root().to_owned();
    let id = book_id.to_owned();
    let protected=crate::settings::protected_resources(store.root()).map_err(|error|error.to_string())?;
    let (count, cleanup) = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        crate::resources::check_path_no_symlink(&root, &directory, true)
            .map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        if directory.exists() {
            for entry in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("original.")
                    || name.starts_with("user-cover-")
                    || name.starts_with("user-book-cover-")
                    || name.starts_with("cover-")
                    || name.starts_with("cover.")
                {
                    continue;
                }
                let metadata = entry.file_type().map_err(|e| e.to_string())?;
                if !metadata.is_file() || metadata.is_symlink() {
                    return Err("Refusing to clear a non-regular chapter cache file".into());
                }
                let path = entry
                    .path()
                    .strip_prefix(&root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                paths.push(path);
            }
        }

        let count = database
            .clear_chapter_cache(id, paths).and_then(crate::database::Pending::wait)
            .map_err(|e| e.to_string())?;
        let cleanup = database.recover_cleanup(protected).and_then(crate::database::Pending::wait);
        Ok::<_, String>((count, cleanup))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(json!({"clearedCount":count,"warning":cleanup.err().map(|e|e.to_string())}))
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) async fn cache_usage(store: &ResourceStore) -> Result<Value, String> {
    let _writer_guard = store
        .transaction_writer_guard()
        .await
        .map_err(|error| error.to_string())?;
    let mut chapter_count = 0u64;
    let mut chapter_bytes = 0u64;
    let mut online_chapter_count = 0u64;
    let mut online_chapter_bytes = 0u64;
    let mut local_chapter_count = 0u64;
    let mut local_chapter_bytes = 0u64;
    let root = store.root();
    for kind in crate::resources::CONTENT_TYPES {
        let directory = root.join("cache").join(kind);
        crate::resources::check_path_no_symlink(root, &directory, true)
            .map_err(|error| error.to_string())?;
        if !directory.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            if id == "debug-source"
                || id == "tts-audio"
                || id.starts_with("rss-")
                || id == READER_BACKGROUND_BOOK_ID
            {
                continue;
            }
            let Some(book)=store.database().book(&id).map_err(|e|e.to_string())? else {continue;};
            let online = book.source_id.is_some() && book.book_url.is_some();
            let cover = book.cover_override.as_deref().or(book.cover.as_deref())
                .and_then(|url| ResourceRef::new(url).ok());
            for file in std::fs::read_dir(entry.path()).map_err(|error| error.to_string())? {
                let file = file.map_err(|error| error.to_string())?;
                let name = file.file_name().to_string_lossy().into_owned();
                let path = file.path();
                crate::resources::check_path_no_symlink(root, &path, false)
                    .map_err(|error| error.to_string())?;
                if name.starts_with("original.")
                    || cover
                        .as_ref()
                        .is_some_and(|cover| root.join(cover.path()) == path)
                    || name.starts_with("user-book-cover-")
                {
                    continue;
                }
                let metadata = file.metadata().map_err(|error| error.to_string())?;
                if !metadata.is_file() {
                    continue;
                }
                chapter_count = chapter_count.saturating_add(1);
                chapter_bytes = chapter_bytes.saturating_add(metadata.len());
                if online {
                    online_chapter_count = online_chapter_count.saturating_add(1);
                    online_chapter_bytes = online_chapter_bytes.saturating_add(metadata.len());
                } else {
                    local_chapter_count = local_chapter_count.saturating_add(1);
                    local_chapter_bytes = local_chapter_bytes.saturating_add(metadata.len());
                }
            }
        }
    }

    Ok(json!({
        "chapterResourceCount": chapter_count,
        "chapterResourceBytes": chapter_bytes,
        "onlineChapterResourceCount": online_chapter_count,
        "onlineChapterResourceBytes": online_chapter_bytes,
        "localChapterResourceCount": local_chapter_count,
        "localChapterResourceBytes": local_chapter_bytes,
    }))
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn clear_book_chapter_cache(
    app: tauri::AppHandle,
    book_id: String,
    service: tauri::State<'_, ApplicationService>,
) -> Result<Value, String> {
    use tauri::Emitter;
    let _operation = service.operation_read().await;
    // 取消在途正文后再清理，避免旧响应重新登记缓存地址。
    let book = service
        .store
        .read_book(&book_id)
        .await
        .map_err(|e| e.to_string())?;
    for chapter in book["chapters"].as_array().into_iter().flatten() {
        if let Some(id) = chapter["id"].as_str() {
            service.cancel_main_text(&book_id, id);
        }
    }
    let result = clear_cache(&service.store, &book_id).await?;
    let _ = app.emit("book-updated", json!({"bookId":book_id}));
    Ok(result)
}
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn get_chapter_cache_usage(
    service: tauri::State<'_, ApplicationService>,
) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    cache_usage(&service.store).await
}
