//! 测试独立组装各模块；不提供业务接口。
use crate::{database::Database, resources::Resources, source_engine::SourceExecutor};
use std::{path::Path, sync::Arc};
#[derive(Clone)]
pub(crate) struct TestApp {
    pub(crate) database: Database,
    pub(crate) resources: Arc<Resources>,
    pub(crate) sources: Arc<crate::source_catalog::SourceCatalog>,
    pub(crate) search: Arc<crate::search::Search>,
    pub(crate) information: Arc<crate::information::Information>,
    pub(crate) text: Arc<crate::main_text::MainText>,
    pub(crate) tts: Arc<crate::reading_tools::tts::Tts>,
    pub(crate) login: Arc<crate::source_login::Login>,
    pub(crate) backup: Arc<crate::backup::state::Operations>,
    _lock: Arc<crate::backup::state::AppDataProcessLock>,
}
impl TestApp {
    pub(crate) async fn open_with_executor(
        root: impl AsRef<Path>,
        executor: Arc<dyn SourceExecutor>,
    ) -> Result<Self, String> {
        let (database, resources, lock) = crate::open_data(root.as_ref()).await?;
        let sources = Arc::new(crate::source_catalog::SourceCatalog::new(database.clone()));
        let backup = Arc::new(crate::backup::state::Operations::new());
        let search = Arc::new(crate::search::Search::new(
            sources.clone(),
            executor.clone(),
            backup.clone(),
        ));
        let information = Arc::new(crate::information::Information::new(
            sources.clone(),
            executor.clone(),
            search.clone(),
        ));
        let text = Arc::new(crate::main_text::MainText::new(
            sources.clone(),
            executor.clone(),
            backup.clone(),
        ));
        let login = Arc::new(crate::source_login::Login::new(
            sources.clone(),
            executor.clone(),
            backup.clone(),
        ));
        let tts = Arc::new(crate::reading_tools::tts::Tts::new(executor));
        Ok(Self {
            database,
            resources,
            sources,
            backup,
            search,
            information,
            text,
            login,
            tts,
            _lock: Arc::new(lock),
        })
    }
}
