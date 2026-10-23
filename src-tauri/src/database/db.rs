//! 数据库底层入口；仅供 database 内部的业务 API 调用。
use super::{Database, ResourceError, Result};
pub(super) use rusqlite::params;
use rusqlite::{Connection, OpenFlags, Params, Row};
use std::{
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    sync::OnceLock,
    task::{Context, Poll},
    thread,
};
use tokio::sync::{mpsc, oneshot};

type Job = Box<dyn FnOnce(&mut Option<(Arc<PathBuf>, Connection)>) + Send>;
static WRITER: OnceLock<Result<mpsc::Sender<Job>>> = OnceLock::new();

/// 只暴露参数化读写；连接、语句和事务不能离开本文件。
pub(super) struct Session<'a>(Option<&'a Connection>);

pub(super) enum WriteMode {
    /// 普通修改：事务内读写，成功提交，失败回滚。
    Transaction,
    /// 备份等不能放入事务的操作，仍在唯一写线程执行。
    Connection,
    /// 恢复前关闭写连接；下一次数据库操作自动重连。
    Closed,
}

impl Session<'_> {
    pub(super) fn read<T, P: Params>(
        &self,
        sql: &str,
        params: P,
        map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let connection = self
            .0
            .ok_or_else(|| ResourceError::new("当前维护操作已关闭数据库连接"))?;
        let mut statement = connection.prepare_cached(sql)?;
        let rows = statement.query_map(params, map)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(super) fn write<P: Params>(&self, sql: &str, params: P) -> Result<usize> {
        Ok(self
            .0
            .ok_or_else(|| ResourceError::new("当前维护操作已关闭数据库连接"))?
            .execute(sql, params)?)
    }
}

/// 一次调用内的所有查询共享只读快照。
pub(super) fn read<T>(
    database: &Database,
    action: impl FnOnce(&Session<'_>) -> Result<T>,
) -> Result<T> {
    let mut connection = Connection::open_with_flags(
        database.file_path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let snapshot = connection.transaction()?;
    action(&Session(Some(&snapshot)))
}

/// 同一队列执行普通事务、事务外备份和关闭连接后的恢复，结束后通知结果。
pub(super) fn write<T: Send + 'static>(
    database: &Database,
    mode: WriteMode,
    action: impl FnOnce(&Session<'_>) -> Result<T> + Send + 'static,
) -> Result<Pending<T>> {
    let root = database.root.clone();
    let writer = WRITER
        .get_or_init(|| {
            let (writes, mut requests) = mpsc::channel::<Job>(128);
            thread::Builder::new().spawn(move || {
                let mut connection = None;
                while let Some(job) = requests.blocking_recv() {
                    job(&mut connection);
                }
            })?;
            Ok(writes)
        })
        .as_ref()
        .map_err(|error| ResourceError::new(error.to_string()))?;
    let (completed, notification) = oneshot::channel();
    writer
        .try_send(Box::new(move |connection| {
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<T> {
                if matches!(mode, WriteMode::Closed) {
                    connection.take();
                    return action(&Session(None));
                }
                if connection.as_ref().map(|(path, _)| path.as_ref()) != Some(root.as_ref()) {
                    connection.take();
                    std::fs::create_dir_all(root.as_ref())?;
                    let mut opened = Connection::open(root.join(super::FILE_NAME))?;
                    opened.pragma_update(None, "journal_mode", "WAL")?;
                    opened.pragma_update(None, "foreign_keys", true)?;
                    match opened
                        .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))?
                    {
                        0 => {
                            let schema = opened.transaction()?;
                            schema.execute_batch(include_str!("schema.sql"))?;
                            schema.commit()?;
                        }
                        16 => (),
                        _ => return Err(ResourceError::new("数据库版本不匹配")),
                    }
                    *connection = Some((root, opened));
                }
                let connection = &mut connection.as_mut().unwrap().1;
                if matches!(mode, WriteMode::Connection) {
                    return action(&Session(Some(connection)));
                }
                let transaction = connection.transaction()?;
                let value = action(&Session(Some(&transaction)))?;
                transaction.commit()?;
                Ok(value)
            }))
            .unwrap_or_else(|_| Err(ResourceError::new("数据库操作异常")));
            let _ = completed.send(result);
        }))
        .map_err(|error| ResourceError::new(error.to_string()))?;
    Ok(Pending(notification))
}

impl From<rusqlite::Error> for ResourceError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(format!("数据库操作失败：{error}"))
    }
}

/// 可等待的完成通知；丢弃通知不会取消已经排队的工作。
pub(crate) struct Pending<T>(oneshot::Receiver<Result<T>>);
impl<T> Future for Pending<T> {
    type Output = Result<T>;
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0)
            .poll(context)
            .map(|result| result.unwrap_or_else(|_| Err(ResourceError::new("数据库写线程已停止"))))
    }
}
impl<T> Pending<T> {
    pub(crate) fn wait(self) -> Result<T> {
        futures_executor::block_on(self)
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) mod reader {
    use tauri::Manager;
    pub(crate) async fn open(app: &tauri::AppHandle) -> Result<(), String> {
        let path = crate::app_data_root(app)
            .map_err(|error| error.to_string())?
            .join(super::super::FILE_NAME);
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(options)
            .await
            .map_err(|error| error.to_string())?;
        app.state::<tauri_plugin_sql::DbInstances>()
            .0
            .write()
            .await
            .insert(
                "sqlite:library-reader".into(),
                tauri_plugin_sql::DbPool::Sqlite(pool),
            );
        Ok(())
    }
    pub(crate) async fn close(app: &tauri::AppHandle) {
        let entry = app
            .state::<tauri_plugin_sql::DbInstances>()
            .0
            .write()
            .await
            .remove("sqlite:library-reader");
        if let Some(tauri_plugin_sql::DbPool::Sqlite(pool)) = entry {
            pool.close().await;
        }
    }
}

impl rusqlite::types::FromSql for crate::resources::ResourceRef {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        Self::new(value.as_str()?)
            .map_err(|error| rusqlite::types::FromSqlError::Other(Box::new(error)))
    }
}
