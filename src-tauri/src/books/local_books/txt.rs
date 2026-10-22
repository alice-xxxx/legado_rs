//! TXT 字符集识别、分章和文本章节生成。

use std::{path::Path, sync::OnceLock};

use encoding_rs::{Encoding, GBK, UTF_16BE, UTF_16LE};
use regex::Regex;

use super::{txt_toc_rules, LocalImportOptions, ParsedBook, ParsedChapter, MAX_EPUB_CHAPTERS};

pub(super) const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

pub(super) fn parse_text_file(
    path: &Path,
    bytes: &[u8],
    options: &LocalImportOptions,
    saved_toc_rules: Option<&txt_toc_rules::TxtTocRulesDocument>,
) -> Result<ParsedBook, String> {
    let decoded = decode_text(&bytes, options.charset.as_deref())?;
    let text = decoded.replace("\r\n", "\n").replace('\r', "\n");
    let headings = match options.toc_regex.as_deref() {
        Some(pattern) => {
            if pattern.len() > 4096 {
                return Err("TXT TOC expression is too long".to_owned());
            }
            let expression = txt_toc_rules::compile_pattern(pattern)?;
            txt_toc_rules::pattern_headings(&text, &expression)?
        }
        None => {
            let rules = saved_toc_rules
                .map(|document| document.rules.as_slice())
                .unwrap_or_default();
            let (has_enabled_rules, selected) = txt_toc_rules::best_rule_headings(&text, rules)?;
            match (has_enabled_rules, selected) {
                (true, selected) => selected.unwrap_or_default(),
                (false, _) if rules.is_empty() => {
                    txt_toc_rules::pattern_headings(&text, default_toc_regex())?
                }
                (false, _) => Vec::new(),
            }
        }
    };

    let mut chapters = Vec::new();
    if let Some(first) = headings.first() {
        let preface = text[..first.line_start].trim();
        if !preface.is_empty() {
            push_text_section(&mut chapters, "序章", preface, 10 * 1024);
        }
        for (index, heading) in headings.iter().enumerate() {
            let body_end = headings
                .get(index + 1)
                .map_or(text.len(), |next| next.line_start);
            let chapter_text = text[heading.body_start..body_end].trim();
            if !chapter_text.is_empty() {
                push_text_section(&mut chapters, &heading.title, chapter_text, 100 * 1024);
            }
        }
    } else {
        push_text_section(&mut chapters, "正文", text.trim(), 10 * 1024);
    }
    if chapters.len() > MAX_EPUB_CHAPTERS {
        return Err(format!(
            "TXT contains more than {} chapters",
            MAX_EPUB_CHAPTERS
        ));
    }
    if chapters.is_empty() {
        return Err("The selected TXT file has no readable text".to_owned());
    }

    let filename = file_stem(path);
    let metadata = Regex::new(r"^《(.+)》\s*作者[:：]\s*(.+)$").expect("filename metadata pattern");
    let (title, author) = metadata
        .captures(&filename)
        .map(|parts| (parts[1].trim().to_owned(), parts[2].trim().to_owned()))
        .unwrap_or((filename, String::new()));
    Ok(ParsedBook {
        title,
        author,
        intro: None,
        chapters,
        assets: Vec::new(),
        cover_asset_id: None,
    })
}

fn push_text_section(chapters: &mut Vec<ParsedChapter>, title: &str, text: &str, max_chars: usize) {
    let mut remaining = text.trim();
    let mut part = 1;
    while !remaining.is_empty() {
        let end = remaining
            .char_indices()
            .nth(max_chars)
            .map_or(remaining.len(), |(index, _)| index);
        let end = if end < remaining.len() {
            remaining[..end]
                .rfind('\n')
                .filter(|index| *index > end / 2)
                .map_or(end, |index| index + 1)
        } else {
            end
        };
        let body = remaining[..end].trim();
        if !body.is_empty() {
            chapters.push(ParsedChapter {
                title: if part == 1 {
                    title.to_owned()
                } else {
                    format!("{title}（{part}）")
                },
                html: None,
                text: Some(body.to_owned()),
            });
            part += 1;
        }
        remaining = remaining[end..].trim_start();
    }
}

fn decode_text(bytes: &[u8], charset: Option<&str>) -> Result<String, String> {
    if let Some(label) = charset {
        let encoding = Encoding::for_label(label.trim().as_bytes())
            .ok_or_else(|| format!("Unsupported text encoding label: {label}"))?;
        let bytes = strip_utf8_bom(bytes);
        let (decoded, _, had_errors) = encoding.decode(bytes);
        if had_errors {
            return Err(format!("TXT data is not valid {label}"));
        }
        return Ok(decoded.into_owned());
    }

    if let Some((encoding, length)) = Encoding::for_bom(bytes) {
        if encoding == UTF_16LE || encoding == UTF_16BE {
            let (decoded, had_errors) = encoding.decode_without_bom_handling(&bytes[length..]);
            if had_errors {
                return Err("TXT contains invalid UTF-16 text".to_owned());
            }
            return Ok(decoded.into_owned());
        }
    }

    let bytes = strip_utf8_bom(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }
    let (decoded, _, had_errors) = GBK.decode(bytes);
    if had_errors {
        return Err(
            "TXT encoding is neither valid UTF-8 nor GBK; select a charset explicitly".to_owned(),
        );
    }
    Ok(decoded.into_owned())
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
}

pub(super) fn default_toc_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        // Requiring a separator after the ordinal avoids treating prose such
        // as “第一章正文。” as another empty chapter. A bare ordinal line is
        // still a valid heading.
        Regex::new(r"(?i)^\s*((?:第[零〇一二三四五六七八九十百千万两\d]+[章节回卷集部](?:[ \t]+[^\r\n]{1,80}|[:：、.．-][ \t]*[^\r\n]{1,80})?|chapter\s+\d+(?:[ \t]+[^\r\n]{1,80}|[:.-][ \t]*[^\r\n]{1,80})?))\s*$")
            .expect("static TXT TOC pattern is valid")
    })
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("本地书籍")
        .chars()
        .take(200)
        .collect()
}
