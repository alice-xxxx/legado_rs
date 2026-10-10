//! EPUB HTML、SVG 和 CSS 的本地资源链接重写。

use std::{collections::HashMap, path::Path, sync::OnceLock};

use regex::Regex;

use super::epub::normalize_epub_href;
pub(super) fn rewrite_epub_html(
    html: &str,
    html_path: &str,
    assets: &HashMap<String, String>,
    chapters: &HashMap<String, String>,
) -> String {
    let mut output = rewrite_html_attr(html, html_path, assets, chapters, false, true);
    output = rewrite_html_attr(&output, html_path, assets, chapters, true, true);
    // srcset is deliberately removed until every candidate can be mapped to a
    // verified local resource. This avoids leaking remote/data URLs to WebView.
    output = srcset_double_regex().replace_all(&output, "").into_owned();
    srcset_single_regex().replace_all(&output, "").into_owned()
}

pub(super) fn rewrite_epub_svg(
    svg: &str,
    svg_path: &str,
    assets: &HashMap<String, String>,
) -> String {
    let no_chapters = HashMap::new();
    let mut output = rewrite_html_attr(svg, svg_path, assets, &no_chapters, false, false);
    output = rewrite_html_attr(&output, svg_path, assets, &no_chapters, true, false);
    output = rewrite_svg_style_attr(&output, svg_path, assets, false);
    output = rewrite_svg_style_attr(&output, svg_path, assets, true);
    svg_style_element_regex()
        .replace_all(&output, |captures: &regex::Captures<'_>| {
            let tag = captures.get(1).map_or("<style>", |value| value.as_str());
            let css = captures.get(2).map_or("", |value| value.as_str());
            format!("{tag}{}</style>", rewrite_css(css, svg_path, assets))
        })
        .into_owned()
}

fn rewrite_html_attr(
    html: &str,
    html_path: &str,
    assets: &HashMap<String, String>,
    chapters: &HashMap<String, String>,
    single_quote: bool,
    allow_external_links: bool,
) -> String {
    let regex = if single_quote {
        html_attr_single_regex()
    } else {
        html_attr_double_regex()
    };
    regex
        .replace_all(html, |captures: &regex::Captures<'_>| {
            let name = captures
                .get(1)
                .map_or("", |value| value.as_str())
                .to_ascii_lowercase();
            let value = captures.get(3).map_or("", |value| value.as_str()).trim();
            if value.starts_with('#') {
                return captures[0].to_owned();
            }
            let tag_start = html[..captures.get_match().start()].rfind('<');
            let tag = tag_start
                .map(|start| &html[start..captures.get_match().start()])
                .unwrap_or_default()
                .trim_start()
                .to_ascii_lowercase();
            let is_anchor = tag.starts_with("<a ") || tag == "<a";
            let allow_external =
                allow_external_links && name == "href" && is_anchor && is_safe_external_link(value);
            // EPUB chapter links are application navigation, not resource
            // paths. Keep the target as typed metadata so Vue can express the
            // navigation intent without knowing the chapter cache layout.
            let raw_fragment = value
                .split_once('#')
                .map(|(_, fragment)| fragment)
                .unwrap_or_default();
            if name == "href" && is_anchor {
                if let Some(path) = normalize_epub_href(html_path, value) {
                    if let Some(chapter_id) = chapters.get(&path) {
                        let href = if raw_fragment.is_empty() {
                            "#".to_owned()
                        } else {
                            format!("#{raw_fragment}")
                        };
                        let prefix = captures.get(2).map_or("", |value| value.as_str());
                        let quote = if single_quote { '\'' } else { '"' };
                        return format!(
                            "{name}{prefix}{}{quote} data-legado-chapter-id={quote}{}{quote} data-legado-fragment={quote}{}{quote}",
                            escape_html_attr(&href),
                            escape_html_attr(chapter_id),
                            escape_html_attr(raw_fragment),
                        );
                    }
                }
            }

            let fragment = if raw_fragment.is_empty() {
                String::new()
            } else {
                format!("#{raw_fragment}")
            };
            let replacement = if allow_external {
                Some(value.to_owned())
            } else if let Some(path) = normalize_epub_href(html_path, value) {
                assets
                    .get(&path)
                    .map(|asset_id| format!("../assets/{asset_id}{fragment}"))
            } else {
                None
            };
            let Some(replacement) = replacement else {
                return String::new();
            };
            let prefix = captures.get(2).map_or("", |value| value.as_str());
            let quote = if single_quote { '\'' } else { '"' };
            format!("{name}{prefix}{}{quote}", escape_html_attr(&replacement))
        })
        .into_owned()
}

fn is_safe_external_link(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

fn rewrite_svg_style_attr(
    svg: &str,
    svg_path: &str,
    assets: &HashMap<String, String>,
    single_quote: bool,
) -> String {
    let regex = if single_quote {
        svg_style_attr_single_regex()
    } else {
        svg_style_attr_double_regex()
    };
    regex
        .replace_all(svg, |captures: &regex::Captures<'_>| {
            let name = captures.get(1).map_or("style", |value| value.as_str());
            let prefix = captures.get(2).map_or("=\"", |value| value.as_str());
            let css = captures.get(3).map_or("", |value| value.as_str());
            let quote = if single_quote { '\'' } else { '"' };
            format!(
                "{name}{prefix}{}{quote}",
                rewrite_css(css, svg_path, assets)
            )
        })
        .into_owned()
}

pub(super) fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
}

pub(super) fn rewrite_css(css: &str, css_path: &str, assets: &HashMap<String, String>) -> String {
    let comment_free = css_comment_regex().replace_all(css, "");
    let lowered = comment_free.to_ascii_lowercase();
    if comment_free.contains('\\')
        || [
            "expression(",
            "-moz-binding",
            "behavior:",
            "image-set(",
            "-webkit-image-set(",
        ]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return String::new();
    }
    let rewritten_urls = css_url_regex()
        .replace_all(&comment_free, |captures: &regex::Captures<'_>| {
            let href = captures
                .get(1)
                .or_else(|| captures.get(2))
                .or_else(|| captures.get(3))
                .map_or("", |value| value.as_str())
                .trim();
            if href.starts_with('#') {
                return format!("url(\"{}\")", escape_css_string(href));
            }
            normalize_epub_href(css_path, href)
                .and_then(|path| assets.get(&path))
                .map(|asset| {
                    let fragment = href
                        .split_once('#')
                        .map(|(_, fragment)| format!("#{fragment}"))
                        .unwrap_or_default();
                    format!(
                        "url(\"{}{}\")",
                        escape_css_string(asset),
                        escape_css_string(&fragment)
                    )
                })
                .unwrap_or_else(|| "url(\"\")".to_owned())
        })
        .into_owned();
    let imports = css_import_regex()
        .replace_all(&rewritten_urls, |captures: &regex::Captures<'_>| {
            let href = captures
                .get(1)
                .or_else(|| captures.get(2))
                .map_or("", |value| value.as_str());
            normalize_epub_href(css_path, href)
                .and_then(|path| assets.get(&path))
                .map(|asset| format!("@import url(\"{asset}\")"))
                .unwrap_or_default()
        })
        .into_owned();
    css_empty_import_regex()
        .replace_all(&imports, "")
        .into_owned()
}

fn escape_css_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "")
}

pub(super) fn chapter_title(html: &str, fallback: &str, index: usize) -> String {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    let regex = TITLE.get_or_init(|| {
        Regex::new(r"(?is)<(?:title|h1|h2)[^>]*>(.*?)</(?:title|h1|h2)\s*>").unwrap()
    });
    if let Some(captures) = regex.captures(html) {
        let title = captures.get(1).map_or("", |capture| capture.as_str());
        let plain = strip_html_tags(title).trim().to_owned();
        if !plain.is_empty() {
            return plain.chars().take(200).collect();
        }
    }
    Path::new(fallback)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("第{}章", index + 1))
}

fn strip_html_tags(input: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    TAG.get_or_init(|| Regex::new(r"(?s)<[^>]*>").unwrap())
        .replace_all(input, " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn html_attr_double_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?is)\b(src|href|xlink:href|poster)(\s*=\s*")([^"]*)""#).unwrap()
    })
}

fn html_attr_single_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)\b(src|href|xlink:href|poster)(\s*=\s*')([^']*)'").unwrap()
    })
}

fn svg_style_attr_double_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"(?is)\b(style)(\s*=\s*")([^"]*)""#).unwrap())
}

fn svg_style_attr_single_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)\b(style)(\s*=\s*')([^']*)'").unwrap())
}

fn srcset_double_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"(?is)\s+srcset\s*=\s*"[^"]*""#).unwrap())
}

fn srcset_single_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)\s+srcset\s*=\s*'[^']*'").unwrap())
}

fn css_url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?is)url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*?))\s*\)"#).unwrap()
    })
}

fn css_import_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"(?is)@import\s+(?:"([^"]+)"|'([^']+)')\s*;?"#).unwrap())
}

fn css_empty_import_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"(?is)@import\s+url\(\s*["']?["']?\s*\)\s*;?"#).unwrap())
}

fn css_comment_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?s)/\*.*?\*/").unwrap())
}

pub(super) fn inline_style_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<style(?:\s[^>]*)?>(.*?)</style\s*>").unwrap())
}

fn svg_style_element_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)(<style(?:\s[^>]*)?>)(.*?)</style\s*>").unwrap())
}
