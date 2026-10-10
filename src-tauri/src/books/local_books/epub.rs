//! EPUB 容器、OPF 清单、XML 与 spine 章节解析。

use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
};

use quick_xml::{Reader, events::Event};
use zip::ZipArchive;

use super::{
    MAX_EPUB_CHAPTERS, ParsedAsset, ParsedBook, ParsedChapter,
    archive::{index_zip_members, read_archive_member},
    common::short_hash,
    rewrite::{
        chapter_title, inline_style_regex, rewrite_css, rewrite_epub_html, rewrite_epub_svg,
    },
};

pub(super) const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EPUB_EXPANDED_BYTES: u64 = 768 * 1024 * 1024;
const MAX_EPUB_FILES: usize = 100_000;
const MAX_EPUB_XML_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EPUB_ASSET_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Debug)]
struct ManifestItem {
    href: String,
    media_type: String,
}

#[derive(Debug)]
struct EpubPackage {
    title: String,
    author: String,
    opf_path: String,
    manifest: HashMap<String, ManifestItem>,
    spine: Vec<String>,
    cover_id: Option<String>,
}

pub(super) fn parse_epub_file(bytes: &[u8]) -> Result<ParsedBook, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "The selected EPUB is not a valid ZIP archive".to_owned())?;
    let members = index_epub_members(&mut archive)?;
    if archive.len() == 0
        || archive
            .by_index(0)
            .map(|entry| {
                entry.name() != "mimetype" || entry.compression() != zip::CompressionMethod::Stored
            })
            .unwrap_or(true)
    {
        return Err("EPUB requires an uncompressed mimetype entry first in the archive".to_owned());
    }
    let mime_type = read_epub_member(&mut archive, &members, "mimetype", 64)?;
    if mime_type.as_slice() != b"application/epub+zip" {
        return Err("The selected ZIP file does not declare the EPUB media type".to_owned());
    }
    let container = read_epub_member(
        &mut archive,
        &members,
        "META-INF/container.xml",
        MAX_EPUB_XML_BYTES,
    )?;
    let container = std::str::from_utf8(&container)
        .map_err(|_| "EPUB container.xml must be UTF-8".to_owned())?;
    let container = parse_xml_tree(container)?;
    let package_href = find_nodes(&container, "rootfile")
        .into_iter()
        .find_map(|node| node.attributes.get("full-path").cloned())
        .ok_or_else(|| "EPUB container.xml does not declare an OPF package".to_owned())?;
    let opf_path = normalize_epub_href("", &package_href)
        .ok_or_else(|| "EPUB package path escapes the archive".to_owned())?;
    let opf_bytes = read_epub_member(&mut archive, &members, &opf_path, MAX_EPUB_XML_BYTES)?;
    let opf_text =
        std::str::from_utf8(&opf_bytes).map_err(|_| "EPUB OPF package must be UTF-8".to_owned())?;
    let package = parse_opf(opf_text, opf_path)?;

    let mut spine_items = Vec::new();
    for id in &package.spine {
        let item = package
            .manifest
            .get(id)
            .ok_or_else(|| format!("EPUB spine refers to missing manifest item '{id}'"))?;
        let path = normalize_epub_href(&package.opf_path, &item.href)
            .ok_or_else(|| format!("Invalid EPUB chapter path for item '{id}'"))?;
        if !members.contains_key(&path) {
            return Err(format!("EPUB chapter file for item '{id}' is missing"));
        }
        if is_chapter_type(&item.media_type) {
            spine_items.push((id.clone(), path));
        }
    }
    if spine_items.is_empty() {
        return Err("EPUB package contains no readable XHTML spine items".to_owned());
    }
    if spine_items.len() > MAX_EPUB_CHAPTERS {
        return Err("EPUB package has too many chapters".to_owned());
    }

    // Store all safe, browser-consumable assets under flat hashed names. The
    // original archive paths are kept only in this private import operation.
    let mut asset_ids = HashMap::new();
    let mut assets = Vec::new();
    for name in members.keys() {
        if !is_importable_asset(name) {
            continue;
        }
        let index = members[name];
        let declared_size = archive
            .by_index(index)
            .map_err(|_| "Cannot inspect EPUB media entry".to_owned())?
            .size();
        if declared_size > MAX_EPUB_ASSET_BYTES {
            return Err("EPUB media entry exceeds the supported size limit".to_owned());
        }
        let asset_id = asset_id(name);
        let content = read_epub_member(&mut archive, &members, name, MAX_EPUB_ASSET_BYTES)?;
        asset_ids.insert(name.clone(), asset_id.clone());
        assets.push(ParsedAsset {
            id: asset_id,
            bytes: content,
        });
    }

    let path_to_chapter = spine_items
        .iter()
        .enumerate()
        .map(|(index, (_, path))| (path.clone(), format!("chapter-{:05}", index + 1)))
        .collect::<HashMap<_, _>>();

    // EPUB resources can reference images from linked CSS. Rewrite all local
    // URL references to the same browser-readable asset directory.
    let mut rewritten_assets = Vec::with_capacity(assets.len());
    for asset in assets {
        let original_path = asset_ids
            .iter()
            .find_map(|(path, id)| (id == &asset.id).then_some(path.clone()));
        if let Some(css_path) = original_path.as_deref().filter(|path| is_css_path(path)) {
            let css = String::from_utf8_lossy(&asset.bytes);
            let rewritten = rewrite_css(&css, &css_path, &asset_ids);
            rewritten_assets.push(ParsedAsset {
                id: asset.id,
                bytes: rewritten.into_bytes(),
            });
        } else if let Some(svg_path) = original_path.as_deref().filter(|path| is_svg_path(path)) {
            let svg = String::from_utf8_lossy(&asset.bytes);
            let rewritten = rewrite_epub_svg(&svg, &svg_path, &asset_ids);
            rewritten_assets.push(ParsedAsset {
                id: asset.id,
                bytes: rewritten.into_bytes(),
            });
        } else {
            rewritten_assets.push(asset);
        }
    }

    let inline_css = inline_style_regex();
    let mut chapters = Vec::with_capacity(spine_items.len());
    for (index, (id, html_path)) in spine_items.into_iter().enumerate() {
        let item = &package.manifest[&id];
        let html_bytes = read_epub_member(&mut archive, &members, &html_path, MAX_EPUB_XML_BYTES)?;
        let html = String::from_utf8_lossy(&html_bytes).into_owned();
        let mut links = String::new();
        let html = inline_css
            .replace_all(&html, |captures: &regex::Captures<'_>| {
                let css = rewrite_css(
                    captures.get(1).map_or("", |value| value.as_str()),
                    &html_path,
                    &asset_ids,
                );
                if css.trim().is_empty() {
                    String::new()
                } else {
                    let id = format!(
                        "inline-{:05}-{}.css",
                        index + 1,
                        short_hash(&format!("{}\0{}", html_path, links.len()))
                    );
                    // Each inline style becomes a normal local stylesheet so the
                    // HTML sanitiser can discard inline style attributes.
                    rewritten_assets.push(ParsedAsset {
                        id: id.clone(),
                        bytes: css.into_bytes(),
                    });
                    links.push_str(&format!(r#"<link rel="stylesheet" href="../assets/{id}">"#));
                    String::new()
                }
            })
            .into_owned();
        let html = rewrite_epub_html(&html, &html_path, &asset_ids, &path_to_chapter);
        let html = format!("{links}{html}");
        chapters.push(ParsedChapter {
            title: chapter_title(&html, &item.href, index),
            html: Some(html),
            text: None,
            pdf_page_index: None,
        });
    }

    let cover_asset_id = package.cover_id.and_then(|id| {
        let item = package.manifest.get(&id)?;
        let path = normalize_epub_href(&package.opf_path, &item.href)?;
        asset_ids.get(&path).cloned()
    });
    Ok(ParsedBook {
        identity: String::new(),
        title: package.title.trim().chars().take(200).collect(),
        author: package.author.trim().chars().take(200).collect(),
        chapters,
        assets: rewritten_assets,
        cover_asset_id,
        pdf_asset_id: None,
    })
}

fn index_epub_members<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<HashMap<String, usize>, String> {
    index_zip_members(archive, MAX_EPUB_FILES, MAX_EPUB_EXPANDED_BYTES, "EPUB")
}

pub(super) fn normalize_epub_href(base_file: &str, href: &str) -> Option<String> {
    if href.is_empty() || href.starts_with('/') || href.starts_with("//") || href.contains('\\') {
        return None;
    }
    let raw_path = href.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .ok()?;
    let path = decoded.as_ref();
    if path.is_empty() || path.contains(':') {
        return None;
    }
    let mut components = if base_file.is_empty() {
        Vec::new()
    } else {
        base_file.split('/').collect::<Vec<_>>()
    };
    if !base_file.is_empty() {
        components.pop();
    }
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component if component.chars().any(char::is_control) => return None,
            component => components.push(component),
        }
    }
    if components.is_empty() {
        None
    } else {
        Some(components.join("/"))
    }
}

fn read_epub_member<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    members: &HashMap<String, usize>,
    name: &str,
    max_size: u64,
) -> Result<Vec<u8>, String> {
    read_archive_member(archive, members, name, max_size, "EPUB")
}

#[derive(Debug, Default)]
struct XmlNode {
    name: String,
    attributes: HashMap<String, String>,
    text: String,
    children: Vec<XmlNode>,
}

fn parse_xml_tree(xml: &str) -> Result<Vec<XmlNode>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack = Vec::<XmlNode>::new();
    let mut roots = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => stack.push(xml_node(&element)?),
            Ok(Event::Empty(element)) => {
                let node = xml_node(&element)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
            Ok(Event::Text(value)) => {
                if let Some(node) = stack.last_mut() {
                    let decoded = quick_xml::escape::unescape(value.as_ref())
                        .map_err(|_| "EPUB XML text has invalid entities".to_owned())?;
                    node.text.push_str(&decoded);
                }
            }
            Ok(Event::CData(value)) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(value.as_ref());
                }
            }
            Ok(Event::End(_)) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| "Malformed EPUB XML tree".to_owned())?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err("EPUB metadata XML is malformed".to_owned()),
        }
    }
    if !stack.is_empty() {
        return Err("EPUB metadata XML is incomplete".to_owned());
    }
    Ok(roots)
}

fn xml_node(element: &quick_xml::events::BytesStart<'_>) -> Result<XmlNode, String> {
    let name = element.local_name().as_ref().to_owned();
    let mut node = XmlNode {
        name,
        ..XmlNode::default()
    };
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|_| "Malformed EPUB XML attribute".to_owned())?;
        let key = attribute.key.local_name().as_ref().to_owned();
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|_| "Malformed EPUB XML attribute value".to_owned())?
            .into_owned();
        node.attributes.insert(key, value);
    }
    Ok(node)
}

fn find_nodes<'a>(nodes: &'a [XmlNode], name: &str) -> Vec<&'a XmlNode> {
    let mut result = Vec::new();
    for node in nodes {
        if node.name == name {
            result.push(node);
        }
        result.extend(find_nodes(&node.children, name));
    }
    result
}

fn find_child<'a>(node: &'a XmlNode, name: &str) -> Option<&'a XmlNode> {
    node.children.iter().find(|child| child.name == name)
}

fn parse_opf(opf: &str, opf_path: String) -> Result<EpubPackage, String> {
    let roots = parse_xml_tree(opf)?;
    let package = roots
        .iter()
        .find(|node| node.name == "package")
        .ok_or_else(|| "EPUB OPF does not contain a package element".to_owned())?;
    let metadata = find_child(package, "metadata");
    let title = metadata
        .and_then(|node| find_nodes(&node.children, "title").into_iter().next())
        .map(|node| node.text.trim().to_owned())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "本地 EPUB".to_owned());
    let author = metadata
        .and_then(|node| find_nodes(&node.children, "creator").into_iter().next())
        .map(|node| node.text.trim().to_owned())
        .unwrap_or_default();
    let manifest_node =
        find_child(package, "manifest").ok_or_else(|| "EPUB OPF has no manifest".to_owned())?;
    let mut manifest = HashMap::new();
    let mut cover_id = None;
    for item_node in manifest_node
        .children
        .iter()
        .filter(|node| node.name == "item")
    {
        let id = item_node.attributes.get("id").cloned().unwrap_or_default();
        let href = item_node
            .attributes
            .get("href")
            .cloned()
            .unwrap_or_default();
        let media_type = item_node
            .attributes
            .get("media-type")
            .cloned()
            .unwrap_or_default();
        let properties = item_node
            .attributes
            .get("properties")
            .into_iter()
            .flat_map(|properties| properties.split_ascii_whitespace())
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        if id.is_empty() || href.is_empty() || manifest.contains_key(&id) {
            return Err("EPUB manifest has an invalid or duplicate item".to_owned());
        }
        if properties.contains("cover-image") {
            cover_id = Some(id.clone());
        }
        manifest.insert(id, ManifestItem { href, media_type });
    }
    if cover_id.is_none() {
        cover_id = metadata.and_then(|node| {
            find_nodes(&node.children, "meta")
                .into_iter()
                .find_map(|meta| {
                    (meta
                        .attributes
                        .get("name")
                        .is_some_and(|name| name == "cover"))
                    .then(|| meta.attributes.get("content").cloned())
                    .flatten()
                })
        });
    }
    let spine_node =
        find_child(package, "spine").ok_or_else(|| "EPUB OPF has no spine".to_owned())?;
    let spine = spine_node
        .children
        .iter()
        .filter(|node| node.name == "itemref")
        .filter(|node| {
            node.attributes
                .get("linear")
                .is_none_or(|linear| linear != "no")
        })
        .filter_map(|node| node.attributes.get("idref").cloned())
        .collect::<Vec<_>>();
    if spine.is_empty() {
        return Err("EPUB spine has no linear reading order".to_owned());
    }
    Ok(EpubPackage {
        title,
        author,
        opf_path,
        manifest,
        spine,
        cover_id,
    })
}

fn is_chapter_type(media_type: &str) -> bool {
    matches!(media_type, "application/xhtml+xml" | "text/html")
}

fn is_importable_asset(path: &str) -> bool {
    let extension = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "css"
            | "png"
            | "jpg"
            | "jpeg"
            | "svg"
            | "webp"
            | "gif"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "mp3"
            | "m4a"
            | "aac"
            | "ogg"
            | "mp4"
            | "webm"
            | "wav"
    )
}

fn is_css_path(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("css"))
}

fn is_svg_path(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

fn asset_id(path: &str) -> String {
    let extension = path
        .rsplit('.')
        .next()
        .filter(|extension| {
            extension.len() <= 8 && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .unwrap_or("bin")
        .to_ascii_lowercase();
    format!("epub-{}.{}", short_hash(path), extension)
}
