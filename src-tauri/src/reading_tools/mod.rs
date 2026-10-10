//! 书签、阅读统计、展示替换和书架整理的 JSON 业务入口。
//!
//! 各职责由子模块实现，并在此重新导出，维持 `crate::reading_tools::...` 路径。

mod bookmarks;
mod common;
mod history;
mod replacements;
mod shelf;

pub use bookmarks::*;
pub use history::*;
pub use replacements::*;
pub use shelf::*;

#[allow(unused_imports)]
pub(crate) use shelf::MAX_BATCH_BOOK_IDS;
pub(crate) use shelf::{apply_shelf_sort, normalize_batch_book_ids};
