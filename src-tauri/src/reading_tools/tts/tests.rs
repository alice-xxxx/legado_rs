//! 验证真实 HTTP 流的拼接和缓冲；人工 MP3 帧不用于听感验收。
use super::*;
use crate::{database::Database, resources::Resources};
use serde_json::json;
use std::time::Duration;
use tauri_plugin_source_engine::*;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

struct AudioEngine {
    frames: usize,
    empty_second: bool,
}
macro_rules! unexpected {
    ($name:ident, $request:ty, $response:ty) => {
        fn $name<'a>(&'a self, _: $request, _: CancellationToken) -> EngineFuture<'a, $response> {
            Box::pin(async { Err("朗读不应调用其他引擎能力".into()) })
        }
    };
}
impl SourceExecutor for AudioEngine {
    unexpected!(search, SearchRequest, SearchPage);
    unexpected!(explore_kinds, SourceRequest, ExploreKinds);
    unexpected!(explore, ExploreRequest, SearchPage);
    unexpected!(detail, BookRequest, Book);
    unexpected!(chapters, BookRequest, ChapterList);
    unexpected!(content, ContentRequest, ChapterContent);
    unexpected!(resolve_media, MediaRequest, MediaLocation);
    unexpected!(login, LoginRequest, LoginState);
    fn http_tts_audio<'a>(
        &'a self,
        request: TtsRequest,
        _: CancellationToken,
    ) -> EngineFuture<'a, AudioData> {
        Box::pin(async move {
            let index = request.text.parse::<usize>().unwrap();
            // 故意乱序完成，请求结果仍须按正文顺序写入。
            sleep(Duration::from_millis(if index % 3 == 1 { 30 } else { 5 })).await;
            let bytes = if self.empty_second && index == 1 {
                vec![]
            } else if self.frames == 0 {
                [
                    b"RIFF".as_slice(),
                    &44u32.to_le_bytes(),
                    b"WAVEfmt ",
                    &16u32.to_le_bytes(),
                    &[1, 0, 1, 0],
                    &8000u32.to_le_bytes(),
                    &16000u32.to_le_bytes(),
                    &[2, 0, 16, 0],
                    b"data",
                    &8u32.to_le_bytes(),
                    &[index as u8; 8],
                ]
                .concat()
            } else {
                mp3(index, self.frames)
            };
            Ok(AudioData {
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                content_type: if self.frames == 0 {
                    "audio/wav"
                } else {
                    "audio/mpeg"
                }
                .into(),
            })
        })
    }
}

fn mp3(index: usize, frames: usize) -> Vec<u8> {
    let mut bytes = b"ID3\x04\0\0\0\0\0\0".to_vec();
    for number in 0..=frames {
        let mut frame = vec![(index + 1) as u8; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
        if number == 0 {
            frame[36..40].copy_from_slice(b"Info");
        }
        bytes.extend(frame);
    }
    bytes
}
fn texts(count: usize) -> Vec<TtsText> {
    (0..count)
        .map(|index| TtsText {
            index,
            text: index.to_string(),
        })
        .collect()
}
async fn resources() -> (tempfile::TempDir, Resources) {
    let root = tempfile::tempdir().unwrap();
    let database = Database::new(root.path()).unwrap();
    let resources = Resources::open(root.path(), database).await.unwrap();
    (root, resources)
}

#[tokio::test]
async fn prepared_audio_is_appended_without_playback_feedback() {
    let (_root, resources) = resources().await;
    let service = Tts::new(Arc::new(AudioEngine {
        frames: 40,
        empty_second: false,
    }));
    let (input, mut incoming) = mpsc::channel(1);
    let (output, mut sources) = mpsc::channel(1);
    let mut assets = vec![];
    let config = json!({});
    let generating = service.generate_tts_audio(
        &resources,
        &config,
        1.0,
        texts(8),
        &mut incoming,
        output,
        &mut assets,
    );
    fn assert_send(_: &impl Send) {}
    assert_send(&generating);
    let consuming = async {
        let mut source = sources.recv().await.unwrap();
        assert!(source.boundaries.borrow().last().unwrap().1 >= 3.0);
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut response = client.get(&source.src).send().await.unwrap();
        assert_eq!(response.headers()["accept-ranges"], "none");
        // 不发送播放事件，所有预取结果也必须已经进入同一条流。
        source
            .boundaries
            .wait_for(|items| items.len() == 8)
            .await
            .unwrap();
        let expected: Vec<_> = (0..8)
            .flat_map(|index| mp3_stream_payload(&mp3(index, 40)).unwrap().0.to_vec())
            .collect();
        let mut body = vec![];
        while body.len() < expected.len() {
            body.extend(response.chunk().await.unwrap().unwrap());
        }
        assert_eq!(body, expected);
        assert!(timeout(Duration::from_millis(40), response.chunk())
            .await
            .is_err());
        input
            .send(vec![TtsText {
                text: "8".into(),
                index: 8,
            }])
            .await
            .unwrap();
        source
            .boundaries
            .wait_for(|items| items.len() == 9)
            .await
            .unwrap();
        drop(input);
        let mut tail = vec![];
        while let Some(bytes) = response.chunk().await.unwrap() {
            tail.extend(bytes);
        }
        assert_eq!(tail, mp3_stream_payload(&mp3(8, 40)).unwrap().0);
        assert!(sources.recv().await.is_none());
        let items = source.boundaries.borrow();
        assert_eq!(
            items.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            (0..9).collect::<Vec<_>>()
        );
        assert!(items.windows(2).all(|pair| pair[0].1 < pair[1].1));
    };
    let (result, ()) = timeout(Duration::from_secs(5), async {
        tokio::join!(generating, consuming)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(assets.len(), 1);
}

#[tokio::test]
async fn short_batch_and_empty_audio_do_not_wait_for_more_text() {
    for (count, empty_second) in [(1, false), (2, true)] {
        let (_root, resources) = resources().await;
        let service = Tts::new(Arc::new(AudioEngine {
            frames: 4,
            empty_second,
        }));
        let (input, mut incoming) = mpsc::channel(1);
        let (output, mut sources) = mpsc::channel(1);
        let mut assets = vec![];
        let config = json!({});
        let generating = service.generate_tts_audio(
            &resources,
            &config,
            1.0,
            texts(count),
            &mut incoming,
            output,
            &mut assets,
        );
        let consuming = async {
            let source = sources.recv().await.unwrap();
            let items = source.boundaries.borrow().clone();
            assert_eq!(
                items.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
                vec![0]
            );
            assert!(items.last().unwrap().1 < 3.0);
            drop(input);
            let body = reqwest::get(source.src)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            let expected: Vec<_> = [0]
                .into_iter()
                .flat_map(|index| mp3_stream_payload(&mp3(index, 4)).unwrap().0.to_vec())
                .collect();
            assert_eq!(body.as_ref(), expected);
            assert!(sources.recv().await.is_none());
        };
        let (result, ()) = timeout(Duration::from_secs(5), async {
            tokio::join!(generating, consuming)
        })
        .await
        .unwrap();
        result.unwrap();
        assert_eq!(assets.len(), 1);
    }
}

#[tokio::test]
async fn independent_formats_are_sealed_before_playback() {
    let (_root, resources) = resources().await;
    let service = Tts::new(Arc::new(AudioEngine {
        frames: 0,
        empty_second: false,
    }));
    let (input, mut incoming) = mpsc::channel(1);
    drop(input);
    let (output, mut sources) = mpsc::channel(1);
    let mut assets = vec![];
    let config = json!({});
    let generating = service.generate_tts_audio(
        &resources,
        &config,
        1.0,
        texts(3),
        &mut incoming,
        output,
        &mut assets,
    );
    let consuming = async {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for index in 0..3 {
            let source = sources.recv().await.unwrap();
            assert_eq!(source.boundaries.borrow()[0].0, index);
            let response = client
                .get(source.src)
                .header("Range", "bytes=0-3")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 206);
            assert_eq!(response.headers()["content-length"], "4");
            assert_eq!(response.bytes().await.unwrap().as_ref(), b"RIFF");
        }
        assert!(sources.recv().await.is_none());
    };
    let (result, ()) = timeout(Duration::from_secs(5), async {
        tokio::join!(generating, consuming)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(assets.len(), 3);
}
