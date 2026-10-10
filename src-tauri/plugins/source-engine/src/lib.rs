use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

pub use models::{
    SourceEngineCall, SourceEngineResponse, SourceLoginWebCancelled, SourceLoginWebCapture,
    SourceLoginWebSession, SourceLoginWebStart, SourceLoginWebStarted,
};

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod commands;
mod error;
mod models;

#[cfg(desktop)]
use desktop::SourceEngine;
#[cfg(mobile)]
use mobile::SourceEngine;

pub use error::{Error, Result};

/// Gives the Tauri command a platform-neutral route to the native parser adapter.
pub trait SourceEngineExt<R: Runtime> {
    fn source_engine(&self) -> &SourceEngine<R>;
}

impl<R: Runtime, T: Manager<R>> SourceEngineExt<R> for T {
    fn source_engine(&self) -> &SourceEngine<R> {
        self.state::<SourceEngine<R>>().inner()
    }
}

/// Registers the Kotlin/Native and Kotlin/Android parser entry points on mobile builds.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("source-engine")
        .invoke_handler(tauri::generate_handler![commands::execute])
        .setup(|app, api| {
            #[cfg(mobile)]
            let source_engine = mobile::init(app, api)?;
            #[cfg(desktop)]
            let source_engine = desktop::init(app, api)?;
            app.manage(source_engine);
            Ok(())
        })
        .build()
}
