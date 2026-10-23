//! 朗读会话：开始清空队列，投递后顺序朗读，只有结束才关闭会话。
use std::sync::Arc;

use crate::source_engine::SourceEngineRequest;
use base64::Engine as _;
use serde::Deserialize;
pub(crate) struct Tts {
    executor: Arc<dyn crate::source_engine::SourceExecutor>,
    tts: tokio::sync::watch::Sender<Option<TtsSession>>,
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    system: std::sync::Mutex<Option<SystemTts>>,
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    pub(crate) app: Option<tauri::AppHandle>,
}
impl Tts {
    pub(crate) fn new(executor: Arc<dyn crate::source_engine::SourceExecutor>) -> Self {
        Self {
            executor,
            tts: tokio::sync::watch::channel(None).0,
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            system: std::sync::Mutex::new(None),
            #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
            app: None,
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use serde_json::json;
use serde_json::Value;
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, watch};

const MAX_HTTP_TTS_TEXT_CHARS: usize = 20_000;
const MAX_HTTP_TTS_AUDIO_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTTP_TTS_AUDIO_BASE64_BYTES: usize = ((MAX_HTTP_TTS_AUDIO_BYTES + 2) / 3) * 4;

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsOptions {
    config_id: Option<String>,
    voice_id: Option<String>,
    rate: f32,
}

#[derive(Deserialize)]
pub struct TtsText {
    text: String,
    index: usize,
}

#[derive(Clone)]
pub(crate) struct TtsSession {
    input: mpsc::Sender<Vec<TtsText>>,
    cancel: watch::Sender<bool>,
}

impl TtsSession {
    async fn stop(self) {
        self.cancel.send_replace(true);
        self.input.closed().await;
    }
}

// 音频边界留在 Rust；播放器只报告实际播放位置，Vue 只收到片段序号。
struct TtsAudio {
    src: String,
    boundaries: watch::Receiver<Vec<(usize, f64)>>,
    played: oneshot::Sender<()>,
}

struct TtsStream {
    writer: crate::resources::TtsWriter,
    profile: Option<Mp3StreamProfile>,
    boundaries: watch::Sender<Vec<(usize, f64)>>,
    played: oneshot::Receiver<()>,
}

impl TtsStream {
    async fn finish(self) -> Result<(), String> {
        drop(self.writer); // 音频写完才发送 EOF；等播放器读完后再切换下一段资源。
        self.played.await.map_err(|_| "朗读播放器已关闭".into())
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl Tts {
    fn system_tts(&self) -> Result<SystemTts, String> {
        let mut system = self.system.lock().map_err(|error| error.to_string())?;
        if system.is_none() {
            *system = Some(SystemTts::new().map_err(|error| error.to_string())?);
        }
        Ok(system.as_ref().unwrap().clone())
    }

    fn tts_progress(&self, progress: Value) {
        use tauri::Emitter;
        if let Some(app) = &self.app {
            let _ = app.emit("tts", progress);
        }
    }

    async fn run_tts(
        &self,
        database: &crate::database::Database,
        resources: &crate::resources::Resources,
        options: TtsOptions,
        mut input: mpsc::Receiver<Vec<TtsText>>,
        mut cancellation: watch::Receiver<bool>,
        mut player: TtsPlayer,
    ) {
        let mut assets = vec![];
        loop {
            let texts = tokio::select! {
                biased;
                _ = cancellation.wait_for(|cancelled| *cancelled) => break,
                texts = input.recv() => match texts { Some(texts) => texts, None => break },
            };
            let reading = async {
                if let Some(id) = &options.config_id {
                    let config = database
                        .tts_config(id)
                        .map_err(|e| e.to_string())?
                        .ok_or("朗读配置不存在或已停用")?;
                    let config = serde_json::to_value(config).map_err(|error| error.to_string())?;
                    let (sender, receiver) = mpsc::channel(1);
                    tokio::try_join!(
                        self.generate_tts_audio(
                            resources,
                            &config,
                            options.rate,
                            texts,
                            &mut input,
                            sender,
                            &mut assets
                        ),
                        self.play_tts_audio(&mut player, receiver),
                    )?;
                } else {
                    self.queue_system_tts(texts, &mut input, &mut player)
                        .await?;
                }
                Ok::<_, String>(())
            };
            let result = tokio::select! {
                biased;
                _ = cancellation.wait_for(|cancelled| *cancelled) => break,
                result = reading => result,
            };
            player.stop();
            for id in assets.drain(..) {
                resources.tts.release(&id);
            }
            if let Err(error) = result {
                self.tts_progress(json!(error));
            }
        }
        player.stop();
        for id in assets {
            resources.tts.release(&id);
        }
    }

    async fn play_tts_audio(
        &self,
        player: &mut TtsPlayer,
        mut sources: mpsc::Receiver<TtsAudio>,
    ) -> Result<(), String> {
        while let Some(source) = sources.recv().await {
            while player.events.try_recv().is_ok() {}
            player.audio(&source.src)?;
            let mut highlighted = None;
            let mut paused = None;
            while let Some(event) = player.events.recv().await {
                if event["state"] == "error" {
                    return Err(event["error"].as_str().unwrap_or("音频播放失败").to_owned());
                }
                let ended = event["state"] == "ended";
                if event["state"] == "time" || event["state"] == "paused" {
                    let next = event["state"] == "paused";
                    if paused != Some(next) {
                        paused = Some(next);
                        self.tts_progress(json!({ "paused": next }));
                    }
                }
                if !ended && event["state"] != "time" {
                    continue;
                }
                // 以播放器实际时间定位片段，不按下载进度或文本长度猜测。
                let boundaries = source.boundaries.borrow();
                let index = if ended {
                    boundaries.last().map(|(index, _)| *index)
                } else {
                    let position = event["position"].as_f64().unwrap_or(0.0);
                    boundaries
                        .get(boundaries.partition_point(|(_, end)| *end <= position))
                        .map(|(index, _)| *index)
                };
                if index.is_some() && index != highlighted {
                    highlighted = index;
                    self.tts_progress(json!(index));
                }
                if ended {
                    break;
                }
            }
            let _ = source.played.send(());
        }
        Ok(())
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[derive(Clone)]
struct SystemTts {
    engine: tts::Tts,
    default_voice: Option<tts::Voice>,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl SystemTts {
    fn new() -> Result<Self, tts::Error> {
        // 指定系统合成器，避免默认后端接管正在运行的屏幕阅读器。
        #[cfg(target_os = "windows")]
        let backend = tts::Backends::WinRt;
        #[cfg(target_os = "linux")]
        let backend = tts::Backends::SpeechDispatcher;
        #[cfg(target_vendor = "apple")]
        let backend = tts::Backends::AvFoundation;
        #[cfg(target_os = "android")]
        let backend = tts::Backends::Android;
        let engine = tts::Tts::new(backend)?;
        let default_voice = if engine.supported_features().get_voice {
            engine.voice()?
        } else {
            None
        };
        Ok(Self {
            engine,
            default_voice,
        })
    }
}

// HTTP 音频由 WebView 输出；系统语音各平台统一使用 tts-rs。
#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
struct TtsPlayer {
    app: tauri::AppHandle,
    events: mpsc::UnboundedReceiver<Value>,
    listeners: Vec<tauri::EventId>,
    system: Option<tts::Tts>,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl TtsPlayer {
    fn new(app: tauri::AppHandle, options: &TtsOptions, service: &Tts) -> Result<Self, String> {
        use tauri::Listener;
        let (sender, events) = mpsc::unbounded_channel();
        let mut listeners = vec![];
        let system = if options.config_id.is_none() {
            let SystemTts {
                engine,
                default_voice,
            } = service.system_tts()?;
            let configure = || -> Result<(), tts::Error> {
                let normal = engine.normal_rate()?;
                // Speech Dispatcher 使用以 0 为默认值的百分比，其他后端使用倍率。
                let rate = if normal > 0.0 {
                    normal * options.rate
                } else {
                    50.0 * options.rate.log2()
                };
                engine.set_rate(rate.clamp(engine.min_rate()?, engine.max_rate()?))?;
                if engine.supported_features().voice {
                    let voice = match options.voice_id.as_deref() {
                        Some(id) => engine
                            .voices()?
                            .into_iter()
                            .find(|voice| voice.id() == id)
                            .or(default_voice),
                        None => default_voice,
                    };
                    if let Some(voice) = voice {
                        engine.set_voice(&voice)?;
                    }
                }
                let tx = sender.clone();
                engine.on_utterance_begin(move |id| {
                    let _ = tx.send(json!({ "id": id.to_string(), "state": "start" }));
                })?;
                let tx = sender.clone();
                engine.on_utterance_end(move |id| {
                    let _ = tx.send(json!({ "id": id.to_string(), "state": "ended" }));
                })?;
                let tx = sender.clone();
                engine.on_utterance_stop(move |id| {
                    let _ = tx.send(json!({ "id": id.to_string(), "state": "cancel" }));
                })?;
                if engine.supported_features().pause {
                    let tx = sender.clone();
                    engine.on_utterance_pause(move |id| {
                        let _ = tx.send(json!({ "id": id.to_string(), "state": "paused" }));
                    })?;
                    let tx = sender.clone();
                    engine.on_utterance_resume(move |id| {
                        let _ = tx.send(json!({ "id": id.to_string(), "state": "resumed" }));
                    })?;
                }
                Ok(())
            };
            if let Err(error) = configure() {
                engine.clear_utterance_callbacks();
                return Err(error.to_string());
            }
            Some(engine)
        } else {
            None
        };
        listeners.push(app.listen("tts-playback", move |event| {
            if let Ok(data) = serde_json::from_str(event.payload()) {
                let _ = sender.send(data);
            }
        }));
        Ok(Self {
            app,
            events,
            listeners,
            system,
        })
    }

    fn speak(&self, item: &TtsText, append: bool) -> Result<String, String> {
        self.system
            .as_ref()
            .ok_or("系统语音尚未初始化")?
            .speak(&item.text, !append)
            .map_err(|error| error.to_string())?
            .map(|id| id.to_string())
            .ok_or_else(|| "系统语音没有接受文本".into())
    }

    fn pause(&self) -> Result<bool, String> {
        let engine = self.system.as_ref().ok_or("系统语音尚未初始化")?;
        if !engine.supported_features().pause {
            return Ok(false);
        }
        engine.pause().map_err(|error| error.to_string())?;
        Ok(true)
    }

    fn resume(&self) -> Result<(), String> {
        if let Some(engine) = &self.system {
            if engine.supported_features().pause {
                engine.resume().map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    fn audio(&self, src: &str) -> Result<(), String> {
        use tauri::Emitter;
        self.app
            .emit("tts-output", json!({ "action": "audio", "src": src }))
            .map_err(|error| error.to_string())
    }

    fn stop(&self) {
        use tauri::Emitter;
        if let Some(engine) = &self.system {
            let _ = engine.stop();
        }
        let _ = self.app.emit("tts-output", json!({ "action": "stop" }));
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl Drop for TtsPlayer {
    fn drop(&mut self) {
        use tauri::Listener;
        if let Some(engine) = &self.system {
            engine.clear_utterance_callbacks();
        }
        for listener in &self.listeners {
            self.app.unlisten(*listener);
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn start_tts(
    backup: tauri::State<'_, Arc<crate::backup::state::Operations>>,
    database: tauri::State<'_, crate::database::Database>,
    resources: tauri::State<'_, Arc<crate::resources::Resources>>,
    options: TtsOptions,
    service: tauri::State<'_, Arc<Tts>>,
) -> Result<(), String> {
    if *backup.restore_barrier.borrow() {
        return Err("正在恢复数据，暂时无法朗读".into());
    }
    if !options.rate.is_finite() || !(0.5..=5.0).contains(&options.rate) {
        return Err("朗读语速必须在 0.5 到 5 之间".into());
    }
    let (input, receiver) = mpsc::channel(8);
    let (cancel, cancellation) = watch::channel(false);
    let previous = service.tts.send_replace(Some(TtsSession {
        input,
        cancel: cancel.clone(),
    }));
    // 开始返回前清空旧文本并释放旧播放器；随后投递的内容只属于新会话。
    if let Some(previous) = previous {
        previous.stop().await;
    }
    if *cancellation.borrow() {
        return Err("朗读已结束".into());
    }
    let player = service
        .app
        .clone()
        .ok_or_else(|| "应用尚未初始化".to_owned())
        .and_then(|app| TtsPlayer::new(app, &options, &service));
    let player = match player {
        Ok(player) => player,
        Err(error) => {
            service.tts.send_if_modified(|session| {
                if session
                    .as_ref()
                    .is_some_and(|session| session.cancel.same_channel(&cancel))
                {
                    *session = None;
                    true
                } else {
                    false
                }
            });
            return Err(error);
        }
    };
    let service = service.inner().clone();
    let database = database.inner().clone();
    let resources = resources.inner().clone();
    tokio::spawn(async move {
        service
            .run_tts(
                &database,
                &resources,
                options,
                receiver,
                cancellation,
                player,
            )
            .await;
    });
    Ok(())
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn stop_tts(service: tauri::State<'_, Arc<Tts>>) -> Result<(), String> {
    service.end_tts().await;
    Ok(())
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn tts_voices(service: tauri::State<'_, Arc<Tts>>) -> Result<Vec<Value>, String> {
    let service = service.inner().clone();
    tokio::task::spawn_blocking(move || {
        let engine = service.system_tts()?.engine;
        if !engine.supported_features().voice { return Ok(vec![]); }
        engine.voices().map(|voices| voices.into_iter().map(|voice| {
            json!({ "voiceURI": voice.id(), "name": voice.name(), "lang": voice.language().to_string() })
        }).collect()).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
#[tauri::command]
pub(crate) async fn submit_tts(
    texts: Vec<TtsText>,
    service: tauri::State<'_, Arc<Tts>>,
) -> Result<(), String> {
    validate_texts(&texts)?;
    let session = service.tts.borrow().clone().ok_or("请先开始朗读")?;
    let mut cancellation = session.cancel.subscribe();
    tokio::select! {
        biased;
        _ = cancellation.wait_for(|cancelled| *cancelled) => Err("朗读已结束".into()),
        result = session.input.send(texts) => result.map_err(|_| "朗读已结束".into()),
    }
}

fn validate_texts(texts: &[TtsText]) -> Result<(), String> {
    if texts.is_empty()
        || texts.len() > 2048
        || texts.iter().map(|item| item.text.len()).sum::<usize>() > 8 * 1024 * 1024
        || texts.iter().any(|item| {
            item.text.trim().is_empty()
                || item.text.chars().count() > MAX_HTTP_TTS_TEXT_CHARS
                || item
                    .text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        })
    {
        return Err("朗读文本为空或过长".into());
    }
    Ok(())
}

impl Tts {
    // 提前排入原生队列，片段交接由系统完成，不等 finish 后再发 speak。
    #[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
    async fn queue_system_tts(
        &self,
        texts: Vec<TtsText>,
        input: &mut mpsc::Receiver<Vec<TtsText>>,
        player: &mut TtsPlayer,
    ) -> Result<(), String> {
        let mut pending = std::collections::VecDeque::from(texts);
        let mut utterances = HashMap::<String, TtsText>::new();
        let mut append = false;
        let mut paused = false;
        loop {
            tokio::select! {
                biased;
                Some(event) = player.events.recv() => {
                    // 不支持原生暂停时保留片段，恢复后从当前片段重读。
                    if event["state"] == "pause-requested" {
                        if paused { continue; }
                        if !player.pause()? {
                            let mut interrupted: Vec<_> = utterances.drain().map(|(_, text)| text).collect();
                            interrupted.sort_by_key(|text| text.index);
                            for text in interrupted.into_iter().rev() { pending.push_front(text); }
                            player.stop();
                            append = false;
                        }
                        paused = true;
                        self.tts_progress(json!({ "paused": true }));
                        continue;
                    }
                    if event["state"] == "resume-requested" {
                        if !paused { continue; }
                        player.resume()?;
                        paused = false;
                        self.tts_progress(json!({ "paused": false }));
                        continue;
                    }
                    if event["state"] == "error" && !event["id"].is_string() {
                        return Err(event["error"].as_str().unwrap_or("系统朗读失败").to_owned());
                    }
                    let Some(id) = event["id"].as_str() else { continue; };
                    let Some(text) = utterances.get(id) else { continue; };
                    match event["state"].as_str() {
                        Some("paused" | "resumed") => {
                            paused = event["state"] == "paused";
                            self.tts_progress(json!({ "paused": paused }));
                        },
                        Some("start") => {
                            self.tts_progress(json!({ "paused": false }));
                            self.tts_progress(json!(text.index));
                        },
                        Some("ended") => {
                            utterances.remove(id);
                        },
                        Some("error" | "cancel") => return Err(event["error"].as_str().unwrap_or("系统朗读中断").to_owned()),
                        _ => (),
                    }
                },
                _ = async {}, if !paused && !pending.is_empty() && utterances.len() < 2 => {
                    let item = pending.pop_front().unwrap();
                    let id = player.speak(&item, append)?;
                    utterances.insert(id, item);
                    append = true;
                },
                texts = input.recv() => match texts { Some(texts) => pending.extend(texts), None => return Ok(()) },
            }
        }
    }

    pub(crate) async fn end_tts(&self) {
        let session = self.tts.send_replace(None);
        if let Some(session) = session {
            session.stop().await;
        }
    }

    // KMP 只执行当前选定的 HTTP 规则；不经过任务系统或临时文件写库。
    async fn tts_bytes(
        &self,
        config: &Value,
        text: &str,
        rate: f32,
    ) -> Result<(Vec<u8>, &'static str), String> {
        let token = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = token.clone().drop_guard();
        let mut request = SourceEngineRequest {
            keyword: Some(text.to_owned()),
            page: Some(((rate - 0.5) * 10.0).round() as i32),
            ..SourceEngineRequest::new("httpTtsAudio", config)
        };
        request.cancellation = Some(token);
        let result = match self.executor.execute(request).await {
            Ok(result) => result,
            Err(error) if error.contains("HTTP TTS returned an empty audio response") => {
                return Ok((vec![], "audio/mpeg"))
            }
            Err(error) => return Err(error),
        };
        let encoded = result["bytesBase64"]
            .as_str()
            .ok_or("朗读引擎没有返回音频")?;
        if encoded.len() > MAX_HTTP_TTS_AUDIO_BASE64_BYTES {
            return Err("朗读音频超过 16 MiB".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_HTTP_TTS_AUDIO_BYTES {
            return Err("朗读音频超过 16 MiB".into());
        }
        if bytes.is_empty() {
            return Ok((bytes, "audio/mpeg"));
        }
        let content_type =
            http_tts_audio_type(result["contentType"].as_str().unwrap_or_default(), &bytes)?;
        Ok((bytes, content_type))
    }

    async fn generate_tts_audio(
        &self,
        resources: &crate::resources::Resources,
        config: &Value,
        rate: f32,
        texts: Vec<TtsText>,
        input: &mut mpsc::Receiver<Vec<TtsText>>,
        output: mpsc::Sender<TtsAudio>,
        assets: &mut Vec<String>,
    ) -> Result<(), String> {
        use futures_util::StreamExt;
        let incoming = futures_util::stream::poll_fn(move |context| input.poll_recv(context))
            .flat_map(futures_util::stream::iter);
        let texts = futures_util::stream::iter(texts).chain(incoming);
        // 提前请求三段，仍按投递顺序拼流；短段落不会逐次等待网络往返。
        let audio = texts
            .map(|item| async move {
                let bytes = self.tts_bytes(config, &item.text, rate).await;
                (item.index, bytes)
            })
            .buffered(3);
        futures_util::pin_mut!(audio);
        let mut stream: Option<TtsStream> = None;
        let mut finishing: Option<oneshot::Receiver<()>> = None;
        while let Some((index, result)) = audio.next().await {
            let (bytes, content_type) = result?;
            if bytes.is_empty() {
                continue;
            }
            let parsed = (content_type == "audio/mpeg")
                .then(|| mp3_stream_payload(&bytes).ok())
                .flatten();
            let (payload, duration, profile) = match parsed {
                Some((payload, duration, profile)) => (payload, duration, Some(profile)),
                None => (bytes.as_slice(), f64::INFINITY, None),
            };
            if stream
                .as_ref()
                .is_some_and(|stream| profile.is_none() || stream.profile != profile)
            {
                stream.take().unwrap().finish().await?;
            }
            let source = if stream.is_none() {
                // 下一段已生成，才等待上一段播放完成；非 MP3 也不串行等待请求。
                if let Some(played) = finishing.take() {
                    played.await.map_err(|_| "朗读播放器已关闭")?;
                }
                let writer = resources
                    .tts
                    .create(content_type)
                    .await
                    .map_err(|error| error.to_string())?;
                assets.push(writer.id.clone());
                let (boundaries, progress) = watch::channel(vec![]);
                let (played, completion) = oneshot::channel();
                let source = TtsAudio {
                    src: format!("{}tts/{}", resources.base_url(), writer.id),
                    boundaries: progress,
                    played,
                };
                stream = Some(TtsStream {
                    writer,
                    profile,
                    boundaries,
                    played: completion,
                });
                Some(source)
            } else {
                None
            };
            let current = stream.as_mut().unwrap();
            current
                .writer
                .append(payload)
                .await
                .map_err(|error| error.to_string())?;
            current.boundaries.send_modify(|items| {
                let end = items.last().map_or(0.0, |(_, end)| *end) + duration;
                items.push((index, end));
            });
            // 首段就交给播放器，后续音频继续追加到同一条流。
            if let Some(source) = source {
                output.send(source).await.map_err(|_| "朗读播放器已关闭")?;
            }
            if profile.is_none() {
                let current = stream.take().unwrap();
                drop(current.writer);
                finishing = Some(current.played);
            }
        }
        if let Some(stream) = stream {
            stream.finish().await?;
        }
        if let Some(played) = finishing {
            played.await.map_err(|_| "朗读播放器已关闭")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Mp3StreamProfile {
    version: u32,
    sample_rate: usize,
    mono: bool,
}

/// 解析 MP3 帧的实际时长，拼流前移除每段自己的 ID3 和时长标记。
fn mp3_stream_payload(bytes: &[u8]) -> Result<(&[u8], f64, Mp3StreamProfile), ()> {
    let mut cursor = 0usize;
    if bytes.starts_with(b"ID3") {
        if bytes.len() < 10 || bytes[6..10].iter().any(|byte| byte & 0x80 != 0) {
            return Err(());
        }
        let size = ((bytes[6] as usize) << 21)
            | ((bytes[7] as usize) << 14)
            | ((bytes[8] as usize) << 7)
            | bytes[9] as usize;
        cursor = 10 + size + if bytes[5] & 0x10 != 0 { 10 } else { 0 };
        if cursor > bytes.len() {
            return Err(());
        }
    }
    let mut start = cursor;
    let mut duration = 0.0;
    let mut frames = 0;
    let mut profile = None;
    while cursor + 4 <= bytes.len() {
        let header = u32::from_be_bytes(bytes[cursor..cursor + 4].try_into().map_err(|_| ())?);
        if header >> 21 != 0x7ff {
            break;
        }
        let version = (header >> 19) & 3;
        let layer = (header >> 17) & 3;
        let bitrate_index = ((header >> 12) & 15) as usize;
        let sample_index = ((header >> 10) & 3) as usize;
        if version == 1
            || layer != 1
            || bitrate_index == 0
            || bitrate_index == 15
            || sample_index == 3
        {
            break;
        }
        let bitrate = if version == 3 {
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ][bitrate_index]
        } else {
            [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][bitrate_index]
        };
        let sample_rate = [44100, 48000, 32000][sample_index]
            / match version {
                3 => 1,
                2 => 2,
                _ => 4,
            };
        let frame_profile = Mp3StreamProfile {
            version,
            sample_rate,
            mono: (header >> 6) & 3 == 3,
        };
        if profile.is_some_and(|previous| previous != frame_profile) {
            return Err(());
        }
        profile = Some(frame_profile);
        let frame_size = (if version == 3 { 144_000 } else { 72_000 }) * bitrate / sample_rate
            + ((header >> 9) & 1) as usize;
        if cursor + frame_size > bytes.len() {
            break;
        }
        // Per-file Xing/Info/VBRI metadata advertises only that file's length.
        // Keeping it makes native players stop at the first generated segment.
        let side_info = if version == 3 {
            if frame_profile.mono {
                17
            } else {
                32
            }
        } else if frame_profile.mono {
            9
        } else {
            17
        };
        let crc = if (header >> 16) & 1 == 0 { 2 } else { 0 };
        let xing = cursor + 4 + crc + side_info;
        let vbri = cursor + 36;
        let metadata = frames == 0
            && (matches!(bytes.get(xing..xing + 4), Some(b"Xing" | b"Info"))
                || bytes.get(vbri..vbri + 4) == Some(b"VBRI"));
        cursor += frame_size;
        if metadata {
            start = cursor;
            continue;
        }
        duration += (if version == 3 { 1152.0 } else { 576.0 }) / sample_rate as f64;
        frames += 1;
    }
    if frames < 2 || bytes.len() - cursor > 256 {
        return Err(());
    }
    Ok((&bytes[start..cursor], duration, profile.ok_or(())?))
}

fn http_tts_audio_type(content_type: &str, bytes: &[u8]) -> Result<&'static str, String> {
    let content_type = content_type.trim().to_ascii_lowercase();
    match content_type.as_str() {
        "audio/mpeg" | "audio/mp3" | "audio/x-mpeg" | "audio/x-mp3" => Ok("audio/mpeg"),
        "audio/mp4" | "audio/x-m4a" => Ok("audio/mp4"),
        "audio/aac" | "audio/aacp" => Ok("audio/aac"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Ok("audio/wav"),
        "audio/ogg" => Ok("audio/ogg"),
        "audio/opus" => Ok("audio/ogg"),
        "audio/flac" | "audio/x-flac" => Ok("audio/flac"),
        "" | "application/octet-stream" => http_tts_audio_type_from_signature(bytes),
        _ => Err(format!(
            "Unsupported HTTP TTS audio content type: {content_type}"
        )),
    }
}

fn http_tts_audio_type_from_signature(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Ok("audio/wav")
    } else if bytes.starts_with(b"OggS") {
        Ok("audio/ogg")
    } else if bytes.starts_with(b"fLaC") {
        Ok("audio/flac")
    } else if bytes.starts_with(b"ID3") {
        Ok("audio/mpeg")
    } else if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        Ok("audio/mp4")
    } else if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xf6 == 0xf0 {
        Ok("audio/aac")
    } else if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0 {
        Ok("audio/mpeg")
    } else {
        Err("HTTP TTS returned an unsupported audio signature".to_owned())
    }
}
