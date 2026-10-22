//! 将持续投递的 TTS 音频片段通过同一 HTTP 流交给播放器。

use super::{
    server::{empty_response, set_cors_headers, stream_response},
    ResourceError,
};
use axum::{
    body::Body,
    http::{header::*, HeaderMap, Method, Response, StatusCode},
};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, watch, Mutex},
};
use tokio_stream::wrappers::ReceiverStream;

#[derive(Clone, Copy, Default)]
struct Buffered {
    length: u64,
    finished: bool,
}

struct AudioBuffer {
    path: tempfile::TempPath,
    buffered: watch::Sender<Buffered>,
    cancellation: watch::Sender<bool>,
    content_type: &'static str,
}

struct Entry {
    audio: Arc<AudioBuffer>,
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
        content_type: &'static str,
    ) -> Result<TtsStreamWriter, ResourceError> {
        let (file, path) = tempfile::NamedTempFile::new()?.into_parts();
        let (buffered, _) = watch::channel(Buffered::default());
        let (cancellation, receiver) = watch::channel(false);
        let audio = Arc::new(AudioBuffer {
            path,
            buffered,
            cancellation,
            content_type,
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
                accessed: Instant::now(),
            },
        );
        Ok(TtsStreamWriter {
            url: format!("{base_url}tts-stream/{id}"),
            id,
            cancellation: receiver,
            audio,
            file: tokio::fs::File::from_std(file),
        })
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
            let mut response =
                stream_response(file, state.length, headers, audio.content_type, "").await;
            if method == Method::HEAD {
                *response.body_mut() = Body::empty();
            }
            return response;
        }
        // The final length does not exist yet. HTTP permits ignoring Range;
        // advertise no seeking while live, and support normal ranges once done.
        let mut response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, audio.content_type)
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

    #[tokio::test]
    async fn live_readers_can_probe_disconnect_and_reconnect_without_consuming_audio() {
        let registry = TtsStreamRegistry::default();
        let mut writer = registry
            .create("http://localhost/r/token/", "audio/mpeg")
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
        let mut writer = registry.create("", "audio/mpeg").await.unwrap();
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
            writers.push(registry.create("", "audio/mpeg").await.unwrap());
        }
        assert!(*writers[0].cancellation.borrow());
        assert_eq!(
            registry
                .serve(&writers[0].id, Method::HEAD, &HeaderMap::new())
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        registry.clear().await;
        assert!(writers.iter().all(|writer| *writer.cancellation.borrow()));
    }
}
