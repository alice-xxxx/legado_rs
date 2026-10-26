//! 独立资源模块：管理分发、文件与代理生命周期，只调用数据库具名 API。
mod io;
mod reference;
pub(crate) mod validation;
pub(crate) use io::settings::protected_resources;
pub use reference::{ResourceError, ResourceRef};
use std::path::{Path, PathBuf};
pub(crate) use validation::check_path_no_symlink;

pub(crate) const CONTENT_TYPES: [&str; 6] = ["text", "image", "audio", "video", "html", "pdf"];

pub struct Resources {
    root: PathBuf,
    database: crate::database::Database,
    pub(crate) cache: io::cache::Cache,
    pub(crate) settings: io::settings::Settings,
    pub(crate) proxy: io::proxy::Proxy,
    pub(crate) tts: io::tts::Tts,
    http: io::LocalHttp,
}

impl Resources {
    pub(crate) async fn open(
        root: impl AsRef<Path>,
        database: crate::database::Database,
    ) -> Result<Self, ResourceError> {
        tokio::fs::create_dir_all(root.as_ref()).await?;
        let root = tokio::fs::canonicalize(root).await?;
        let cache = io::cache::Cache::new(root.join("cache")).await?;
        let settings = io::settings::Settings::new(root.clone()).await?;
        let proxy = io::proxy::Proxy::new()?;
        let tts = io::tts::Tts::default();
        let http = io::start(cache.clone(), settings.clone(), proxy.clone(), tts.clone()).await?;
        let resources = Self {
            root,
            database,
            cache,
            settings,
            proxy,
            tts,
            http,
        };
        resources.reload_proxy().await.map_err(ResourceError::new)?;
        Ok(resources)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn base_url(&self) -> &str {
        &self.http.base_url
    }
    pub fn resume_http(&self) -> Result<(), ResourceError> {
        Ok(self.http.resume()?)
    }
    pub fn url_for(&self, reference: &ResourceRef) -> String {
        if reference.is_local() {
            format!(
                "{}{}",
                self.base_url(),
                validation::percent_encode_path(reference.path())
            )
        } else {
            reference.to_string()
        }
    }
    pub(crate) fn descriptor(&self, reference: &ResourceRef) -> serde_json::Value {
        let mime = validation::mime_type(reference.path());
        let format = if mime.starts_with("text/html") {
            "html"
        } else if mime.starts_with("text/plain") {
            "text"
        } else if mime.starts_with("image/") {
            "image"
        } else if mime == "application/pdf" {
            "pdf"
        } else {
            "binary"
        };
        serde_json::json!({"resourceId":reference.as_str(),"src":self.url_for(reference),"contentType":mime,"format":format})
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn app_bootstrap(
    resources: tauri::State<'_, std::sync::Arc<Resources>>,
) -> Result<serde_json::Value, String> {
    resources.bootstrap().await
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn clear_book_chapter_cache(
    book_id: String,
    resources: tauri::State<'_, std::sync::Arc<Resources>>,
) -> Result<serde_json::Value, String> {
    resources.clear_cache(&book_id).await
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub async fn get_chapter_cache_usage(
    resources: tauri::State<'_, std::sync::Arc<Resources>>,
) -> Result<serde_json::Value, String> {
    resources.cache_usage().await
}
