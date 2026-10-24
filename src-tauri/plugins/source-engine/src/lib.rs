use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

pub use models::*;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod api;
mod browser;
mod context;
mod error;
mod host;
pub mod http;
pub mod http_client;
mod image_ops;
mod jni_bridge;
#[cfg(desktop)]
mod jvm;
mod models;
pub mod source;
pub mod storage;
mod wire;

pub use api::{EngineFuture, SourceExecutor};
pub use context::ExecutionContext;
#[cfg(desktop)]
pub use desktop::JvmEngine;
pub use wire::TextOr;

#[cfg(all(feature = "desktop-runtime", desktop))]
pub use browser::install_app as install_browser_app;

#[cfg(desktop)]
pub use desktop::SourceEngine;
#[cfg(mobile)]
pub use mobile::SourceEngine;

pub use error::{Error, Result};

/// Typed source operations and native login browser controls for every app runtime.
pub trait SourceEngineExt<R: Runtime> {
    fn source_engine(&self) -> &SourceEngine<R>;
}

impl<R: Runtime, T: Manager<R>> SourceEngineExt<R> for T {
    fn source_engine(&self) -> &SourceEngine<R> {
        self.state::<SourceEngine<R>>().inner()
    }
}

/// Registers the platform engine adapter and its application state.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("source-engine")
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
