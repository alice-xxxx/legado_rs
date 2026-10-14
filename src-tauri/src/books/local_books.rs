//! 本地 TXT、EPUB、CBZ 与 PDF 的导入入口。
//!
//! WebView 只接收 Rust 准备好的书籍和章节资源；文件内容不经 IPC，也不执行书源规则。

#[path = "txt_toc_rules.rs"]
pub mod txt_toc_rules;

#[path = "local_books/archive.rs"]
mod archive;
#[path = "local_books/cbz.rs"]
mod cbz;
#[path = "local_books/common.rs"]
mod common;
#[path = "local_books/epub.rs"]
mod epub;
#[path = "local_books/pdf.rs"]
mod pdf;
#[path = "local_books/rewrite.rs"]
mod rewrite;
#[path = "local_books/txt.rs"]
mod txt;

#[cfg(test)]
#[path = "local_books/import_tests.rs"]
mod import_tests;

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    models::{BookDocument, ChapterDescriptor, ProgressSummary, CURRENT_SCHEMA_VERSION},
    resources::ResourceStore,
};

const MAX_PDF_PASSWORD_BYTES: usize = 1024;
const MAX_EPUB_CHAPTERS: usize = 20_000;
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
    title: String,
    author: String,
    intro: Option<String>,
    chapters: Vec<ParsedChapter>,
    assets: Vec<ParsedAsset>,
    cover_asset_id: Option<String>,
    pdf_asset_id: Option<String>,
}

/// Import an app-managed staged local file. Rust assigns a content ID and
/// stores the original before parsing; the picker path is never a reading URL.
pub(crate) struct LocalBookImportOutcome {
    pub(crate) book: BookDocument,
}

pub async fn import_local_book(
    store: &ResourceStore,
    selected_path: impl AsRef<Path>,
    identity_path: impl AsRef<Path>,
    options: &LocalImportOptions,
) -> Result<BookDocument, String> {
    Ok(
        import_local_book_with_outcome(store, selected_path, identity_path, options)
            .await?
            .book,
    )
}

pub(crate) async fn import_local_book_with_outcome(
    store: &ResourceStore,
    selected_path: impl AsRef<Path>,
    identity_path: impl AsRef<Path>,
    options: &LocalImportOptions,
) -> Result<LocalBookImportOutcome, String> {
    let selected_path = selected_path.as_ref().to_path_buf();
    let display_path = identity_path.as_ref().to_path_buf();
    let _import = store.local_import_guard().await;
    let original_extension = selected_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| matches!(extension.as_str(), "txt" | "epub" | "cbz" | "pdf"))
        .ok_or_else(|| "Unsupported local book format".to_owned())?;
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
    let read_path = selected_path.clone();
    let identity_options = options.clone();
    let identity_rules = saved_toc_rules.clone();
    let identity_extension = original_extension.clone();
    let identity = tokio::task::spawn_blocking(move || {
        let bytes = read_selected_file(&read_path)?;
        Ok::<_, String>(local_identity(
            &bytes,
            &identity_extension,
            &identity_options,
            identity_rules.as_ref(),
        ))
    })
    .await
    .map_err(|error| format!("Local book import worker failed: {error}"))??;
    let book_id = format!("local-{identity}");
    let content_type = match original_extension.as_str() {
        "epub" => "html",
        "cbz" => "image",
        "pdf" => "pdf",
        _ => "text",
    };
    store
        .bind_book_type(&book_id, content_type)
        .await
        .map_err(|error| error.to_string())?;
    // Re-importing the same file is idempotent. In particular, do not rewrite
    // the user's progress or cached chapters merely because the picker was
    // opened again for an already imported file.
    let book_ref = store
        .book_ref(&book_id)
        .map_err(|error| error.to_string())?;
    if store.root().join(book_ref.path()).is_file() {
        store
            .persist_local_original(&book_id, &original_extension, &selected_path)
            .await
            .map_err(|error| error.to_string())?;
        let existing = store
            .read_json_ref(&book_ref)
            .await
            .map_err(|error| error.to_string())?;
        let book = serde_json::from_value(existing)
            .map_err(|error| format!("Existing local book document is invalid: {error}"))?;
        return Ok(LocalBookImportOutcome { book });
    }
    let result = async {
        store
            .persist_local_original(&book_id, &original_extension, &selected_path)
            .await
            .map_err(|error| error.to_string())?;
        let original = store
            .root()
            .join("cache")
            .join(content_type)
            .join(&book_id)
            .join(format!("original.{original_extension}"));
        let extension = original_extension.clone();
        let parsed = tokio::task::spawn_blocking(move || {
            let bytes = read_selected_file(&original)?;
            parse_selected_file(
                &display_path,
                &extension,
                bytes,
                &options,
                saved_toc_rules.as_ref(),
            )
        })
        .await
        .map_err(|error| format!("Local book import worker failed: {error}"))??;
        persist_parsed_book(store, &book_id, parsed, &original_extension).await
    }
    .await;
    match result {
        Ok(book) => Ok(LocalBookImportOutcome { book }),
        Err(error) => {
            let _ = store.remove_book_resources(&book_id).await;
            Err(error)
        }
    }
}

async fn persist_parsed_book(
    store: &ResourceStore,
    book_id: &str,
    parsed: ParsedBook,
    original_extension: &str,
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
    let mut image_pages = Vec::new();
    for (index, parsed_chapter) in parsed.chapters.into_iter().enumerate() {
        let chapter_id = format!("chapter-{:05}", index + 1);
        let is_pdf_page = parsed.pdf_asset_id.is_some() && parsed_chapter.pdf_page_index.is_some();
        let is_rich_text = parsed_chapter.html.is_some();
        if original_extension == "cbz" {
            let pages = parsed.assets.iter().enumerate().map(|(page_index, asset)| {
                serde_json::json!({"id": format!("p{}", page_index + 1), "url": asset.id})
            }).collect::<Vec<_>>();
            image_pages.push(serde_json::json!(pages));
            chapters.push(ChapterDescriptor {
                id: chapter_id,
                title: parsed_chapter.title,
                index,
                resource: None,
            });
            continue;
        }
        let src = if let (Some(pdf_asset_id), Some(page_index)) = (
            parsed.pdf_asset_id.as_deref(),
            parsed_chapter.pdf_page_index,
        ) {
            store
                .write_pdf_page(book_id, &chapter_id, pdf_asset_id, page_index, "page-fit")
                .await
        } else if let Some(html) = parsed_chapter.html {
            store
                .write_local_chapter_rich_text(book_id, &chapter_id, &html)
                .await
        } else {
            store
                .write_chapter_text(
                    book_id,
                    &chapter_id,
                    parsed_chapter.text.as_deref().unwrap_or_default(),
                )
                .await
        }
        .map_err(|error| error.to_string())?;
        let content_format = if is_pdf_page {
            "pdf"
        } else if is_rich_text {
            "richText"
        } else {
            "text"
        };
        chapters.push(ChapterDescriptor {
            id: chapter_id,
            title: parsed_chapter.title,
            index,
            resource: Some(crate::models::ChapterResourceDescriptor::for_chapter(
                &src,
                content_format,
            )),
        });
    }

    let latest_chapter = chapters.last().map(|chapter| chapter.title.clone());
    let local_cover = cover_src.as_ref().map(ToString::to_string);
    let display_base = crate::book_display_metadata::initial_base(
        &parsed.title,
        &parsed.author,
        None,
        local_cover.as_deref(),
    )?;
    let book = BookDocument {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: book_id.to_owned(),
        title: parsed.title,
        can_change_source: false,
        author: parsed.author,
        cover_src,
        intro: parsed.intro,
        kind: (original_extension == "cbz").then(|| "comic".to_owned()),
        content_type: Some(
            match original_extension {
                "cbz" => "image",
                "epub" => "html",
                "pdf" => "pdf",
                _ => "text",
            }
            .to_owned(),
        ),
        media_type: None,
        word_count: None,
        source_id: None,
        source_name: None,
        source_group: None,
        display_base: Some(display_base),
        display_overrides: None,
        chapter_count: chapters.len(),
        latest_chapter,
        progress: ProgressSummary::default(),
        chapters,
    };
    let mut document = serde_json::to_value(&book).map_err(|error| error.to_string())?;
    document["original"] = serde_json::json!(format!("original.{original_extension}"));
    if original_extension == "cbz" {
        for (index, pages) in image_pages.into_iter().enumerate() {
            document["chapters"][index]["pages"] = pages;
        }
    }
    store
        .write_json_ref(
            &store.book_ref(book_id).map_err(|error| error.to_string())?,
            &document,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(book)
}

fn read_selected_file(path: &Path) -> Result<Vec<u8>, String> {
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
    let max_size = match extension.as_str() {
        "txt" => txt::MAX_FILE_BYTES,
        "epub" => epub::MAX_FILE_BYTES,
        "cbz" => cbz::MAX_FILE_BYTES,
        "pdf" => pdf::MAX_FILE_BYTES,
        _ => return Err("Supported local book formats are TXT, EPUB, CBZ, and PDF".to_owned()),
    };
    if metadata.len() == 0 || metadata.len() > max_size {
        return Err(format!(
            "Selected file size must be between 1 byte and {max_size} bytes"
        ));
    }
    std::fs::read(&path).map_err(|error| format!("Cannot read selected file: {error}"))
}

fn parse_selected_file(
    path: &Path,
    extension: &str,
    bytes: Vec<u8>,
    options: &LocalImportOptions,
    saved_toc_rules: Option<&txt_toc_rules::TxtTocRulesDocument>,
) -> Result<ParsedBook, String> {
    if options
        .pdf_password
        .as_deref()
        .is_some_and(|password| password.len() > MAX_PDF_PASSWORD_BYTES)
    {
        return Err("PDF password exceeds the supported length limit".to_owned());
    }
    match extension {
        "txt" => txt::parse_text_file(&path, &bytes, options, saved_toc_rules),
        "epub" => epub::parse_epub_file(&bytes),
        "cbz" => cbz::parse_cbz_file(&path, &bytes),
        "pdf" => pdf::parse_pdf_file(bytes, &path, options.pdf_password.as_deref()),
        _ => unreachable!("extension was checked above"),
    }
}

fn local_identity(
    bytes: &[u8],
    extension: &str,
    options: &LocalImportOptions,
    rules: Option<&txt_toc_rules::TxtTocRulesDocument>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.update([0]);
    hasher.update(extension.as_bytes());
    hasher.update([0]);
    if let Ok(options) = serde_json::to_vec(options) {
        hasher.update(options);
    }
    if extension == "txt"
        && options.toc_regex.is_none()
        && rules.is_none_or(|rules| rules.rules.is_empty())
    {
        hasher.update(txt::default_toc_regex().as_str().as_bytes());
    }
    if let Some(rules) = rules {
        for rule in rules.rules.iter().filter(|rule| rule.enable) {
            hasher.update([0]);
            hasher.update(rule.rule.as_bytes());
            hasher.update(rule.serial_number.to_le_bytes());
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
