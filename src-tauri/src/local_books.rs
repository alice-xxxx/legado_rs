//! Imports local text, EPUB, CBZ, and PDF files into public book/HTML resources
//! used by books fetched through the Kotlin source engine.
//!
//! This module accepts filesystem paths only after the native file picker has
//! returned them. It never sends file contents through Tauri IPC and never
//! evaluates source rules or scripts.

#[path = "txt_toc_rules.rs"]
pub mod txt_toc_rules;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::OnceLock;

use encoding_rs::{Encoding, GBK};
use image::{ImageFormat, ImageReader, Limits};
use lopdf::{Dictionary, Document as PdfDocument, LoadOptions, Object};
use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::models::{
    BookDocument, ChapterDescriptor, ProgressSummary, ReaderDefaults, CURRENT_SCHEMA_VERSION,
};
use crate::resources::ResourceStore;

const MAX_TEXT_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EPUB_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EPUB_EXPANDED_BYTES: u64 = 768 * 1024 * 1024;
const MAX_EPUB_FILES: usize = 100_000;
const MAX_EPUB_XML_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EPUB_CHAPTERS: usize = 20_000;
const MAX_EPUB_ASSET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CBZ_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CBZ_EXPANDED_BYTES: u64 = 768 * 1024 * 1024;
const MAX_CBZ_FILES: usize = 100_000;
const MAX_CBZ_PAGES: usize = 20_000;
const MAX_CBZ_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PDF_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PDF_DECOMPRESSED_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 10_000;
const MAX_PDF_PASSWORD_BYTES: usize = 1024;

/// Rust-side import options. The optional TOC pattern is a regular expression
/// over complete text lines; it is not a book-source rule and is never sent to
/// the source engine or WebView for execution.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalImportOptions {
    /// An encoding_rs label such as `gbk`, `big5`, or `utf-8`. When omitted,
    /// valid UTF-8 is preferred and legacy Chinese TXT files fall back to GBK.
    #[serde(default)]
    pub charset: Option<String>,
    /// An optional user TOC expression. Capture group 1 supplies the title;
    /// without a capture, the full matching line becomes the chapter title.
    #[serde(default)]
    pub toc_regex: Option<String>,
    /// Used only while inspecting a protected PDF. Never serialized or saved.
    #[serde(default, skip_serializing)]
    pub pdf_password: Option<String>,
}

impl fmt::Debug for LocalImportOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalImportOptions")
            .field("charset", &self.charset)
            .field("toc_regex", &self.toc_regex)
            .field(
                "pdf_password",
                &self.pdf_password.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

#[derive(Debug)]
struct ParsedChapter {
    title: String,
    html: Option<String>,
    text: Option<String>,
    pdf_page_index: Option<u32>,
}

#[derive(Debug)]
struct ParsedAsset {
    id: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct ParsedBook {
    identity: String,
    title: String,
    author: String,
    chapters: Vec<ParsedChapter>,
    assets: Vec<ParsedAsset>,
    cover_asset_id: Option<String>,
    pdf_asset_id: Option<String>,
}

/// Import a user-selected local `.txt` or `.epub` path. The caller should pass
/// only a path returned by a native picker or another controlled file API.
/// The importer persists browser-ready chapter HTML, EPUB media assets, and a
/// JSON `BookDocument`; the caller owns shelf insertion and UI notification.
pub async fn import_local_book(
    store: &ResourceStore,
    selected_path: impl AsRef<Path>,
    defaults: &ReaderDefaults,
    options: &LocalImportOptions,
) -> Result<BookDocument, String> {
    let selected_path = selected_path.as_ref().to_path_buf();
    let options = options.clone();
    let saved_toc_rules = if selected_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        && options.toc_regex.is_none()
    {
        Some(txt_toc_rules::read_document(store).await?)
    } else {
        None
    };
    let parsed = tokio::task::spawn_blocking(move || {
        parse_selected_file(&selected_path, &options, saved_toc_rules.as_ref())
    })
    .await
    .map_err(|error| format!("Local book import worker failed: {error}"))??;

    let book_id = format!("local-{}", parsed.identity);
    // Re-importing the same file is idempotent. In particular, do not rewrite
    // the user's progress or cached chapters merely because the picker was
    // opened again for an already imported file.
    let book_ref = store
        .book_ref(&book_id)
        .map_err(|error| error.to_string())?;
    if store.root().join(book_ref.path()).is_file() {
        let existing = store
            .read_json_ref(&book_ref)
            .await
            .map_err(|error| error.to_string())?;
        return serde_json::from_value(existing)
            .map_err(|error| format!("Existing local book document is invalid: {error}"));
    }
    let result = persist_parsed_book(store, &book_id, parsed, defaults).await;
    if result.is_err() {
        let _ = store.remove_book_resources(&book_id).await;
    }
    result
}

async fn persist_parsed_book(
    store: &ResourceStore,
    book_id: &str,
    parsed: ParsedBook,
    defaults: &ReaderDefaults,
) -> Result<BookDocument, String> {
    let mut cover_src = None;
    for asset in &parsed.assets {
        let reference = store
            .write_asset(book_id, &asset.id, &asset.bytes)
            .await
            .map_err(|error| error.to_string())?;
        if parsed.cover_asset_id.as_deref() == Some(asset.id.as_str()) {
            cover_src = Some(reference);
        }
    }

    if parsed.chapters.is_empty() {
        return Err("The selected book contains no readable chapters".to_owned());
    }
    if parsed.chapters.len() > MAX_EPUB_CHAPTERS {
        return Err("The selected book has too many chapters".to_owned());
    }

    let mut chapters = Vec::with_capacity(parsed.chapters.len());
    for (index, parsed_chapter) in parsed.chapters.into_iter().enumerate() {
        let chapter_id = format!("chapter-{:05}", index + 1);
        let src = if let (Some(pdf_asset_id), Some(page_index)) = (
            parsed.pdf_asset_id.as_deref(),
            parsed_chapter.pdf_page_index,
        ) {
            store
                .write_pdf_page(
                    book_id,
                    &chapter_id,
                    pdf_asset_id,
                    page_index,
                    "page-fit",
                    defaults,
                )
                .await
        } else if let Some(html) = parsed_chapter.html {
            store
                .write_chapter_html(book_id, &chapter_id, &html, defaults)
                .await
        } else {
            store
                .write_chapter_text(
                    book_id,
                    &chapter_id,
                    parsed_chapter.text.as_deref().unwrap_or_default(),
                    defaults,
                )
                .await
        }
        .map_err(|error| error.to_string())?;
        chapters.push(ChapterDescriptor {
            id: chapter_id,
            title: parsed_chapter.title,
            index,
            src: Some(src),
        });
    }

    let latest_chapter = chapters.last().map(|chapter| chapter.title.clone());
    let book = BookDocument {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: book_id.to_owned(),
        title: parsed.title,
        can_change_source: false,
        author: parsed.author,
        cover_src,
        intro: None,
        kind: None,
        word_count: None,
        source_id: None,
        source_name: None,
        source_group: None,
        chapter_count: chapters.len(),
        latest_chapter,
        progress: ProgressSummary::default(),
        chapters,
    };
    store
        .write_book(&book)
        .await
        .map_err(|error| error.to_string())?;
    Ok(book)
}

fn parse_selected_file(
    path: &Path,
    options: &LocalImportOptions,
    saved_toc_rules: Option<&txt_toc_rules::TxtTocRulesDocument>,
) -> Result<ParsedBook, String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("Cannot resolve selected file: {error}"))?;
    let metadata =
        std::fs::metadata(&path).map_err(|error| format!("Cannot read selected file: {error}"))?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".to_owned());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if options
        .pdf_password
        .as_deref()
        .is_some_and(|password| password.len() > MAX_PDF_PASSWORD_BYTES)
    {
        return Err("PDF password exceeds the supported length limit".to_owned());
    }
    let max_size = match extension.as_str() {
        "txt" => MAX_TEXT_FILE_BYTES,
        "epub" => MAX_EPUB_FILE_BYTES,
        "cbz" => MAX_CBZ_FILE_BYTES,
        "pdf" => MAX_PDF_FILE_BYTES,
        _ => return Err("Supported local book formats are TXT, EPUB, CBZ, and PDF".to_owned()),
    };
    if metadata.len() == 0 || metadata.len() > max_size {
        return Err(format!(
            "Selected file size must be between 1 byte and {max_size} bytes"
        ));
    }
    let bytes =
        std::fs::read(&path).map_err(|error| format!("Cannot read selected file: {error}"))?;
    let identity = local_identity(&path, &bytes, options);
    let mut parsed = match extension.as_str() {
        "txt" => parse_text_file(&path, &bytes, options, saved_toc_rules),
        "epub" => parse_epub_file(&bytes),
        "cbz" => parse_cbz_file(&path, &bytes),
        "pdf" => parse_pdf_file(bytes, &path, options.pdf_password.as_deref()),
        _ => unreachable!("extension was checked above"),
    }?;
    parsed.identity = identity;
    Ok(parsed)
}

fn local_identity(path: &Path, bytes: &[u8], options: &LocalImportOptions) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_os_str().to_string_lossy().as_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    hasher.update([0]);
    if let Ok(options) = serde_json::to_vec(options) {
        hasher.update(options);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_cbz_file(path: &Path, bytes: &[u8]) -> Result<ParsedBook, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "The selected CBZ is not a valid ZIP archive".to_owned())?;
    let members = index_zip_members(&mut archive, MAX_CBZ_FILES, MAX_CBZ_EXPANDED_BYTES, "CBZ")?;
    let mut pages = members
        .keys()
        .filter(|name| cbz_image_format(name).is_some())
        .cloned()
        .collect::<Vec<_>>();
    pages.sort_by(|left, right| {
        natord::compare_ignore_case(left, right).then_with(|| left.cmp(right))
    });
    if pages.is_empty() {
        return Err("CBZ archive contains no supported image pages".to_owned());
    }
    if pages.len() > MAX_CBZ_PAGES {
        return Err("CBZ archive contains too many image pages".to_owned());
    }

    let mut assets = Vec::with_capacity(pages.len());
    let mut chapters = Vec::with_capacity(pages.len());
    let mut cover_asset_id = None;
    for (index, name) in pages.iter().enumerate() {
        let format = cbz_image_format(name).expect("filtered supported image format");
        let image_bytes =
            read_archive_member(&mut archive, &members, name, MAX_CBZ_IMAGE_BYTES, "CBZ")?;
        let mut decoder = ImageReader::with_format(Cursor::new(&image_bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(32_768);
        limits.max_image_height = Some(32_768);
        limits.max_alloc = Some(256 * 1024 * 1024);
        decoder.limits(limits);
        decoder
            .decode()
            .map_err(|_| format!("CBZ page image is invalid or exceeds decoder limits: {name}"))?;

        let extension = Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let id = format!("cbz-{}.{}", short_hash(name), extension);
        if index == 0 {
            cover_asset_id = Some(id.clone());
        }
        assets.push(ParsedAsset {
            id: id.clone(),
            bytes: image_bytes,
        });
        let page_name = Path::new(name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.trim().is_empty())
            .map(|stem| stem.chars().take(200).collect::<String>())
            .unwrap_or_else(|| format!("Page {}", index + 1));
        chapters.push(ParsedChapter {
            title: page_name.clone(),
            html: Some(format!(
                r#"<img src="../assets/{id}" alt="{}">"#,
                escape_html_attr(&page_name)
            )),
            text: None,
            pdf_page_index: None,
        });
    }

    let title = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("CBZ Book")
        .chars()
        .take(200)
        .collect();
    Ok(ParsedBook {
        identity: String::new(),
        title,
        author: String::new(),
        chapters,
        assets,
        cover_asset_id,
        pdf_asset_id: None,
    })
}

fn cbz_image_format(name: &str) -> Option<ImageFormat> {
    match Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::WebP),
        "gif" => Some(ImageFormat::Gif),
        "bmp" => Some(ImageFormat::Bmp),
        _ => None,
    }
}

fn parse_pdf_file(
    bytes: Vec<u8>,
    path: &Path,
    password: Option<&str>,
) -> Result<ParsedBook, String> {
    let options = LoadOptions {
        password: password.map(str::to_owned),
        max_decompressed_size: Some(MAX_PDF_DECOMPRESSED_STREAM_BYTES),
        ..LoadOptions::default()
    };
    let document =
        PdfDocument::load_mem_with_options(&bytes, options).map_err(|error| match error {
            lopdf::Error::Unimplemented(message) if message.contains("requires a password") => {
                "This PDF is password protected; provide pdfPassword to inspect it".to_owned()
            }
            lopdf::Error::InvalidPassword => "The PDF password is incorrect".to_owned(),
            _ => "The selected PDF is damaged, unsupported, or could not be opened".to_owned(),
        })?;
    if document.is_encrypted() {
        return Err("This PDF is password protected; provide pdfPassword to inspect it".to_owned());
    }
    let pages = document.get_pages();
    if pages.is_empty() {
        return Err("PDF contains no readable pages".to_owned());
    }
    validate_pdf_page_count(pages.len())?;

    let info = pdf_info_dictionary(&document);
    let title = info
        .and_then(|info| pdf_info_text(info, b"Title"))
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "PDF Book".to_owned())
        .chars()
        .take(200)
        .collect();
    let author = info
        .and_then(|info| pdf_info_text(info, b"Author"))
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    let chapters = pages
        .keys()
        .enumerate()
        .map(|(index, _)| ParsedChapter {
            title: format!("Page {}", index + 1),
            html: None,
            text: None,
            pdf_page_index: Some(index as u32),
        })
        .collect::<Vec<_>>();
    let pdf_asset_id = format!("pdf-{}.pdf", short_hash(path.to_string_lossy().as_ref()));
    Ok(ParsedBook {
        identity: String::new(),
        title,
        author,
        chapters,
        assets: vec![ParsedAsset {
            id: pdf_asset_id.clone(),
            bytes,
        }],
        cover_asset_id: None,
        pdf_asset_id: Some(pdf_asset_id),
    })
}

fn pdf_info_dictionary(document: &PdfDocument) -> Option<&Dictionary> {
    let info = document.trailer.get(b"Info").ok()?;
    match info {
        Object::Reference(id) => document.get_object(*id).ok()?.as_dict().ok(),
        Object::Dictionary(dictionary) => Some(dictionary),
        _ => None,
    }
}

fn pdf_info_text(info: &Dictionary, key: &[u8]) -> Option<String> {
    lopdf::decode_text_string(info.get(key).ok()?).ok()
}

fn validate_pdf_page_count(page_count: usize) -> Result<(), String> {
    if page_count == 0 {
        return Err("PDF contains no readable pages".to_owned());
    }
    if page_count > MAX_PDF_PAGES {
        return Err(format!("PDF exceeds the {MAX_PDF_PAGES}-page import limit"));
    }
    Ok(())
}

fn parse_text_file(
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
                (false, _) => txt_toc_rules::pattern_headings(&text, default_toc_regex())?,
            }
        }
    };

    let mut chapters = Vec::new();
    if let Some(first) = headings.first() {
        let preface = text[..first.line_start].trim();
        if !preface.is_empty() {
            chapters.push(ParsedChapter {
                title: "序章".to_owned(),
                html: None,
                text: Some(preface.to_owned()),
                pdf_page_index: None,
            });
        }
        for (index, heading) in headings.iter().enumerate() {
            let body_end = headings
                .get(index + 1)
                .map_or(text.len(), |next| next.line_start);
            let chapter_text = text[heading.body_start..body_end].trim();
            if !chapter_text.is_empty() {
                chapters.push(ParsedChapter {
                    title: heading.title.clone(),
                    html: None,
                    text: Some(chapter_text.to_owned()),
                    pdf_page_index: None,
                });
            }
        }
    } else {
        chapters.push(ParsedChapter {
            title: "正文".to_owned(),
            html: None,
            text: Some(text.trim().to_owned()),
            pdf_page_index: None,
        });
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

    Ok(ParsedBook {
        identity: String::new(),
        title: file_stem(path),
        author: String::new(),
        chapters,
        assets: Vec::new(),
        cover_asset_id: None,
        pdf_asset_id: None,
    })
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

fn default_toc_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        // Requiring a separator after the ordinal avoids treating prose such
        // as “第一章正文。” as another empty chapter. A bare ordinal line is
        // still a valid heading.
        Regex::new(r"(?i)^\s*((?:第[零〇一二三四五六七八九十百千万两\d]+[章节回卷集部](?:[ \t]+[^。！？!?]{1,80}|[:：、.．-][ \t]*[^。！？!?]{1,80})?|chapter\s+\d+(?:[ \t]+[^.!?]{1,80}|[:.-][ \t]*[^.!?]{1,80})?))\s*$")
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

fn parse_epub_file(bytes: &[u8]) -> Result<ParsedBook, String> {
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
        if let Some(css_path) = original_path.filter(|path| is_css_path(path)) {
            let css = String::from_utf8_lossy(&asset.bytes);
            let rewritten = rewrite_css(&css, &css_path, &asset_ids);
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

fn index_zip_members<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    max_files: usize,
    max_expanded_bytes: u64,
    format: &str,
) -> Result<HashMap<String, usize>, String> {
    if archive.len() > max_files {
        return Err(format!("{format} contains too many ZIP entries"));
    }
    let mut names = HashMap::new();
    let mut expanded_total = 0u64;
    for index in 0..archive.len() {
        let member = archive
            .by_index(index)
            .map_err(|_| format!("Cannot inspect {format} ZIP entry"))?;
        let name = member.name().to_owned();
        if member
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!(
                "{format} must not contain symbolic-link ZIP entries"
            ));
        }
        let normalized = validate_zip_member_name(&name)
            .ok_or_else(|| format!("{format} contains an unsafe ZIP entry path"))?;
        if member.is_dir() {
            continue;
        }
        if names.insert(normalized, index).is_some() {
            return Err(format!("{format} contains duplicate ZIP entry names"));
        }
        expanded_total = expanded_total.saturating_add(member.size());
        if expanded_total > max_expanded_bytes {
            return Err(format!("{format} expands beyond the supported size limit"));
        }
    }
    Ok(names)
}

fn validate_zip_member_name(name: &str) -> Option<String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.contains(':')
    {
        return None;
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return None;
    }
    let parts = trimmed.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return None;
    }
    Some(trimmed.to_owned())
}

fn normalize_epub_href(base_file: &str, href: &str) -> Option<String> {
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

fn read_archive_member<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    members: &HashMap<String, usize>,
    name: &str,
    max_size: u64,
    format: &str,
) -> Result<Vec<u8>, String> {
    let index = *members
        .get(name)
        .ok_or_else(|| format!("{format} entry is missing: {name}"))?;
    let member = archive
        .by_index(index)
        .map_err(|_| format!("Cannot open {format} ZIP entry"))?;
    if member.size() > max_size {
        return Err(format!("{format} entry exceeds the supported size limit"));
    }
    let mut bytes = Vec::with_capacity(member.size() as usize);
    member
        .take(max_size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("Cannot decompress {format} ZIP entry"))?;
    if bytes.len() as u64 > max_size {
        return Err(format!(
            "{format} entry expands beyond the supported size limit"
        ));
    }
    Ok(bytes)
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

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn rewrite_epub_html(
    html: &str,
    html_path: &str,
    assets: &HashMap<String, String>,
    chapters: &HashMap<String, String>,
) -> String {
    let mut output = rewrite_html_attr(html, html_path, assets, chapters, false);
    output = rewrite_html_attr(&output, html_path, assets, chapters, true);
    // srcset is deliberately removed until every candidate can be mapped to a
    // verified local resource. This avoids leaking remote/data URLs to WebView.
    output = srcset_double_regex().replace_all(&output, "").into_owned();
    srcset_single_regex().replace_all(&output, "").into_owned()
}

fn rewrite_html_attr(
    html: &str,
    html_path: &str,
    assets: &HashMap<String, String>,
    chapters: &HashMap<String, String>,
    single_quote: bool,
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
            let allow_external = name == "href" && is_anchor && is_safe_external_link(value);
            let replacement = if allow_external {
                Some(value.to_owned())
            } else if let Some(path) = normalize_epub_href(html_path, value) {
                chapters
                    .get(&path)
                    .map(|chapter_id| format!("../chapters/{chapter_id}.html"))
                    .or_else(|| {
                        assets
                            .get(&path)
                            .map(|asset_id| format!("../assets/{asset_id}"))
                    })
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

fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
}

fn rewrite_css(css: &str, css_path: &str, assets: &HashMap<String, String>) -> String {
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
                .map(|asset| format!("url(\"{}\")", escape_css_string(asset)))
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

fn chapter_title(html: &str, fallback: &str, index: usize) -> String {
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

fn inline_style_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<style(?:\s[^>]*)?>(.*?)</style\s*>").unwrap())
}

#[cfg(test)]
mod tests {
    use super::{import_local_book, validate_pdf_page_count, LocalImportOptions};
    use crate::local_books::txt_toc_rules;
    use crate::models::{ProgressDocument, ReaderDefaults, CURRENT_SCHEMA_VERSION};
    use crate::resources::ResourceStore;
    use base64::Engine;
    use encoding_rs::GBK;
    use image::{GenericImageView, ImageBuffer, Rgba};
    use lopdf::content::{Content, Operation};
    use lopdf::dictionary;
    use lopdf::{
        Document as PdfDocument, EncryptionState, EncryptionVersion, LoadOptions, Object,
        Permissions, Stream,
    };
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn png_fixture(color: [u8; 4]) -> Vec<u8> {
        let image = ImageBuffer::from_pixel(2, 2, Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn pdf_fixture(page_count: usize, password: Option<&str>) -> Vec<u8> {
        let mut document = PdfDocument::with_version("1.4");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(lopdf::dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = document.add_object(lopdf::dictionary! {
            "Font" => lopdf::dictionary! { "F1" => font_id },
        });
        let mut page_ids = Vec::with_capacity(page_count);
        for index in 0..page_count {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 14.into()]),
                    Operation::new("Td", vec![36.into(), 740.into()]),
                    Operation::new(
                        "Tj",
                        vec![Object::string_literal(format!(
                            "Fixture page {}",
                            index + 1
                        ))],
                    ),
                    Operation::new("ET", vec![]),
                ],
            };
            let content_id = document.add_object(Stream::new(
                lopdf::dictionary! {},
                content.encode().unwrap(),
            ));
            page_ids.push(document.add_object(lopdf::dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }));
        }
        let pages = lopdf::dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_count as i64,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        };
        document.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = document.add_object(lopdf::dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        let info_id = document.add_object(lopdf::dictionary! {
            "Title" => lopdf::text_string("Fixture PDF Title"),
            "Author" => lopdf::text_string("Fixture PDF Author"),
        });
        document.trailer.set("Root", catalog_id);
        document.trailer.set("Info", info_id);
        document.trailer.set(
            "ID",
            vec![
                Object::string_literal("fixture-id"),
                Object::string_literal("fixture-id"),
            ],
        );
        if let Some(password) = password {
            let state = EncryptionState::try_from(EncryptionVersion::V1 {
                document: &document,
                owner_password: "fixture-owner",
                user_password: password,
                permissions: Permissions::default(),
            })
            .unwrap();
            document.encrypt(&state).unwrap();
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn write_cbz(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        for (name, bytes) in entries {
            zip.start_file(
                *name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn checked_in_fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/local-books")
            .join(name)
    }

    #[tokio::test]
    async fn imports_cbz_in_natural_order_and_serves_decodable_image_assets() {
        let temporary = tempfile::tempdir().unwrap();
        let store = ResourceStore::open(temporary.path().join("app-data")).unwrap();
        let cbz_path = temporary.path().join("Comic.cbz");
        let page1 = png_fixture([240, 30, 20, 255]);
        let page2 = png_fixture([20, 220, 30, 255]);
        let page10 = png_fixture([20, 30, 240, 255]);
        write_cbz(
            &cbz_path,
            &[
                ("pages/page10.png", &page10),
                ("pages/page2.png", &page2),
                ("pages/page1.png", &page1),
            ],
        );

        let book = import_local_book(
            &store,
            &cbz_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(book.title, "Comic");
        assert!(!book.can_change_source);
        assert_eq!(book.chapter_count, 3);
        assert_eq!(
            book.chapters
                .iter()
                .map(|chapter| chapter.title.as_str())
                .collect::<Vec<_>>(),
            ["page1", "page2", "page10"]
        );

        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let expected_colors = [[240, 30, 20, 255], [20, 220, 30, 255], [20, 30, 240, 255]];
        let mut first_image_url = None;
        for (index, chapter) in book.chapters.iter().enumerate() {
            let chapter_url =
                reqwest::Url::parse(&server.url_for(chapter.src.as_ref().unwrap())).unwrap();
            let html = reqwest::get(chapter_url.clone())
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .text()
                .await
                .unwrap();
            assert!(html.contains(&format!("alt=\"{}\"", chapter.title)));
            let relative = html
                .split("src=\"../assets/")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let image_url = chapter_url.join(&format!("../assets/{relative}")).unwrap();
            if index == 0 {
                first_image_url = Some(image_url.clone());
            }
            let image_response = reqwest::get(image_url)
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            assert_eq!(
                image_response.headers()[reqwest::header::CONTENT_TYPE],
                "image/png"
            );
            let image_bytes = image_response.bytes().await.unwrap();
            let decoded = image::load_from_memory(&image_bytes).unwrap();
            assert_eq!(decoded.get_pixel(0, 0).0, expected_colors[index]);
        }
        let cover = book.cover_src.as_ref().unwrap();
        assert_eq!(
            first_image_url.unwrap(),
            reqwest::Url::parse(&server.url_for(cover)).unwrap()
        );
    }

    #[tokio::test]
    async fn cbz_corrupt_images_and_unsafe_zip_paths_fail_before_writing_resources() {
        let temporary = tempfile::tempdir().unwrap();
        let store = ResourceStore::open(temporary.path().join("app-data")).unwrap();
        let valid = png_fixture([100, 120, 140, 255]);
        let corrupt_path = temporary.path().join("Corrupt.cbz");
        write_cbz(
            &corrupt_path,
            &[("1.png", &valid), ("2.png", b"not a PNG image")],
        );
        let corrupt_bytes = std::fs::read(&corrupt_path).unwrap();
        let corrupt_id = super::local_identity(
            &corrupt_path,
            &corrupt_bytes,
            &LocalImportOptions::default(),
        );
        let error = import_local_book(
            &store,
            &corrupt_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("image is invalid"));
        assert!(!store
            .root()
            .join(format!("books/local-{corrupt_id}/book.json"))
            .exists());

        let unsafe_path = temporary.path().join("Unsafe.cbz");
        write_cbz(&unsafe_path, &[("../escaped.png", &valid)]);
        let error = import_local_book(
            &store,
            &unsafe_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("unsafe ZIP entry path"));
        assert!(!temporary.path().join("escaped.png").exists());
    }

    #[tokio::test]
    async fn imports_pdf_pages_with_range_readable_original_binary_and_password_errors() {
        let temporary = tempfile::tempdir().unwrap();
        let store = ResourceStore::open(temporary.path().join("app-data")).unwrap();
        let pdf_path = temporary.path().join("Fixture.pdf");
        let pdf_bytes = pdf_fixture(2, None);
        std::fs::write(&pdf_path, &pdf_bytes).unwrap();
        let book = import_local_book(
            &store,
            &pdf_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(book.title, "Fixture PDF Title");
        assert_eq!(book.author, "Fixture PDF Author");
        assert_eq!(book.chapter_count, 2);
        assert_eq!(book.chapters[1].title, "Page 2");

        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let chapter_url =
            reqwest::Url::parse(&server.url_for(book.chapters[0].src.as_ref().unwrap())).unwrap();
        let html = reqwest::get(chapter_url.clone())
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html.contains("data-legado-document=\"pdf-page\""));
        assert!(html.contains("data-page-index=\"0\""));
        assert!(html.contains("data-default-zoom=\"page-fit\""));
        let relative_pdf = html
            .split("href=\"../assets/")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let pdf_url = chapter_url
            .join(&format!("../assets/{relative_pdf}"))
            .unwrap();
        let range = reqwest::Client::new()
            .get(pdf_url.clone())
            .header(reqwest::header::RANGE, "bytes=0-7")
            .send()
            .await
            .unwrap();
        assert_eq!(range.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            range.headers()[reqwest::header::CONTENT_TYPE],
            "application/pdf"
        );
        assert_eq!(range.bytes().await.unwrap().as_ref(), b"%PDF-1.4");

        let complete_response = reqwest::get(pdf_url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(
            complete_response.headers()[reqwest::header::CONTENT_TYPE],
            "application/pdf"
        );
        let served_pdf = complete_response.bytes().await.unwrap();
        let reopened =
            PdfDocument::load_mem_with_options(&served_pdf, LoadOptions::default()).unwrap();
        assert_eq!(reopened.get_pages().len(), 2);

        let protected_path = temporary.path().join("Protected.pdf");
        let protected_bytes = pdf_fixture(2, Some("secret-reader-password"));
        std::fs::write(&protected_path, &protected_bytes).unwrap();
        let no_password = import_local_book(
            &store,
            &protected_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(no_password.contains("password protected"));
        let wrong_options = LocalImportOptions {
            pdf_password: Some("wrong-password".to_owned()),
            ..LocalImportOptions::default()
        };
        let wrong_password = import_local_book(
            &store,
            &protected_path,
            &ReaderDefaults::default(),
            &wrong_options,
        )
        .await
        .unwrap_err();
        assert!(wrong_password.contains("password is incorrect"));
        assert!(!format!("{wrong_options:?}").contains("wrong-password"));
        assert!(!serde_json::to_string(&wrong_options)
            .unwrap()
            .contains("pdfPassword"));

        let correct_options = LocalImportOptions {
            pdf_password: Some("secret-reader-password".to_owned()),
            ..LocalImportOptions::default()
        };
        let protected_book = import_local_book(
            &store,
            &protected_path,
            &ReaderDefaults::default(),
            &correct_options,
        )
        .await
        .unwrap();
        let pdf_resource = store
            .asset_ref(
                &protected_book.id,
                &format!(
                    "pdf-{}.pdf",
                    super::short_hash(protected_path.to_string_lossy().as_ref())
                ),
            )
            .unwrap();
        let encrypted_asset = reqwest::get(server.url_for(&pdf_resource))
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(encrypted_asset.as_ref(), protected_bytes.as_slice());
        let reopened = PdfDocument::load_mem_with_options(
            &encrypted_asset,
            LoadOptions::with_password("secret-reader-password"),
        )
        .unwrap();
        assert_eq!(reopened.get_pages().len(), 2);
    }

    #[tokio::test]
    async fn checked_in_pdf_and_cbz_fixtures_survive_resource_server_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let data_root = temporary.path().join("app-data");
        let store = ResourceStore::open(&data_root).unwrap();
        let defaults = ReaderDefaults::default();

        let pdf_path = checked_in_fixture("two-page-text.pdf");
        let pdf_book =
            import_local_book(&store, &pdf_path, &defaults, &LocalImportOptions::default())
                .await
                .unwrap();
        assert_eq!(pdf_book.chapter_count, 2);
        assert_eq!(pdf_book.chapters[0].title, "Page 1");
        assert_eq!(pdf_book.chapters[1].title, "Page 2");

        let encrypted_path = checked_in_fixture("two-page-encrypted.pdf");
        let encrypted_options = LocalImportOptions {
            pdf_password: Some("fixture-pass".to_owned()),
            ..LocalImportOptions::default()
        };
        let encrypted_book =
            import_local_book(&store, &encrypted_path, &defaults, &encrypted_options)
                .await
                .unwrap();
        assert_eq!(encrypted_book.chapter_count, 2);

        let cbz_path = checked_in_fixture("three-page-comic.cbz");
        let comic_book =
            import_local_book(&store, &cbz_path, &defaults, &LocalImportOptions::default())
                .await
                .unwrap();
        assert_eq!(comic_book.chapter_count, 3);
        assert_eq!(
            comic_book
                .chapters
                .iter()
                .map(|chapter| chapter.title.as_str())
                .collect::<Vec<_>>(),
            ["page1", "page2", "page10"]
        );

        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let pdf_chapter_url =
            reqwest::Url::parse(&server.url_for(pdf_book.chapters[0].src.as_ref().unwrap()))
                .unwrap();
        let pdf_html = reqwest::get(pdf_chapter_url.clone())
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(pdf_html.contains("data-legado-document=\"pdf-page\""));
        assert!(pdf_html.contains("data-page-index=\"0\""));
        assert!(pdf_html.contains("data-default-zoom=\"page-fit\""));
        let pdf_relative = pdf_html
            .split("href=\"../assets/")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let pdf_url = pdf_chapter_url
            .join(&format!("../assets/{pdf_relative}"))
            .unwrap();
        let pdf_response = reqwest::get(pdf_url.clone())
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(
            pdf_response.headers()[reqwest::header::CONTENT_TYPE],
            "application/pdf"
        );
        let pdf_bytes = pdf_response.bytes().await.unwrap();
        let parsed_pdf =
            PdfDocument::load_mem_with_options(&pdf_bytes, LoadOptions::default()).unwrap();
        assert_eq!(parsed_pdf.get_pages().len(), 2);

        let encrypted_chapter_url =
            reqwest::Url::parse(&server.url_for(encrypted_book.chapters[0].src.as_ref().unwrap()))
                .unwrap();
        let encrypted_html = reqwest::get(encrypted_chapter_url.clone())
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        let encrypted_relative = encrypted_html
            .split("href=\"../assets/")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let encrypted_url = encrypted_chapter_url
            .join(&format!("../assets/{encrypted_relative}"))
            .unwrap();
        let encrypted_bytes = reqwest::get(encrypted_url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let parsed_encrypted = PdfDocument::load_mem_with_options(
            &encrypted_bytes,
            LoadOptions::with_password("fixture-pass"),
        )
        .unwrap();
        assert_eq!(parsed_encrypted.get_pages().len(), 2);
        let password_options = LocalImportOptions {
            pdf_password: Some("fixture-pass".to_owned()),
            ..LocalImportOptions::default()
        };
        assert!(!format!("{password_options:?}").contains("fixture-pass"));
        assert!(!serde_json::to_string(&password_options)
            .unwrap()
            .contains("pdfPassword"));

        let expected_colors = [[217, 65, 65], [52, 168, 83], [53, 105, 212]];
        for (index, chapter) in comic_book.chapters.iter().enumerate() {
            let chapter_url =
                reqwest::Url::parse(&server.url_for(chapter.src.as_ref().unwrap())).unwrap();
            let html = reqwest::get(chapter_url.clone())
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .text()
                .await
                .unwrap();
            let image_relative = html
                .split("src=\"../assets/")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let image_url = chapter_url
                .join(&format!("../assets/{image_relative}"))
                .unwrap();
            let response = reqwest::get(image_url)
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            assert_eq!(
                response.headers()[reqwest::header::CONTENT_TYPE],
                "image/png"
            );
            let image = image::load_from_memory(&response.bytes().await.unwrap()).unwrap();
            assert_eq!(image.dimensions(), (640, 900));
            let center = image.get_pixel(20, 20).0;
            assert_eq!(&center[..3], &expected_colors[index]);
        }

        server.shutdown().await.unwrap();
        drop(store);
        let reopened_store = ResourceStore::open(&data_root).unwrap();
        for book in [&pdf_book, &encrypted_book, &comic_book] {
            let book_ref = reopened_store.book_ref(&book.id).unwrap();
            let serialized = reopened_store.read_json_ref(&book_ref).await.unwrap();
            let reopened: crate::models::BookDocument = serde_json::from_value(serialized).unwrap();
            assert_eq!(reopened.chapter_count, book.chapter_count);
        }

        let reopened_server = reopened_store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let reopened_cbz = reopened_store
            .read_json_ref(&reopened_store.book_ref(&comic_book.id).unwrap())
            .await
            .unwrap();
        let first_chapter_ref: crate::resources::ResourceRef =
            serde_json::from_value(reopened_cbz["chapters"][0]["src"].clone()).unwrap();
        let first_chapter_url =
            reqwest::Url::parse(&reopened_server.url_for(&first_chapter_ref)).unwrap();
        let first_html = reqwest::get(first_chapter_url.clone())
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(first_html.contains("alt=\"page1\""));
        let first_image = first_html
            .split("src=\"../assets/")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let first_image_url = first_chapter_url
            .join(&format!("../assets/{first_image}"))
            .unwrap();
        let image = reqwest::get(first_image_url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let decoded = image::load_from_memory(&image).unwrap();
        assert_eq!(decoded.dimensions(), (640, 900));
        assert_eq!(&decoded.get_pixel(20, 20).0[..3], &expected_colors[0]);
        reopened_server.shutdown().await.unwrap();
    }

    #[test]
    fn pdf_page_limit_and_damaged_file_are_reported() {
        assert!(validate_pdf_page_count(super::MAX_PDF_PAGES).is_ok());
        assert!(validate_pdf_page_count(super::MAX_PDF_PAGES + 1)
            .unwrap_err()
            .contains("page import limit"));
        assert!(validate_pdf_page_count(0).is_err());
        let damaged =
            super::parse_pdf_file(b"not a pdf".to_vec(), std::path::Path::new("bad.pdf"), None)
                .unwrap_err();
        assert!(damaged.contains("damaged"));
    }

    #[tokio::test]
    async fn imports_utf8_bom_txt_and_serves_each_chapter_as_html() {
        assert!(super::default_toc_regex().is_match("第一章 初遇"));
        assert!(!super::default_toc_regex().is_match("第一章正文。"));
        let temporary = tempfile::tempdir().unwrap();
        let data_root = temporary.path().join("app-data");
        let store = ResourceStore::open(&data_root).unwrap();
        let text_path = temporary.path().join("Fixture.txt");
        let fixture_text =
            "序言\n欢迎阅读。\n\n第一章 初遇\n第一章正文。\n\n第二章 重逢\n第二章正文。\n";
        std::fs::write(&text_path, format!("\u{feff}{fixture_text}")).unwrap();

        let book = import_local_book(
            &store,
            &text_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(book.title, "Fixture");
        assert_eq!(book.chapter_count, 3);
        assert_eq!(book.chapters[0].title, "序章");
        assert_eq!(book.chapters[1].title, "第一章 初遇");
        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let chapter_url = server.url_for(book.chapters[1].src.as_ref().unwrap());
        let response = reqwest::get(chapter_url).await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let html = response.text().await.unwrap();
        assert!(html.contains("第一章正文。"));
        assert!(html.contains("--reader-font-size:19px"));
    }

    #[tokio::test]
    async fn imports_epub_spine_order_and_serves_sanitized_html_and_media() {
        let temporary = tempfile::tempdir().unwrap();
        let data_root = temporary.path().join("app-data");
        let store = ResourceStore::open(&data_root).unwrap();
        let epub_path = temporary.path().join("Fixture.epub");
        let file = std::fs::File::create(&epub_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, content) in [
            (
                "META-INF/container.xml",
                r#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/book.opf"/></rootfiles></container>"#,
            ),
            (
                "OEBPS/book.opf",
                r#"<?xml version="1.0"?><package><metadata><dc:title>Fixture EPUB</dc:title><dc:creator>A Writer</dc:creator><meta name="cover" content="cover"/></metadata><manifest><item id="cover" href="Images/cover.png" media-type="image/png"/><item id="style" href="Styles/book.css" media-type="text/css"/><item id="later" href="Text/second.xhtml" media-type="application/xhtml+xml"/><item id="first" href="Text/first.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="later"/><itemref idref="first"/></spine></package>"#,
            ),
            (
                "OEBPS/Styles/book.css",
                "body { color: #123456; } .picture { background-image: url('../Images/cover.png'); } @import 'https://invalid.example/x.css';",
            ),
            (
                "OEBPS/Text/second.xhtml",
                r#"<?xml version="1.0"?><html><head><title>Second</title><link rel="stylesheet" href="../Styles/book.css"/></head><body><h1>Second Chapter</h1><img src="../Images/cover.png"/><script>alert('bad')</script></body></html>"#,
            ),
            (
                "OEBPS/Text/first.xhtml",
                r#"<html><head><title>First</title></head><body><p>First in spine.</p></body></html>"#,
            ),
        ] {
            zip.start_file(name, deflated).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==")
            .unwrap();
        zip.start_file("OEBPS/Images/cover.png", stored).unwrap();
        zip.write_all(&png).unwrap();
        zip.finish().unwrap();

        let book = import_local_book(
            &store,
            &epub_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(book.title, "Fixture EPUB");
        assert_eq!(book.author, "A Writer");
        assert_eq!(
            book.chapters
                .iter()
                .map(|chapter| chapter.title.as_str())
                .collect::<Vec<_>>(),
            ["Second", "First"]
        );
        let cover = book.cover_src.as_ref().unwrap();
        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let chapter_url =
            reqwest::Url::parse(&server.url_for(book.chapters[0].src.as_ref().unwrap())).unwrap();
        let chapter_response = reqwest::get(chapter_url.clone()).await.unwrap();
        let html = chapter_response.text().await.unwrap();
        assert!(html.contains("../assets/epub-"));
        assert!(!html.contains("<script"));
        assert!(html.contains("rel=\"stylesheet\""));

        let cover_url = reqwest::Url::parse(&server.url_for(cover)).unwrap();
        let image_rel = html
            .split("src=\"../assets/")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let resolved_image_url = chapter_url.join(&format!("../assets/{image_rel}")).unwrap();
        assert_eq!(resolved_image_url, cover_url);
        let image_response = reqwest::get(resolved_image_url).await.unwrap();
        assert_eq!(image_response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            image_response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .unwrap(),
            "image/png"
        );
        let image = image_response.bytes().await.unwrap();
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(u32::from_be_bytes(image[16..20].try_into().unwrap()), 1);
        assert_eq!(u32::from_be_bytes(image[20..24].try_into().unwrap()), 1);

        let css_id = html
            .split("href=\"../assets/")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let css_ref = store.asset_ref(&book.id, css_id).unwrap();
        let css_url = reqwest::Url::parse(&server.url_for(&css_ref)).unwrap();
        let css_response = reqwest::get(css_url.clone()).await.unwrap();
        let css = css_response.text().await.unwrap();
        let css_image_rel = css
            .split("url(\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let resolved_css_image = css_url.join(css_image_rel).unwrap();
        assert_eq!(resolved_css_image, cover_url);
        let css_image_response = reqwest::get(resolved_css_image).await.unwrap();
        assert_eq!(css_image_response.status(), reqwest::StatusCode::OK);
        assert!(css_image_response
            .bytes()
            .await
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!css.contains("invalid.example"));
    }

    #[tokio::test]
    async fn imports_gbk_unicode_text_without_toc_and_preserves_progress_on_duplicate_import() {
        let temporary = tempfile::tempdir().unwrap();
        let data_root = temporary.path().join("app-data");
        let store = ResourceStore::open(&data_root).unwrap();
        let text_path = temporary.path().join("《星海》.txt");
        let source = format!(
            "《星海》\n{}",
            "没有目录标题，中文正文完整保留。\n".repeat(12_000)
        );
        let (encoded, _, had_errors) = GBK.encode(&source);
        assert!(!had_errors);
        std::fs::write(&text_path, encoded.as_ref()).unwrap();

        let first = import_local_book(
            &store,
            &text_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(first.title, "《星海》");
        assert_eq!(first.chapter_count, 1);
        assert_eq!(first.chapters[0].title, "正文");
        let chapter = store
            .read_json_ref(&store.book_ref(&first.id).unwrap())
            .await
            .unwrap();
        assert_eq!(chapter["chapters"][0]["title"], "正文");
        let progress = ProgressDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            book_id: first.id.clone(),
            chapter_id: Some(first.chapters[0].id.clone()),
            chapter_index: 0,
            offset: 456,
            updated_at_ms: 123456,
        };
        store.write_progress(&first.id, &progress).await.unwrap();

        let second = import_local_book(
            &store,
            &text_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(second.id, first.id);
        let saved_progress = store
            .read_json_ref(&store.progress_ref(&first.id).unwrap())
            .await
            .unwrap();
        assert_eq!(saved_progress["offset"], 456);
        assert_eq!(saved_progress["updatedAtMs"], 123456);
    }

    #[tokio::test]
    async fn persisted_txt_toc_rules_drive_import_after_reopen_and_allow_one_off_override() {
        let temporary = tempfile::tempdir().unwrap();
        let data_root = temporary.path().join("app-data");
        let store = ResourceStore::open(&data_root).unwrap();
        let rule = txt_toc_rules::TxtTocRule {
            id: "volume-lines".to_owned(),
            name: "Volume headings".to_owned(),
            rule: r"^VOLUME\s+\d+:\s+(.{1,120})$".to_owned(),
            example: Some("VOLUME 1: The First Dawn".to_owned()),
            serial_number: 0,
            enable: true,
        };
        txt_toc_rules::upsert_rule(&store, rule).await.unwrap();
        drop(store);

        let store = ResourceStore::open(&data_root).unwrap();
        let custom_path = checked_in_fixture("custom-toc.txt");
        let custom_book = import_local_book(
            &store,
            &custom_path,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            custom_book
                .chapters
                .iter()
                .map(|chapter| chapter.title.as_str())
                .collect::<Vec<_>>(),
            ["序章", "The First Dawn", "The Last Light"]
        );
        assert_eq!(custom_book.chapter_count, 3);

        let server = store
            .start_http("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let chapter_url =
            reqwest::Url::parse(&server.url_for(custom_book.chapters[1].src.as_ref().unwrap()))
                .unwrap();
        let chapter_html = reqwest::get(chapter_url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(chapter_html.contains("The first chapter body belongs here."));
        assert!(!chapter_html.contains("VOLUME 1"));

        let override_options = LocalImportOptions {
            toc_regex: Some(r"^SCENE\s+\d+\s+/\s+(.{1,120})$".to_owned()),
            ..LocalImportOptions::default()
        };
        let override_book = import_local_book(
            &store,
            checked_in_fixture("toc-override.txt"),
            &ReaderDefaults::default(),
            &override_options,
        )
        .await
        .unwrap();
        assert_eq!(
            override_book
                .chapters
                .iter()
                .map(|chapter| chapter.title.as_str())
                .collect::<Vec<_>>(),
            ["The Bridge", "The Crossing"]
        );
        assert_eq!(
            txt_toc_rules::read_document(&store)
                .await
                .unwrap()
                .rules
                .len(),
            1
        );
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_persisted_toc_rule_blocks_txt_import_before_creating_book_resources() {
        let temporary = tempfile::tempdir().unwrap();
        let store = ResourceStore::open(temporary.path().join("app-data")).unwrap();
        let rules_ref = txt_toc_rules::resource_ref(&store).unwrap();
        store
            .write_json_ref(
                &rules_ref,
                &serde_json::json!({
                    "schemaVersion": 1,
                    "rules": [{
                        "id": "broken-regex",
                        "name": "Broken",
                        "rule": "(",
                        "serialNumber": 0,
                        "enable": true,
                    }],
                }),
            )
            .await
            .unwrap();
        let fixture = checked_in_fixture("custom-toc.txt");
        let bytes = std::fs::read(&fixture).unwrap();
        let identity = super::local_identity(&fixture, &bytes, &LocalImportOptions::default());
        let error = import_local_book(
            &store,
            &fixture,
            &ReaderDefaults::default(),
            &LocalImportOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("Invalid TXT TOC expression"));
        assert!(!store
            .root()
            .join(format!("books/local-{identity}/book.json"))
            .exists());
    }

    #[test]
    fn epub_href_resolution_rejects_archive_escape_but_accepts_parent_relative_asset() {
        assert_eq!(
            super::normalize_epub_href("OPS/Text/part.xhtml", "../../Images/a.png").as_deref(),
            Some("Images/a.png")
        );
        assert_eq!(
            super::normalize_epub_href("OPS/Text/part.xhtml", "../../../outside").as_deref(),
            None
        );
        assert_eq!(
            super::normalize_epub_href("OPS/Text/part.xhtml", "file:///etc/passwd"),
            None
        );
    }
}
