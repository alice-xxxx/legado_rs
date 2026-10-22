use super::*;
use crate::application::ApplicationService;
use crate::source_engine::EngineFuture;
use crate::source_engine::SourceEngineRequest;
use crate::source_engine::SourceExecutor;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

struct AudioExecutor {
    requested: mpsc::UnboundedSender<(String, tokio_util::sync::CancellationToken)>,
    next: Arc<tokio::sync::Semaphore>,
}

impl SourceExecutor for AudioExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            assert_eq!(request.operation, "httpTtsAudio");
            assert_eq!(request.page, Some(5));
            let text = request.keyword.unwrap();
            self.requested
                .send((text.clone(), request.cancellation.unwrap()))
                .unwrap();
            self.next.acquire().await.unwrap().forget();
            let (bytes, mime) = if text == "wav" {
                (b"RIFF\0\0\0\0WAVE".to_vec(), "audio/wav")
            } else {
                let mut bytes = vec![0; 417 * 3];
                for frame in bytes.chunks_mut(417) {
                    frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
                }
                (bytes, "audio/mpeg")
            };
            Ok(
                json!({ "bytesBase64": base64::engine::general_purpose::STANDARD.encode(bytes), "contentType": mime }),
            )
        })
    }
}

#[tokio::test]
async fn audio_plays_before_later_text_finishes_and_formats_keep_their_own_streams() {
    let root = tempfile::tempdir().unwrap();
    let (requested, mut requests) = mpsc::unbounded_channel();
    let next = Arc::new(tokio::sync::Semaphore::new(1));
    let service = ApplicationService::open_with_executor(
        root.path(),
        Arc::new(AudioExecutor {
            requested,
            next: next.clone(),
        }),
    )
    .await
    .unwrap();
    let (_, config) = crate::http_tts_config::import_configs(
        r#"{"name":"test","url":"https://example.invalid/tts"}"#,
    )
    .unwrap()
    .remove(0);
    let config=config["config"].clone();
    let (output, mut sources) = mpsc::channel(1);
    let (input, mut texts) = mpsc::channel(1);
    let generating = {
        let service = service.clone();
        tokio::spawn(async move {
            let mut assets = vec![];
            let result = service
                .generate_tts_audio(
                    &config,
                    1.0,
                    vec![TtsText {
                        text: "first".into(),
                        index: 7,
                    }],
                    &mut texts,
                    output,
                    &mut assets,
                )
                .await;
            (result, assets)
        })
    };
    let source = sources.recv().await.unwrap();
    assert_eq!(requests.recv().await.unwrap().0, "first");
    assert_eq!(source.boundaries.borrow()[0].0, 7);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut response = client.get(&source.src).send().await.unwrap();
    assert_eq!(
        response.headers()[reqwest::header::CONTENT_TYPE],
        "audio/mpeg"
    );
    assert_eq!(response.chunk().await.unwrap().unwrap().len(), 417 * 3);
    // 队列暂空也不关闭音频流，下一批直接追加，不重启播放器。
    assert!(
        tokio::time::timeout(Duration::from_millis(30), response.chunk())
            .await
            .is_err()
    );
    input
        .send(vec![
            TtsText {
                text: "second".into(),
                index: 8,
            },
            TtsText {
                text: "wav".into(),
                index: 9,
            },
        ])
        .await
        .unwrap();
    assert_eq!(requests.recv().await.unwrap().0, "second");
    assert_eq!(requests.recv().await.unwrap().0, "wav");
    assert!(!generating.is_finished());
    next.add_permits(1);
    assert_eq!(response.chunk().await.unwrap().unwrap().len(), 417 * 3);
    next.add_permits(1);
    assert!(response.chunk().await.unwrap().is_none());
    assert_eq!(
        source
            .boundaries
            .borrow()
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        [7, 8]
    );
    source.played.send(()).unwrap();
    let wav = sources.recv().await.unwrap();
    assert_eq!(wav.boundaries.borrow()[0].0, 9);
    let response = client.get(wav.src).send().await.unwrap();
    assert_eq!(
        response.headers()[reqwest::header::CONTENT_TYPE],
        "audio/wav"
    );
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        b"RIFF\0\0\0\0WAVE"
    );
    wav.played.send(()).unwrap();
    assert!(!generating.is_finished());
    drop(input);
    let (result, assets) = generating.await.unwrap();
    result.unwrap();
    assert_eq!(assets.len(), 2);
    for id in assets {
        service.server.release_tts_stream(&id).await;
    }
}

#[tokio::test]
async fn ending_pending_synthesis_cancels_the_engine_request() {
    let root = tempfile::tempdir().unwrap();
    let (requested, mut requests) = mpsc::unbounded_channel();
    let service = ApplicationService::open_with_executor(
        root.path(),
        Arc::new(AudioExecutor {
            requested,
            next: Arc::new(tokio::sync::Semaphore::new(0)),
        }),
    )
    .await
    .unwrap();
    let generating =
        tokio::spawn(async move { service.tts_bytes(&json!({}), "pending", 1.0).await });
    let (_, token) = requests.recv().await.unwrap();
    assert!(!token.is_cancelled());
    generating.abort();
    assert!(generating.await.unwrap_err().is_cancelled());
    assert!(token.is_cancelled());
}

#[tokio::test]
async fn draining_text_keeps_the_session_open_and_stop_waits_for_player_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let (requested, _) = mpsc::unbounded_channel();
    let service = ApplicationService::open_with_executor(
        root.path(),
        Arc::new(AudioExecutor {
            requested,
            next: Arc::new(tokio::sync::Semaphore::new(0)),
        }),
    )
    .await
    .unwrap();
    let (input, mut receiver) = mpsc::channel(1);
    let (cancel, mut cancellation) = watch::channel(false);
    service.tts.send_replace(Some(TtsSession {
        input: input.clone(),
        cancel,
    }));
    let input = service.tts.borrow().as_ref().unwrap().input.clone();
    for text in ["first", "later"] {
        let texts = vec![TtsText {
            text: text.into(),
            index: 0,
        }];
        validate_texts(&texts).unwrap();
        input.send(texts).await.unwrap();
        assert_eq!(receiver.recv().await.unwrap()[0].text, text);
        assert!(!*cancellation.borrow());
        assert!(service.tts.borrow().is_some());
    }
    let stopping = tokio::spawn({
        let service = service.clone();
        async move {
            service.end_tts().await;
        }
    });
    cancellation.wait_for(|cancelled| *cancelled).await.unwrap();
    assert!(service.tts.borrow().is_none());
    assert!(!stopping.is_finished());
    drop(receiver);
    stopping.await.unwrap();
}
#[test]
fn strips_id3_and_counts_mpeg_frames_for_stream_boundaries() {
    // MPEG-1 Layer III, 128 kb/s, 44.1 kHz: 417 bytes per frame.
    let mut bytes = b"ID3\x04\0\0\0\0\0\0".to_vec();
    for _ in 0..3 {
        let mut frame = vec![0; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        bytes.extend(frame);
    }
    let (payload, duration, _) = mp3_stream_payload(&bytes).unwrap();
    assert_eq!(payload.len(), 3 * 417);
    assert!((duration - 3.0 * 1152.0 / 44100.0).abs() < 0.0001);
}

#[test]
fn rejects_container_bytes_that_cannot_be_joined_as_mp3() {
    assert!(mp3_stream_payload(b"RIFF\0\0\0\0WAVE").is_err());
    assert!(mp3_stream_payload(b"ID3\x04\0\0\xff\0\0\0").is_err());
    assert!(mp3_stream_payload(b"ID3\x04\0\0\x7f\x7f\x7f\x7f").is_err());
}

#[test]
fn removes_per_file_duration_frames_before_joining_chapter_audio() {
    let mut bytes = Vec::new();
    for index in 0..3 {
        let mut frame = vec![0; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        if index == 0 {
            frame[36..40].copy_from_slice(b"Info");
        }
        bytes.extend(frame);
    }
    let (payload, duration, _) = mp3_stream_payload(&bytes).unwrap();
    assert_eq!(payload.len(), 2 * 417);
    assert!((duration - 2.0 * 1152.0 / 44100.0).abs() < 0.0001);
}
