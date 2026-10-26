//! Real audio fixtures exercise decoding, continuous encoding and the HTTP stream.
use super::*;
use crate::{database::Database, resources::Resources};
use serde_json::json;
use std::time::Duration;
use tauri_plugin_source_engine::*;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

const FORMATS: &[(&str, &[u8])] = &[
    ("audio/wav", include_bytes!("fixtures/tone.wav")),
    ("audio/mpeg", include_bytes!("fixtures/tone.mp3")),
    ("audio/aac", include_bytes!("fixtures/tone.aac")),
    ("audio/mp4", include_bytes!("fixtures/tone.m4a")),
    ("audio/flac", include_bytes!("fixtures/tone.flac")),
    ("audio/ogg", include_bytes!("fixtures/tone.ogg")),
    ("audio/opus", include_bytes!("fixtures/tone.opus")),
];

struct AudioEngine {
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
            // Complete out of order, but preserve the original text order in the stream.
            sleep(Duration::from_millis(if index % 3 == 1 { 30 } else { 5 })).await;
            let (content_type, bytes) = FORMATS[index % FORMATS.len()];
            Ok(AudioData {
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(
                    if self.empty_second && index == 1 {
                        &[]
                    } else {
                        bytes
                    },
                ),
                content_type: content_type.into(),
            })
        })
    }
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
async fn mixed_formats_continue_in_one_stream_without_playback_feedback() {
    let (_root, resources) = resources().await;
    let service = Tts::new(Arc::new(AudioEngine {
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
        assert!(source.boundaries.borrow().len() >= 3);
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client.get(&source.src).send().await.unwrap();
        assert_eq!(response.headers()["content-type"], "audio/mpeg");
        assert_eq!(response.headers()["accept-ranges"], "none");
        // The producer must keep appending before any playback progress is reported.
        source
            .boundaries
            .wait_for(|items| items.len() == 8)
            .await
            .unwrap();
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
        // Finish the audio stream while the session remains open for user controls.
        input.send(vec![]).await.unwrap();
        let bytes = response.bytes().await.unwrap();
        assert!(sources.recv().await.is_none());
        let items = source.boundaries.borrow().clone();
        assert_eq!(
            items.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            (0..9).collect::<Vec<_>>()
        );
        assert!(items.windows(2).all(|pair| pair[0].1 < pair[1].1));
        let decoded = audio::decode(bytes.to_vec()).unwrap();
        let duration = decoded.frames() as f64 / audio::SAMPLE_RATE as f64;
        assert!((duration - items.last().unwrap().1).abs() < 0.1);
        // A valid continuous audio signal must span every input format boundary.
        for (_, end) in items.iter().take(8) {
            let center = (*end * audio::SAMPLE_RATE as f64) as usize;
            let samples = &decoded.data[0]
                [center.saturating_sub(4410)..(center + 4410).min(decoded.frames())];
            let energy =
                samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
            assert!(energy > 0.001, "silent audio boundary at {end}");
        }
    };
    let (result, ()) = timeout(Duration::from_secs(15), async {
        tokio::join!(generating, consuming)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(assets.len(), 1);
}

#[tokio::test]
async fn short_batch_and_empty_audio_do_not_wait_for_more_text() {
    for (count, empty_second) in [(1, false), (2, false), (2, true)] {
        let (_root, resources) = resources().await;
        let service = Tts::new(Arc::new(AudioEngine { empty_second }));
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
            let expected: Vec<_> = (0..count)
                .filter(|index| !(empty_second && *index == 1))
                .collect();
            assert_eq!(
                source
                    .boundaries
                    .borrow()
                    .iter()
                    .map(|(index, _)| *index)
                    .collect::<Vec<_>>(),
                expected
            );
            input.send(vec![]).await.unwrap();
            let bytes = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(source.src)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            assert!(audio::decode(bytes.to_vec()).unwrap().frames() > 0);
            assert!(sources.recv().await.is_none());
        };
        let (result, ()) = timeout(Duration::from_secs(15), async {
            tokio::join!(generating, consuming)
        })
        .await
        .unwrap();
        result.unwrap();
        assert_eq!(assets.len(), 1);
    }
}

#[test]
fn all_input_formats_decode_to_the_same_sample_rate() {
    for (format, bytes) in FORMATS {
        let audio =
            audio::decode(bytes.to_vec()).unwrap_or_else(|error| panic!("{format}: {error}"));
        assert_eq!(audio.sample_rate.get(), audio::SAMPLE_RATE);
        let duration = audio.frames() as f64 / audio::SAMPLE_RATE as f64;
        assert!((0.45..0.65).contains(&duration), "{format}: {duration}");
    }
    assert!(audio::decode(b"invalid audio".to_vec()).is_err());
}
