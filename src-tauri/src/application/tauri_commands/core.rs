//! 应用启动与基础状态 Tauri 命令。

use super::*;

#[tauri::command]
pub async fn app_bootstrap(service: State<'_, ApplicationService>) -> Result<Value, String> {
    let _operation = service.operation_read().await;
    service.bootstrap().await
}
