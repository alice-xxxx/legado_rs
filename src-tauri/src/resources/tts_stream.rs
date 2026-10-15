//! Replayable chapter audio. Each HTTP reader has its own cursor; the producer
//! writes to a bounded temporary file rather than handing one socket a receiver.

use super::{
    server::{empty_response, set_cors_headers, stream_response},
    ResourceError,
};
use axum::{
    body::Body,
    http::{header::*, HeaderMap, Method, Response, StatusCode},
};
use serde::Serialize;
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, watch, Mutex},
};
use tokio_stream::wrappers::ReceiverStream;

const MAX_STREAM_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStreamProgress {
    pub boundaries: Vec<TtsStreamBoundary>,
    pub state: &'static str,
    pub next_index: usize,
    pub skipped: usize,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStreamBoundary {
    pub index: usize,
    pub highlight_index: usize,
    pub end_seconds: f64,
}

#[derive(Clone, Copy, Default)]
struct Buffered {
    length: u64,
    finished: bool,
}

struct AudioBuffer {
    path: tempfile::TempPath,
    buffered: watch::Sender<Buffered>,
    cancellation: watch::Sender<bool>,
}

struct Entry {
    audio: Arc<AudioBuffer>,
    progress: TtsStreamProgress,
    accessed: Instant,
}

#[derive(Clone, Default)]
pub(super) struct TtsStreamRegistry(Arc<Mutex<HashMap<String, Entry>>>);

pub struct TtsStreamWriter {
    pub id: String,
    pub url: String,
    pub cancellation: watch::Receiver<bool>,
    audio: Arc<AudioBuffer>,
    file: tokio::fs::File,
}

impl TtsStreamWriter {
    pub async fn append(&mut self, bytes: &[u8]) -> Result<(), std::io::Error> {
        if *self.cancellation.borrow() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "TTS stream cancelled",
            ));
        }
        let length = self.audio.buffered.borrow().length + bytes.len() as u64;
        if length > MAX_STREAM_BYTES {
            return Err(std::io::Error::other(
                "HTTP TTS chapter audio exceeds 256 MiB",
            ));
        }
        self.file.write_all(bytes).await?;
        self.file.flush().await?;
        self.audio
            .buffered
            .send_modify(|state| state.length = length);
        Ok(())
    }
}

impl Drop for TtsStreamWriter {
    fn drop(&mut self) {
        self.audio
            .buffered
            .send_modify(|state| state.finished = true);
    }
}

impl TtsStreamRegistry {
    pub async fn create(
        &self,
        base_url: &str,
        progress: TtsStreamProgress,
    ) -> Result<TtsStreamWriter, ResourceError> {
        let (file, path) = tempfile::NamedTempFile::new()?.into_parts();
        let (buffered, _) = watch::channel(Buffered::default());
        let (cancellation, receiver) = watch::channel(false);
        let audio = Arc::new(AudioBuffer {
            path,
            buffered,
            cancellation,
        });
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut entries = self.0.lock().await;
        // A lost WebView cannot release its old handle. Bound retained sessions
        // without a timer or a dependency on the lifetime of an HTTP connection.
        if entries.len() >= 4 {
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.accessed)
                .map(|(id, _)| id.clone())
            {
                if let Some(entry) = entries.remove(&oldest) {
                    entry.audio.cancellation.send_replace(true);
                }
            }
        }
        entries.insert(
            id.clone(),
            Entry {
                audio: audio.clone(),
                progress,
                accessed: Instant::now(),
            },
        );
        Ok(TtsStreamWriter {
            url: format!("{base_url}tts-stream/{id}.mp3"),
            id,
            cancellation: receiver,
            audio,
            file: tokio::fs::File::from_std(file),
        })
    }

    pub async fn progress(&self, id: &str) -> Option<TtsStreamProgress> {
        let mut entries = self.0.lock().await;
        let entry = entries.get_mut(id)?;
        entry.accessed = Instant::now();
        Some(entry.progress.clone())
    }

    pub async fn update(&self, id: &str, progress: TtsStreamProgress) {
        if let Some(entry) = self.0.lock().await.get_mut(id) {
            entry.progress = progress;
        }
    }

    pub async fn release(&self, id: &str) -> bool {
        if let Some(entry) = self.0.lock().await.remove(id) {
            entry.audio.cancellation.send_replace(true);
            true
        } else {
            false
        }
    }

    pub async fn clear(&self) {
        for (_, entry) in self.0.lock().await.drain() {
            entry.audio.cancellation.send_replace(true);
        }
    }

    pub async fn serve(&self, id: &str, method: Method, headers: &HeaderMap) -> Response<Body> {
        let audio = {
            let mut entries = self.0.lock().await;
            let Some(entry) = entries.get_mut(id) else {
                return empty_response(StatusCode::NOT_FOUND);
            };
            entry.accessed = Instant::now();
            entry.audio.clone()
        };
        let mut buffered = audio.buffered.subscribe();
        let mut cancelled = audio.cancellation.subscribe();
        let state = *buffered.borrow_and_update();
        let file = match tokio::fs::File::open(&audio.path).await {
            Ok(file) => file,
            Err(_) => return empty_response(StatusCode::NOT_FOUND),
        };
        if state.finished {
            let mut response = stream_response(file, state.length, headers, "audio/mpeg", "").await;
            if method == Method::HEAD {
                *response.body_mut() = Body::empty();
            }
            return response;
        }
        // The final length does not exist yet. HTTP permits ignoring Range;
        // advertise no seeking while live, and support normal ranges once done.
        let mut response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "audio/mpeg")
            .header(CACHE_CONTROL, "no-store")
            .header(ACCEPT_RANGES, "none")
            .body(Body::empty())
            .unwrap();
        set_cors_headers(response.headers_mut());
        if method == Method::HEAD {
            return response;
        }
        let (sender, receiver) = mpsc::channel(2);
        tokio::spawn(async move {
            let _keep_alive = audio;
            let mut file = file;
            let mut offset = 0;
            loop {
                if *cancelled.borrow() {
                    return;
                }
                let state = *buffered.borrow_and_update();
                while offset < state.length {
                    let mut bytes = vec![0; (state.length - offset).min(64 * 1024) as usize];
                    match file.read_exact(&mut bytes).await {
                        Ok(_) => offset += bytes.len() as u64,
                        Err(error) => {
                            let _ = sender.send(Err(error)).await;
                            return;
                        }
                    }
                    tokio::select! {
                        result = sender.send(Ok(bytes)) => if result.is_err() { return; },
                        _ = cancelled.changed() => return,
                    }
                }
                if state.finished {
                    return;
                }
                tokio::select! {
                    _ = sender.closed() => return,
                    _ = cancelled.changed() => return,
                    result = buffered.changed() => if result.is_err() { return; },
                }
            }
        });
        *response.body_mut() = Body::from_stream(ReceiverStream::new(receiver));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tokio_stream::StreamExt;

    fn progress() -> TtsStreamProgress {
        TtsStreamProgress {
            boundaries: vec![],
            state: "running",
            next_index: 0,
            skipped: 0,
            error: None,
        }
    }

    #[tokio::test]
    async fn live_readers_can_probe_disconnect_and_reconnect_without_consuming_audio() {
        let registry = TtsStreamRegistry::default();
        let mut writer = registry
            .create("http://localhost/r/token/", progress())
            .await
            .unwrap();
        let id = writer.id.clone();
        writer.append(b"first").await.unwrap();
        let probe = registry.serve(&id, Method::HEAD, &HeaderMap::new()).await;
        assert_eq!(probe.status(), StatusCode::OK);
        assert!(to_bytes(probe.into_body(), 100).await.unwrap().is_empty());
        let first = registry.serve(&id, Method::GET, &HeaderMap::new()).await;
        let mut reader = first.into_body().into_data_stream();
        assert_eq!(reader.next().await.unwrap().unwrap().as_ref(), b"first");
        drop(reader);
        writer.append(b"second").await.unwrap();
        let second = registry.serve(&id, Method::GET, &HeaderMap::new()).await;
        let third = registry.serve(&id, Method::GET, &HeaderMap::new()).await;
        writer.append(b"third").await.unwrap();
        drop(writer);
        for response in [second, third] {
            let bytes = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                to_bytes(response.into_body(), 100),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(bytes.as_ref(), b"firstsecondthird");
        }
        let mut headers = HeaderMap::new();
        headers.insert(RANGE, "bytes=3-7".parse().unwrap());
        let range = registry.serve(&id, Method::GET, &headers).await;
        assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(range.headers()[CONTENT_RANGE], "bytes 3-7/16");
        assert_eq!(
            to_bytes(range.into_body(), 100).await.unwrap().as_ref(),
            b"stsec"
        );
        assert!(registry.release(&id).await);
        assert_eq!(
            registry.serve(&id, Method::GET, &headers).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn cancellation_unblocks_idle_readers_and_removes_the_temporary_file() {
        let registry = TtsStreamRegistry::default();
        let mut writer = registry.create("", progress()).await.unwrap();
        let path = writer.audio.path.to_path_buf();
        let response = registry
            .serve(&writer.id, Method::GET, &HeaderMap::new())
            .await;
        assert!(registry.release(&writer.id).await);
        assert!(writer.append(b"late").await.is_err());
        drop(writer);
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            to_bytes(response.into_body(), 100),
        )
        .await
        .unwrap()
        .unwrap();
        // The response task releases its file and removal guard before ending.
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn lost_handles_are_bounded_and_restore_cancels_all_producers() {
        let registry = TtsStreamRegistry::default();
        let mut writers = Vec::new();
        for _ in 0..5 {
            writers.push(registry.create("", progress()).await.unwrap());
        }
        assert!(*writers[0].cancellation.borrow());
        assert!(registry.progress(&writers[0].id).await.is_none());
        registry.clear().await;
        assert!(writers.iter().all(|writer| *writer.cancellation.borrow()));
    }
}
