//! EPUB 3 navigation / EPUB 2 NCX boundaries in spine reading order.
use std::collections::HashMap;

use super::{find_child, find_nodes, normalize_epub_href, parse_xml_tree, XmlNode};

pub(super) struct TocEntry {
    pub title: String,
    pub path: String,
    pub fragment: String,
}

pub(super) struct SpineDocument {
    pub path: String,
    pub title: String,
    pub body: String,
    pub styles: String,
}

pub(super) struct ChapterSlice {
    pub title: String,
    pub start: (usize, usize),
    pub end: (usize, usize),
}

fn node_text(node: &XmlNode) -> String {
    node.text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn parse_toc(xml: &str, path: &str, ncx: bool) -> Result<Vec<TocEntry>, String> {
    let roots = parse_xml_tree(xml)?;
    let links = if ncx {
        find_nodes(&roots, "navPoint")
            .into_iter()
            .filter_map(|point| {
                Some((
                    node_text(find_child(point, "navLabel")?),
                    find_child(point, "content")?.attributes.get("src")?.clone(),
                ))
            })
            .collect::<Vec<_>>()
    } else {
        let navs = find_nodes(&roots, "nav");
        let nav = navs
            .iter()
            .find(|nav| {
                nav.attributes
                    .get("type")
                    .is_some_and(|types| types.split_whitespace().any(|kind| kind == "toc"))
            })
            .or_else(|| {
                navs.iter().find(|nav| {
                    nav.attributes
                        .get("role")
                        .is_some_and(|role| role == "doc-toc")
                })
            });
        nav.map(|nav| {
            find_nodes(&nav.children, "a")
                .into_iter()
                .filter_map(|link| Some((node_text(link), link.attributes.get("href")?.clone())))
                .collect()
        })
        .unwrap_or_default()
    };
    Ok(links
        .into_iter()
        .filter_map(|(title, href)| {
            if title.is_empty() {
                return None;
            }
            let fragment = percent_encoding::percent_decode_str(
                href.split_once('#').map_or("", |(_, anchor)| anchor),
            )
            .decode_utf8()
            .ok()?
            .into_owned();
            Some(TocEntry {
                title: title.chars().take(200).collect(),
                path: normalize_epub_href(path, &href)?,
                fragment,
            })
        })
        .collect())
}

pub(super) fn document_parts(html: &str) -> (String, String) {
    let body = regex::Regex::new(r"(?is)<body\b[^>]*>(.*?)</body\s*>").expect("body pattern");
    let head = regex::Regex::new(r"(?is)<head\b[^>]*>(.*?)</head\s*>").expect("head pattern");
    let styles = regex::Regex::new(r"(?is)<link\b[^>]*>|<style\b[^>]*>.*?</style\s*>")
        .expect("styles pattern");
    let stylesheet = head
        .captures(html)
        .map(|head| {
            styles
                .find_iter(&head[1])
                .map(|m| m.as_str())
                .collect::<String>()
        })
        .unwrap_or_default();
    let content = body
        .captures(html)
        .map(|body| body[1].to_owned())
        .unwrap_or_else(|| html.to_owned());
    (content, stylesheet)
}

fn fragment_offset(body: &str, fragment: &str) -> Option<usize> {
    if fragment.is_empty() {
        return Some(0);
    }
    let tags =
        regex::Regex::new(r#"(?is)<[a-z][^>]*\s(?:id|name)\s*=\s*(?:"([^"]*)"|'([^']*)')[^>]*>"#)
            .expect("anchor pattern");
    let offset = tags.captures_iter(body).find_map(|tag| {
        let id = tag.get(1).or_else(|| tag.get(2))?.as_str();
        (quick_xml::escape::unescape(id).ok()?.as_ref() == fragment)
            .then(|| tag.get(0).unwrap().start())
    });
    offset
}

pub(super) fn chapter_slices(
    documents: &[SpineDocument],
    toc: &[TocEntry],
) -> (Vec<ChapterSlice>, HashMap<String, String>) {
    let mut boundaries = Vec::new();
    for entry in toc {
        let Some(index) = documents
            .iter()
            .position(|document| document.path == entry.path)
        else {
            continue;
        };
        let Some(offset) = fragment_offset(&documents[index].body, &entry.fragment) else {
            continue;
        };
        let position = (index, offset);
        if boundaries
            .last()
            .is_some_and(|(last, _, _)| *last > position)
        {
            boundaries.clear();
            break;
        }
        if !boundaries
            .iter()
            .any(|(existing, _, _)| *existing == position)
        {
            boundaries.push((
                position,
                entry.title.clone(),
                format!("{}#{}", entry.path, entry.fragment),
            ));
        }
    }
    if boundaries.is_empty() {
        boundaries = documents
            .iter()
            .enumerate()
            .map(|(index, document)| {
                (
                    (index, 0),
                    document.title.clone(),
                    format!("{}#", document.path),
                )
            })
            .collect();
    } else if boundaries[0].0 .0 > 0
        || (boundaries[0].0 .1 > 0 && {
            let prefix = &documents[0].body[..boundaries[0].0 .1];
            let text = regex::Regex::new(r"(?s)<[^>]*>")
                .expect("tags pattern")
                .replace_all(prefix, "");
            !text.trim().is_empty() || prefix.contains("<img") || prefix.contains("<svg")
        })
    {
        boundaries.insert(
            0,
            (
                (0, 0),
                documents[0].title.clone(),
                format!("{}#", documents[0].path),
            ),
        );
    }
    let last = documents.len() - 1;
    let mut slices = Vec::new();
    let mut targets = HashMap::new();
    for (index, (start, title, target)) in boundaries.iter().enumerate() {
        let id = format!("chapter-{:05}", index + 1);
        targets.insert(target.clone(), id.clone());
        let end = boundaries
            .get(index + 1)
            .map_or((last, documents[last].body.len()), |next| next.0);
        let end_document = if end.1 == 0 && end.0 > start.0 {
            end.0 - 1
        } else {
            end.0
        };
        for document in &documents[start.0..=end_document] {
            targets
                .entry(document.path.clone())
                .or_insert_with(|| id.clone());
        }
        slices.push(ChapterSlice {
            title: title.clone(),
            start: *start,
            end,
        });
    }
    (slices, targets)
}
