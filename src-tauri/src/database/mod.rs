//! 对外只提供类型化业务 API；db 是唯一数据库执行入口。
use crate::resources::ResourceError;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
mod activity;
mod catalog;
mod db;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) mod fe_db;
mod files;
mod library;
mod types;
pub(crate) use activity::{storage, Mutation};
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) use db::reader;
pub(crate) use db::Pending;
use db::{params, Session, WriteMode};
pub(crate) use files::verify_snapshot;
pub(crate) use types::*;
pub(crate) const FILE_NAME: &str = "library.sqlite3";
type Result<T> = std::result::Result<T, ResourceError>;

#[derive(Clone)]
pub(crate) struct Database {
    root: Arc<PathBuf>,
}
impl Database {
    pub(crate) fn new(root: &Path) -> Result<Self> {
        let database = Self {
            root: Arc::new(root.to_owned()),
        };
        db::write(&database, WriteMode::Transaction, |_| Ok(()))?.wait()?;
        Ok(database)
    }
    fn file_path(&self) -> PathBuf {
        self.root.join(FILE_NAME)
    }
}
pub(crate) fn online_book_id(source: &str, kind: &str, url: &str) -> String {
    format!(
        "cache-{:016x}",
        crate::support::stable_hash(&format!("{source}\0{kind}\0{url}"))
    )
}
