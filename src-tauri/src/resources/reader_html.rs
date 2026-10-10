//! 生成章节 HTML，并净化阅读样式。

use crate::models::ReaderDefaults;

pub(super) fn chapter_document(content: &str, defaults: &ReaderDefaults) -> String {
    let style = safe_reader_style(defaults);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>:root{{--reader-font-family:{font_family};--reader-font-size:{font_size}px;--reader-line-height:{line_height};--reader-text-color:{text_color};--reader-background-color:{background_color};--reader-text-align:{text_align}}}html,body{{margin:0;min-height:100%;background:var(--reader-background-color);color:var(--reader-text-color);font-family:var(--reader-font-family);font-size:var(--reader-font-size);line-height:var(--reader-line-height)}}.chapter-content{{text-align:var(--reader-text-align);overflow-wrap:anywhere}}.chapter-content img{{max-width:100%;height:auto}}.chapter-content p{{margin:0 0 1em}}.audio-chapter{{display:grid;min-height:35vh;place-items:center;padding:1.5rem}}.audio-chapter audio{{display:block;width:min(100%,46rem)}}.video-chapter{{display:grid;min-height:45vh;place-items:center;padding:1rem}}.video-chapter video{{display:block;width:100%;max-height:75vh;background:#000}}</style></head><body><article class=\"chapter-content\">{content}</article></body></html>",
        font_family = style.font_family,
        font_size = style.font_size_px,
        line_height = style.line_height,
        text_color = style.text_color,
        background_color = style.background_color,
        text_align = style.text_align,
    )
}

struct SafeReaderStyle {
    font_family: String,
    font_size_px: f32,
    line_height: f32,
    text_color: String,
    background_color: String,
    text_align: String,
}

fn safe_reader_style(defaults: &ReaderDefaults) -> SafeReaderStyle {
    let font_family = if !defaults.font_family.is_empty()
        && defaults.font_family.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, ' ' | ',' | '-' | '_' | '\'' | '"')
        }) {
        defaults.font_family.clone()
    } else {
        ReaderDefaults::default().font_family
    };
    SafeReaderStyle {
        font_family,
        font_size_px: if defaults.font_size_px.is_finite() {
            defaults.font_size_px.clamp(8.0, 72.0)
        } else {
            ReaderDefaults::default().font_size_px
        },
        line_height: if defaults.line_height.is_finite() {
            defaults.line_height.clamp(1.0, 3.0)
        } else {
            ReaderDefaults::default().line_height
        },
        text_color: safe_color(&defaults.text_color, "#3f3b34"),
        background_color: safe_color(&defaults.background_color, "#f7f3e9"),
        text_align: match defaults.text_align.as_str() {
            "left" | "right" | "center" | "justify" | "start" | "end" => {
                defaults.text_align.clone()
            }
            _ => "justify".to_owned(),
        },
    }
}

fn safe_color(color: &str, fallback: &str) -> String {
    let hex = color.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return format!("#{hex}");
    }
    let lower = color.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "black"
            | "white"
            | "red"
            | "green"
            | "blue"
            | "gray"
            | "grey"
            | "transparent"
            | "currentcolor"
            | "navy"
            | "teal"
            | "olive"
            | "maroon"
            | "purple"
            | "silver"
            | "orange"
            | "yellow"
    ) {
        return lower;
    }
    fallback.to_owned()
}
