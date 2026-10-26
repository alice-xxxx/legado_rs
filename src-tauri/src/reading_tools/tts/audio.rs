//! 各种输入音频只解码一次，整个会话共用一个 MP3 编码器，不在片段间插入文件头或编码器间隙。
use mp3lame_encoder::{Bitrate, Builder, DualPcm, Encoder, FlushGap, Quality};
use std::{io::Cursor, num::NonZeroU32, sync::OnceLock};
use symphonium::{symphonia, DecodeConfig, DecodedAudioF32};

pub(super) const SAMPLE_RATE: u32 = 44_100;

pub(super) fn decode(bytes: Vec<u8>) -> Result<DecodedAudioF32, String> {
    static CODECS: OnceLock<symphonia::core::codecs::registry::CodecRegistry> = OnceLock::new();
    let codecs = CODECS.get_or_init(|| {
        let mut codecs = symphonia::core::codecs::registry::CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut codecs);
        codecs.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        codecs
    });
    let source = symphonium::probe_from_source(Box::new(Cursor::new(bytes)), None, None)
        .map_err(|error| error.to_string())?;
    symphonium::decode_f32(
        source,
        &DecodeConfig {
            max_bytes: 64 * 1024 * 1024,
            ..Default::default()
        },
        NonZeroU32::new(SAMPLE_RATE),
        None,
        Some(codecs),
    )
    .map_err(|error| error.to_string())
}

pub(super) fn encoder() -> Result<Encoder, String> {
    Builder::new()
        .ok_or_else(|| "无法初始化朗读音频编码器".to_owned())?
        .with_num_channels(2)
        .map_err(|error| error.to_string())?
        .with_sample_rate(SAMPLE_RATE)
        .map_err(|error| error.to_string())?
        .with_brate(Bitrate::Kbps128)
        .map_err(|error| error.to_string())?
        .with_quality(Quality::Best)
        .map_err(|error| error.to_string())?
        // 流没有最终长度，不能写入单个文件的时长标记。
        .with_to_write_vbr_tag(false)
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())
}

pub(super) async fn append(
    mut encoder: Encoder,
    audio: DecodedAudioF32,
) -> Result<(Encoder, Vec<u8>), String> {
    tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        let left = audio.data.first().ok_or("朗读音频没有声道")?;
        let right = audio.data.get(1).unwrap_or(left);
        bytes.reserve(mp3lame_encoder::max_required_buffer_size(left.len()));
        encoder
            .encode_to_vec(DualPcm { left, right }, &mut bytes)
            .map_err(|error| error.to_string())?;
        Ok((encoder, bytes))
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(super) fn finish(encoder: &mut Encoder) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(7200);
    encoder
        .flush_to_vec::<FlushGap>(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}
