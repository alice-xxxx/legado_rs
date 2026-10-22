//! 资源登记、提交后清理和数据库备份。
use super::*;

impl Database {
    pub(crate) fn save_media(&self, request: MediaRequest) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            if s.write("INSERT INTO media_requests(id,book_id,url) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET url=excluded.url WHERE media_requests.book_id=excluded.book_id", params![request.id,request.book_id,request.url])?==0 {
                return Err(ResourceError::new("媒体请求属于其他书籍"));
            }
            s.write("DELETE FROM media_headers WHERE owner_id=?1", [&request.id])?;
            for (position, (name, value)) in request.headers.into_iter().enumerate() {
                s.write(
                    "INSERT INTO media_headers(owner_id,position,name,value) VALUES(?1,?2,?3,?4)",
                    params![request.id, position as i64, name, value],
                )?;
            }
            Ok(())
        })
    }

    pub(crate) fn remove_media(&self, book: String, id: String) -> Result<Pending<()>> {
        db::write(self, WriteMode::Transaction, move |s| {
            s.write("DELETE FROM media_requests WHERE id=?1 AND book_id=?2 AND NOT EXISTS(SELECT 1 FROM content_media WHERE media_id=?1)",params![id,book])?;
            Ok(())
        })
    }

    pub(crate) fn media(&self) -> Result<Vec<MediaRequest>> {
        db::read(self, |s| {
            let mut requests = s.read(
                "SELECT id,coalesce(book_id,''),url FROM media_requests",
                [],
                |r| {
                    Ok(MediaRequest {
                        id: r.get(0)?,
                        book_id: r.get(1)?,
                        url: r.get(2)?,
                        ..Default::default()
                    })
                },
            )?;
            let indices: std::collections::HashMap<_, _> = requests
                .iter()
                .enumerate()
                .map(|(index, request)| (request.id.clone(), index))
                .collect();
            s.read(
                "SELECT owner_id,name,value FROM media_headers ORDER BY owner_id,position",
                [],
                |r| {
                    let owner: String = r.get(0)?;
                    if let Some(&index) = indices.get(&owner) {
                        requests[index].headers.insert(r.get(1)?, r.get(2)?);
                    }
                    Ok(())
                },
            )?;
            Ok(requests)
        })
    }

    pub(crate) fn has_media(&self, book: &str, id: &str) -> Result<bool> {
        db::read(self, |s| {
            Ok(s.read("SELECT EXISTS(SELECT 1 FROM media_requests m WHERE m.id=?2 AND (m.book_id=?1 OR EXISTS(SELECT 1 FROM content_media a JOIN chapters c ON c.content_id=a.content_id WHERE a.media_id=m.id AND c.book_id=?1)))",params![book,id],|r|r.get(0))?[0])
        })
    }

    pub(crate) fn asset_referenced(&self, uri: &str) -> Result<bool> {
        db::read(self, |s| referenced(s, uri))
    }

    pub(crate) fn clear_chapter_cache(
        &self,
        book: String,
        paths: Vec<String>,
    ) -> Result<Pending<usize>> {
        db::write(self, WriteMode::Transaction, move |s| {
            let online = s
                .read(
                    "SELECT source_id IS NOT NULL AND book_url IS NOT NULL FROM books WHERE id=?1",
                    [&book],
                    |r| r.get::<_, bool>(0),
                )?
                .pop()
                .unwrap_or(false);
            if !online {
                return Err(ResourceError::new("本地书籍不能清除正文"));
            }
            let count = library::detach(s, &book)?;
            schedule(s, paths)?;
            Ok(count)
        })
    }

    pub(crate) fn recover_cleanup(&self, protected: Vec<String>) -> Result<Pending<()>> {
        let root = self.root.clone();
        db::write(self, WriteMode::Connection, move |s| {
            let protected: std::collections::HashSet<_> = protected.into_iter().collect();
            for path in s.read("SELECT path FROM pending_cleanup", [], |r| {
                r.get::<_, String>(0)
            })? {
                let uri = format!("resource://{path}");
                if protected.contains(&uri) || referenced(s, &uri)? {
                    continue;
                }
                let reference = crate::resources::ResourceRef::new(&uri)?;
                if !reference.path().starts_with("cache/") {
                    return Err(ResourceError::new("清理路径必须位于缓存目录"));
                }
                let file = root.join(reference.path());
                crate::resources::check_path_no_symlink(&root, &file, true)?;
                match std::fs::remove_file(&file) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error.into()),
                }
                s.write("DELETE FROM pending_cleanup WHERE path=?1", [path])?;
                if let Some(parent) = file.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
            }
            Ok(())
        })
    }

    /// 元数据先入队；等待方还能等到后续文件清理，清理失败保留待办供下次重试。
    pub(crate) fn remove_book_assets(
        &self,
        book: String,
        protected: Vec<String>,
    ) -> Result<impl std::future::Future<Output = Result<()>>> {
        let root = self.root.clone();
        let removed = db::write(self, WriteMode::Transaction, move |s| {
            let mut paths = Vec::new();
            for kind in crate::resources::CONTENT_TYPES {
                let directory = root.join("cache").join(kind).join(&book);
                crate::resources::check_path_no_symlink(&root, &directory, true)?;
                if !directory.exists() {
                    continue;
                }
                for entry in std::fs::read_dir(directory)? {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        return Err(ResourceError::new("缓存目录含非普通文件"));
                    }
                    paths.push(format!(
                        "cache/{kind}/{book}/{}",
                        entry.file_name().to_string_lossy()
                    ));
                }
            }
            s.write("DELETE FROM books WHERE id=?1", [book])?;
            library::purge(s)?;
            schedule(s, paths)
        })?;
        let database = self.clone();
        Ok(async move {
            removed.await?;
            let cleanup = match database.recover_cleanup(protected) {
                Ok(pending) => pending.await,
                Err(error) => Err(error),
            };
            if let Err(error) = cleanup {
                eprintln!("缓存清理待重试：{error}");
            }
            Ok(())
        })
    }

    pub(crate) fn backup_file(&self, path: PathBuf) -> Result<Pending<()>> {
        db::write(self, WriteMode::Connection, move |s| {
            s.write("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
            Ok(())
        })
    }

    pub(crate) fn restore_archive(
        &self,
        path: PathBuf,
    ) -> std::result::Result<(), crate::backup::state::TransactionError> {
        let root = self.root.clone();
        db::write(self, WriteMode::Closed, move |_| {
            Ok(crate::backup::restore::restore_backup_sync(&root, &path))
        })
        .and_then(Pending::wait)
        .map_err(|error| crate::backup::state::TransactionError::new(error.to_string()))?
    }
}

fn schedule(s: &Session<'_>, paths: Vec<String>) -> Result<()> {
    for path in paths {
        s.write(
            "INSERT OR IGNORE INTO pending_cleanup(path) VALUES(?1)",
            [path],
        )?;
    }
    Ok(())
}
fn referenced(s: &Session<'_>, uri: &str) -> Result<bool> {
    Ok(s.read(
        "SELECT EXISTS(SELECT 1 FROM asset_references WHERE uri=?1)",
        [uri],
        |r| r.get(0),
    )?[0])
}
pub(crate) fn verify_snapshot(root: &Path) -> Result<()> {
    let database = Database {
        root: Arc::new(root.to_owned()),
    };
    db::read(&database, |s| {
        let version = s.read("PRAGMA user_version", [], |r| r.get::<_, u32>(0))?[0];
        if version != 15 {
            return Err(ResourceError::new("备份数据库版本不匹配"));
        }
        let integrity = s.read("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?;
        if integrity != ["ok"] {
            return Err(ResourceError::new("备份数据库损坏"));
        }
        if !s
            .read("PRAGMA foreign_key_check", [], |_| Ok(()))?
            .is_empty()
        {
            return Err(ResourceError::new("备份数据库关系损坏"));
        }
        Ok(())
    })
}
