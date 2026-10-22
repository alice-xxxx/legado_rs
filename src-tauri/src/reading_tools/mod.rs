//! 书签、阅读统计和正文展示替换规则。
//!
//! 各职责由子模块实现，并在此重新导出，维持 `crate::reading_tools::...` 路径。

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) mod dictionary;
pub(crate) mod progress;

mod history;
mod replacements;

pub use history::*;
pub use replacements::*;

pub(crate) mod tts;
