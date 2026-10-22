//! TTS 会话保留已生成音频，支持持续播放和完成后的分段重读。
use super::stream::{self, Progress};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};
use tokio::{io::AsyncWriteExt, sync::watch};

struct Audio {
    path: tempfile::TempPath,
    progress: watch::Sender<Progress>,
    content_type: &'static str,
}

#[derive(Clone, Default)]
pub(crate) struct Tts(Arc<Mutex<HashMap<String, Arc<Audio>>>>);

pub(crate) struct Writer {
    pub id: String,
    audio: Arc<Audio>,
    file: tokio::fs::File,
}

impl Tts {
    pub(crate) async fn create(&self, content_type: &'static str) -> io::Result<Writer> {
        let temporary = tokio::task::spawn_blocking(tempfile::NamedTempFile::new)
            .await
            .map_err(io::Error::other)??;
        let (file, path) = temporary.into_parts();
        let audio = Arc::new(Audio {
            path,
            progress: watch::channel(Progress::default()).0,
            content_type,
        });
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.0.lock().unwrap().insert(id.clone(), audio.clone());
        Ok(Writer {
            id,
            audio,
            file: tokio::fs::File::from_std(file),
        })
    }

    pub(crate) fn release(&self, id: &str) -> bool {
        let Some(audio) = self.0.lock().unwrap().remove(id) else {
            return false;
        };
        audio.progress.send_modify(|state| state.cancelled = true);
        true
    }

    pub(crate) fn clear(&self) {
        for (_, audio) in self.0.lock().unwrap().drain() {
            audio.progress.send_modify(|state| state.cancelled = true);
        }
    }

    pub(super) fn routes(&self) -> Router {
        Router::new()
            .route("/{id}", get(read))
            .with_state(self.clone())
    }
}

impl Writer {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.audio.progress.borrow().cancelled
    }

    pub(crate) async fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "TTS 已取消"));
        }
        self.file.write_all(bytes).await?;
        self.file.flush().await?;
        self.audio
            .progress
            .send_modify(|state| state.length += bytes.len() as u64);
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.audio
            .progress
            .send_modify(|state| state.finished = true);
    }
}

async fn read(
    State(tts): State<Tts>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let audio = tts
        .0
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    let progress = audio.progress.subscribe();
    let state = *progress.borrow();
    if state.cancelled {
        return Err(StatusCode::NOT_FOUND);
    }
    let file = tokio::fs::File::open(&audio.path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let response = if state.finished {
        stream::seekable(
            file,
            state.length,
            audio.content_type,
            &method,
            &headers,
            audio.clone(),
        )
        .await?
    } else {
        Response::builder()
            .header(header::CONTENT_TYPE, audio.content_type)
            .header(header::ACCEPT_RANGES, "none")
            .body(stream::body(
                &method,
                stream::growing(file, progress),
                audio,
            ))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    Ok(stream::no_store(response))
}
