use std::{path::PathBuf, sync::Arc};

/// App-owned paths and storage; the engine never imports the application's database.
#[derive(Clone)]
pub struct ExecutionContext {
    pub data_dir: PathBuf,
    pub resource_dir: Option<PathBuf>,
    pub storage: Arc<dyn crate::storage::StorageHost>,
    pub cancellation: Option<tokio_util::sync::CancellationToken>,
}
