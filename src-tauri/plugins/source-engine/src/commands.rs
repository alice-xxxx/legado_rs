use tauri::{AppHandle, Runtime, command};

use crate::{Result, SourceEngineCall, SourceEngineExt, SourceEngineResponse};

#[command]
pub(crate) async fn execute<R: Runtime>(
    app: AppHandle<R>,
    call: SourceEngineCall,
) -> Result<SourceEngineResponse> {
    app.source_engine().execute(call)
}
