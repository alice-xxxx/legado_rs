//! 缓存使用公共文件存储，入口限定在 cache/ 下。
pub(crate) use super::file::Files as Cache;
use crate::resources::{ResourceError, ResourceRef};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde_json::{json, Value};

impl Cache {
    pub(crate) fn reference(
        kind: &str,
        owner: &str,
        name: &str,
    ) -> Result<ResourceRef, ResourceError> {
        ResourceRef::new(format!("resource://cache/{kind}/{owner}/{name}"))
    }
    pub(crate) async fn save_asset(
        &self,
        kind: &str,
        owner: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<ResourceRef, ResourceError> {
        let reference = Self::reference(kind, owner, name)?;
        self.write_file(Self::name(&reference)?, bytes)
            .await?;
        Ok(reference)
    }
    pub(crate) async fn read_content(
        &self,
        reference: &ResourceRef,
    ) -> Result<serde_json::Value, ResourceError> {
        let name = Self::name(reference)?;
        let bytes = self.read_file(name).await?;
        let text =
            String::from_utf8(bytes).map_err(|error| ResourceError::new(error.to_string()))?;
        Ok(if name.ends_with(".html") {
            serde_json::json!({"kind":"richText","markup":text})
        } else {
            serde_json::json!({"kind":"text","text":text})
        })
    }
    pub(crate) async fn save_content(
        &self,
        reference: &ResourceRef,
        body: &str,
    ) -> Result<(), ResourceError> {
        Ok(self
            .write_file(Self::name(reference)?, body.as_bytes())
            .await?)
    }
    pub(crate) async fn contains(&self, reference: &ResourceRef) -> Result<bool, ResourceError> {
        Ok(self.exists(Self::name(reference)?).await?)
    }
    pub(crate) async fn remove_asset(
        &self,
        reference: &ResourceRef,
    ) -> Result<bool, ResourceError> {
        Ok(self.remove(Self::name(reference)?).await?)
    }
    fn name(reference: &ResourceRef) -> Result<&str, ResourceError> {
        reference
            .path()
            .strip_prefix("cache/")
            .ok_or_else(|| ResourceError::new("引用不属于缓存"))
    }
}
pub(super) fn routes(cache: Cache, base: String) -> Router {
    Router::new()
        .route("/{*path}", get(read))
        .with_state((cache, base))
}

async fn read(
    State((cache, base)): State<(Cache, String)>,
    Path(name): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    cache.read(&name, &method, &headers, &base).await
}

impl crate::resources::Resources {
    pub(crate) fn asset_in_use(&self, reference: &ResourceRef) -> Result<bool, ResourceError> {
        Ok(self.database.asset_referenced(reference.as_str())?
            || crate::resources::protected_resources(&self.root)?.contains(&reference.to_string()))
    }

    pub(crate) async fn clear_cache(&self, book: &str) -> Result<Value, String> {
        crate::support::validate_id(book, "bookId")?;
        let kind = self
            .database
            .book_type(book)
            .map_err(|e| e.to_string())?
            .ok_or("书籍不存在")?;
        let directory = self.root.join("cache").join(&kind).join(book);
        crate::resources::check_path_no_symlink(&self.root, &directory, true)
            .map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        if directory.exists() {
            for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if [
                    "original.",
                    "user-cover-",
                    "user-book-cover-",
                    "cover-",
                    "cover.",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix))
                {
                    continue;
                }
                if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    return Err("缓存含非普通文件".into());
                }
                paths.push(format!("cache/{kind}/{book}/{name}"));
            }
        }
        let protected =
            crate::resources::protected_resources(&self.root).map_err(|e| e.to_string())?;
        let cleared = self
            .database
            .clear_chapter_cache(book.to_owned(), paths)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        let cleanup = self
            .database
            .recover_cleanup(protected)
            .map_err(|e| e.to_string())?
            .await;
        Ok(json!({"clearedCount":cleared,"warning":cleanup.err().map(|e|e.to_string())}))
    }

    pub(crate) async fn cache_usage(&self) -> Result<Value, String> {
        let mut count = [0u64; 2];
        let mut bytes = [0u64; 2];
        for kind in crate::resources::CONTENT_TYPES {
            let directory = self.root.join("cache").join(kind);
            if !directory.exists() {
                continue;
            }
            for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().into_owned();
                let Some(book) = self.database.book(&id).map_err(|e| e.to_string())? else {
                    continue;
                };
                let index = usize::from(book.source_id.is_none());
                for file in std::fs::read_dir(entry.path()).map_err(|e| e.to_string())? {
                    let file = file.map_err(|e| e.to_string())?;
                    let path = file.path();
                    crate::resources::check_path_no_symlink(&self.root, &path, false)
                        .map_err(|e| e.to_string())?;
                    let metadata = file.metadata().map_err(|e| e.to_string())?;
                    if metadata.is_file() {
                        count[index] += 1;
                        bytes[index] += metadata.len();
                    }
                }
            }
        }
        Ok(
            json!({"chapterResourceCount":count[0]+count[1],"chapterResourceBytes":bytes[0]+bytes[1],"onlineChapterResourceCount":count[0],"onlineChapterResourceBytes":bytes[0],"localChapterResourceCount":count[1],"localChapterResourceBytes":bytes[1]}),
        )
    }
}
