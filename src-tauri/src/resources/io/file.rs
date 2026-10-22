//! 普通文件的原子写入、访问边界和按文件协调读写。
use super::super::validation::{check_path_no_symlink, mime_type};
use super::stream;
use axum::{
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::RwLock;

#[derive(Clone)]
pub(crate) struct Files(Arc<Storage>);

struct Storage {
    root: PathBuf,
    locks: Mutex<HashMap<PathBuf, Arc<RwLock<()>>>>,
}

impl Files {
    /// root 是本模块的文件根目录，所有操作使用相对路径。
    pub(crate) async fn new(root: PathBuf) -> io::Result<Self> {
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self(Arc::new(Storage {
            root: tokio::fs::canonicalize(root).await?,
            locks: Mutex::new(HashMap::new()),
        })))
    }

    pub(crate) async fn write_file(&self, path: &str, bytes: &[u8]) -> io::Result<()> {
        let bytes = bytes.to_owned();
        self.replace(path, move |file| file.write_all(&bytes)).await
    }

    /// 大文件直接复制，不把整本原件装入内存。
    pub(crate) async fn copy_file<F>(
        &self,
        path: &str,
        mut source: std::fs::File,
        progress: F,
    ) -> io::Result<()>
    where
        F: Fn(u64, u64) + Send + 'static,
    {
        let total = source.metadata()?.len();
        self.replace(path, move |file| {
            use std::io::Read;
            let mut buffer = [0; 64 * 1024];
            let mut copied = 0;
            progress(0, total);
            loop {
                let count = source.read(&mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                file.write_all(&buffer[..count])?;
                copied += count as u64;
                if copied % (1024 * 1024) < count as u64 || copied == total {
                    progress(copied, total);
                }
            }
        })
        .await
    }

    async fn replace<F>(&self, name: &str, write: F) -> io::Result<()>
    where
        F: FnOnce(&mut std::fs::File) -> io::Result<()> + Send + 'static,
    {
        let path = self.path(name)?;
        let writing = self.lock(&path).write_owned().await;
        tokio::task::spawn_blocking(move || {
            // 等待方取消后，实际写入继续持有锁，直到原子替换完成。
            let _writing = writing;
            std::fs::create_dir_all(path.parent().unwrap())?;
            atomicwrites::AtomicFile::new(&path, atomicwrites::AllowOverwrite)
                .write(|file| {
                    write(file)?;
                    file.sync_all()
                })
                .map_err(io::Error::other)
        })
        .await
        .map_err(io::Error::other)?
    }

    pub(crate) async fn read_file(&self, name: &str) -> io::Result<Vec<u8>> {
        let path = self.path(name)?;
        let _reading = self.lock(&path).read_owned().await;
        tokio::fs::read(path).await
    }

    pub(crate) async fn exists(&self, name: &str) -> io::Result<bool> {
        let path = self.path(name)?;
        let _reading = self.lock(&path).read_owned().await;
        match tokio::fs::metadata(path).await {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn remove(&self, name: &str) -> io::Result<bool> {
        let path = self.path(name)?;
        let _writing = self.lock(&path).write_owned().await;
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn path(&self, name: &str) -> io::Result<PathBuf> {
        let path = self.0.root.join(name);
        check_path_no_symlink(&self.0.root, &path, true).map_err(io::Error::other)?;
        Ok(path)
    }

    fn lock(&self, path: &std::path::Path) -> Arc<RwLock<()>> {
        self.0
            .locks
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_default()
            .clone()
    }

    pub(super) async fn read(
        &self,
        name: &str,
        method: &Method,
        headers: &HeaderMap,
        base: &str,
    ) -> Result<Response, StatusCode> {
        let path = self.path(name).map_err(|_| StatusCode::NOT_FOUND)?;
        let reading = self.lock(&path).read_owned().await;
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let metadata = file
            .metadata()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !metadata.is_file() {
            return Err(StatusCode::NOT_FOUND);
        }
        let content_type = mime_type(name);
        if content_type.starts_with("text/html") {
            use tokio::io::AsyncReadExt;
            let mut html = String::new();
            file.read_to_string(&mut html)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let html = super::super::reference::html_urls(&html, base)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            return stream::seekable(
                std::io::Cursor::new(html.as_bytes().to_vec()),
                html.len() as u64,
                content_type,
                method,
                headers,
                reading,
            )
            .await;
        }
        stream::seekable(file, metadata.len(), content_type, method, headers, reading).await
    }
}
