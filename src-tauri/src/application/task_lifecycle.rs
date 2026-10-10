//! 后台任务的生命周期功能入口。
use super::*;

#[path = "task_lifecycle/control.rs"]
mod control;
#[path = "task_lifecycle/management.rs"]
mod management;
#[path = "task_lifecycle/registry.rs"]
mod registry;
#[path = "task_lifecycle/start.rs"]
mod start;

use registry::task_summary;
// 应用服务的兄弟模块和 Tauri 命令需要复用任务摘要。
