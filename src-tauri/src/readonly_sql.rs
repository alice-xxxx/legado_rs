//! The SQL plugin receives a pre-opened, read-only pool. JavaScript cannot load a writer.
use std::path::Path;
use tauri::Manager;
pub(crate) const ALIAS: &str = "sqlite:library-reader";
pub(crate) async fn open(app: &tauri::AppHandle, path: &Path) -> Result<(), String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await
        .map_err(|e| e.to_string())?;
    app.state::<tauri_plugin_sql::DbInstances>()
        .0
        .write()
        .await
        .insert(ALIAS.into(), tauri_plugin_sql::DbPool::Sqlite(pool));
    Ok(())
}
pub(crate) async fn close(app: &tauri::AppHandle) {
    let pool = app
        .state::<tauri_plugin_sql::DbInstances>()
        .0
        .write()
        .await
        .remove(ALIAS);
    if let Some(tauri_plugin_sql::DbPool::Sqlite(pool)) = pool {
        pool.close().await;
    }
}
