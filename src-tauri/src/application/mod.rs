//! 按职责组织应用入口、持久化和平台桥接。

// 应用层只编排业务入口；引擎、存储、文件和配置由对应模块提供。
mod import;
mod login;
mod main_text;
mod more_information;
#[cfg(test)]
#[path = "tests/performance_tests.rs"]
mod performance_tests;
mod search;
pub(crate) mod service;
mod tts;

pub use search::*;
pub(crate) use service::RestoreAdmissionGuard;
pub use service::{ApplicationService, OperationReadGuard};

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use more_information::*;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use main_text::*;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use login::*;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use tts::*;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub use import::*;

#[cfg(test)]
#[path = "tests/reading_tests.rs"]
mod reading_tests;

#[cfg(test)]
#[path = "tests/settings_tests.rs"]
mod settings_tests;
