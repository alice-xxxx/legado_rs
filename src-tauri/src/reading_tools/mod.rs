//! 书签、阅读统计、展示替换和书架整理的 JSON 业务入口。
//!
//! 各职责由子模块实现，并在此重新导出，维持 `crate::reading_tools::...` 路径。

mod common;
mod history;
mod replacements;
mod shelf;

pub use history::*;
pub use replacements::*;
pub use shelf::*;
