//! 阅读器设置与默认值。
use crate::resources::ResourceRef;
use serde::{Deserialize, Serialize};
/// 阅读与播放设置；Vue 按这些选项显示正文和控制播放器。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReaderDefaults {
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size_px")]
    pub font_size_px: f32,
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    /// 背景图片保存稳定引用，提供给界面时转换为本地 HTTP 地址。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image_src: Option<ResourceRef>,
    pub background_image_opacity: u8,
    #[serde(default = "default_text_align")]
    pub text_align: String,
    pub themes: ReaderThemes,
    #[serde(default = "default_preload_count")]
    pub preload_count: usize,
    pub reading_mode: ReadingMode,
    pub audio_playback_rate: f32,
    pub audio_skip_seconds: u64,
    pub video_playback_rate: f32,
    pub tts_engine: String,
    #[serde(default = "default_tts_rate")]
    pub tts_rate: f32,
    #[serde(rename = "ttsVoiceURI")]
    pub tts_voice_uri: String,
    pub http_tts_config_id: Option<String>,
    pub tts_continue_across_chapters: bool,
    pub tts_start_position: String,
}

/// 阅读布局和翻页效果。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingMode {
    None,
    #[default]
    Cover,
    Slide,
    #[serde(rename = "slide-vertical")]
    SlideVertical,
    Fade,
    Flip,
    Simulation,
    Vertical,
}

/// 阅读主题。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReaderTheme {
    #[default]
    Paper,
    Sepia,
    Dark,
    White,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ReaderThemes {
    pub light: ReaderTheme,
    pub dark: ReaderTheme,
}

impl Default for ReaderThemes {
    fn default() -> Self {
        Self {
            light: ReaderTheme::Paper,
            dark: ReaderTheme::Dark,
        }
    }
}

impl Default for ReaderDefaults {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size_px: default_font_size_px(),
            line_height: default_line_height(),
            background_image_src: None,
            background_image_opacity: 20,
            text_align: default_text_align(),
            themes: ReaderThemes::default(),
            preload_count: default_preload_count(),
            reading_mode: ReadingMode::default(),
            audio_playback_rate: 1.0,
            audio_skip_seconds: 15,
            video_playback_rate: 1.0,
            tts_engine: "system".to_owned(),
            tts_rate: default_tts_rate(),
            tts_voice_uri: String::new(),
            http_tts_config_id: None,
            tts_continue_across_chapters: true,
            tts_start_position: "current".to_owned(),
        }
    }
}

fn default_font_family() -> String {
    "serif".to_owned()
}

fn default_font_size_px() -> f32 {
    19.0
}

fn default_line_height() -> f32 {
    1.8
}

fn default_tts_rate() -> f32 {
    1.0
}

fn default_text_align() -> String {
    "justify".to_owned()
}

fn default_preload_count() -> usize {
    5
}
