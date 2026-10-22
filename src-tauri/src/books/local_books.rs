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
#[path = "local_books/rewrite.rs"]
mod rewrite;
#[path = "local_books/txt.rs"]
mod txt;


use std::path::Path;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    database::{Book, Chapter, Page},
    resources::Resources,
};

const MAX_EPUB_CHAPTERS: usize = 20_000;
/// Rust-side import options. The optional TOC pattern is a regular expression
/// over complete text lines; it is not a book-source rule and is never sent to
/// the source engine or WebView for execution.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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
}

#[derive(Debug)]
struct ParsedChapter {
    title: String,
    html: Option<String>,
    text: Option<String>,
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
}

/// 直接复制到最终缓存，再按格式处理这份缓存文件。
pub type ImportProgress = Arc<dyn Fn(&str, u64, u64) + Send + Sync>;

pub(crate) async fn import_local_book(
    database: &crate::database::Database,
    resources: &Resources,
    mut file: File,
    mut display_path: std::path::PathBuf,
    options: &LocalImportOptions,
    progress: ImportProgress,
) -> Result<String, String> {
    // Android URI 可能没有扩展名，直接读取文件句柄识别，不创建暂存文件。
    if !display_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "txt" | "epub" | "cbz" | "pdf"
            )
        })
    {
        let mut header = [0; 5];
        let length = file.read(&mut header).map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        let extension = if header[..length].starts_with(b"%PDF-") {
            "pdf"
        } else if header[..length].starts_with(b"PK") {
            let mut zip = zip::ZipArchive::new(&mut file).map_err(|error| error.to_string())?;
            if zip.by_name("META-INF/container.xml").is_ok() {
                "epub"
            } else {
                "cbz"
            }
        } else {
            "txt"
        };
        display_path.set_extension(extension);
        file.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
    }
    let original_extension = display_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| matches!(extension.as_str(), "txt" | "epub" | "cbz" | "pdf"))
        .ok_or_else(|| "Unsupported local book format".to_owned())?;
    let options = options.clone();
    let saved_toc_rules = if display_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        && options.toc_regex.is_none()
    {
        Some(txt_toc_rules::read_document(database).await?)
    } else {
        None
    };
    let identity_options = options.clone();
    let identity_rules = saved_toc_rules.clone();
    let identity_extension = original_extension.clone();
    let reading_progress = progress.clone();
    let (identity, file) = tokio::task::spawn_blocking(move || {
        let size = file.metadata().map_err(|error| error.to_string())?.len();
        let maximum = match identity_extension.as_str() {
            "txt" => txt::MAX_FILE_BYTES,
            "epub" => epub::MAX_FILE_BYTES,
            "cbz" => cbz::MAX_FILE_BYTES,
            _ => 512 * 1024 * 1024,
        };
        if size == 0 || size > maximum {
            return Err("Unsupported local file size".to_owned());
        }
        let identity = local_identity(
            &mut file,
            &identity_extension,
            &identity_options,
            identity_rules.as_ref(),
            &reading_progress,
        )?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((identity, file))
    })
    .await
    .map_err(|error| error.to_string())??;
    let book_id = format!("local-{identity}");
    let content_type = match original_extension.as_str() {
        "epub" => "html",
        "cbz" => "image",
        "pdf" => "pdf",
        _ => "text",
    };
    let exists = database
        .book_type(&book_id)
        .map_err(|e| e.to_string())?
        .is_some();
    let original = format!("{content_type}/{book_id}/original.{original_extension}");
    if !resources
        .cache
        .exists(&original)
        .await
        .map_err(|e| e.to_string())?
    {
        resources
            .cache
            .copy_file(&original, file, {
                let progress = progress.clone();
                move |copied, total| progress("copy", copied, total)
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    if exists {
        return Ok(book_id);
    }
    let result = async {
        if original_extension == "pdf" {
            // PDF 只登记原件；实际页数和打开文档由 Vue 阅读器处理。
            let title = display_path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("PDF");
            progress("save", 0, 1);
            database
                .import_book(Book {
                    id: book_id.clone(),
                    content_type: "pdf".into(),
                    title: Some(title.into()),
                    author: Some(String::new()),
                    url: Some("original.pdf".into()),
                    original: Some("original.pdf".into()),
                    page_count: Some(0),
                    ..Default::default()
                })
                .map_err(|e| e.to_string())?
                .await
                .map_err(|e| e.to_string())?;
            return Ok(book_id.clone());
        }
        progress("parse", 0, 0);
        let bytes = resources
            .cache
            .read_file(&original)
            .await
            .map_err(|error| error.to_string())?;
        let extension = original_extension.clone();
        let parsed = tokio::task::spawn_blocking(move || {
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
        persist_parsed_book(
            database,
            resources,
            &book_id,
            content_type,
            parsed,
            &original_extension,
            &progress,
        )
        .await
    }
    .await;
    match result {
        Ok(book) => Ok(book),
        Err(error) => {
            let protected = crate::resources::protected_resources(resources.root())
                .map_err(|e| e.to_string())?;
            let _ = database
                .remove_book_assets(book_id.clone(), protected)
                .map_err(|e| e.to_string())?
                .await;
            Err(error)
        }
    }
}

async fn persist_parsed_book(
    database: &crate::database::Database,
    resources: &Resources,
    book_id: &str,
    content_type: &str,
    parsed: ParsedBook,
    original_extension: &str,
    progress: &ImportProgress,
) -> Result<String, String> {
    let mut cover_src = None;
    let total = (parsed.assets.len() + parsed.chapters.len()) as u64;
    for (index, asset) in parsed.assets.iter().enumerate() {
        progress("cache", index as u64, total);
        let reference = resources
            .cache
            .save_asset(content_type, book_id, &asset.id, &asset.bytes)
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
        progress("cache", (parsed.assets.len() + index) as u64, total);
        let chapter_id = format!("chapter-{:05}", index + 1);
        let is_rich_text = parsed_chapter.html.is_some();
        if original_extension == "cbz" {
            let pages = parsed
                .assets
                .iter()
                .enumerate()
                .map(|(page_index, asset)| Page {
                    id: format!("p{}", page_index + 1),
                    src: asset.id.clone(),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            chapters.push(Chapter {
                id: chapter_id,
                title: parsed_chapter.title,
                index,
                pages,
                ..Default::default()
            });
            continue;
        }
        let src = if let Some(html) = parsed_chapter.html {
            crate::book_files::html(
                resources,
                content_type,
                book_id,
                &chapter_id,
                &html,
                None,
                true,
            )
            .await
        } else {
            crate::book_files::text(
                resources,
                content_type,
                book_id,
                &chapter_id,
                parsed_chapter.text.as_deref().unwrap_or_default(),
            )
            .await
        }
        .map_err(|error| error.to_string())?;
        let content_format = if is_rich_text { "richText" } else { "text" };
        chapters.push(Chapter {
            id: chapter_id,
            title: parsed_chapter.title,
            index,
            resource: Some(crate::book_files::ChapterResourceDescriptor::for_chapter(
                &src,
                content_format,
            )),
            ..Default::default()
        });
    }

    progress("cache", total, total);
    let book = Book {
        id: book_id.to_owned(),
        content_type: content_type.to_owned(),
        title: Some(parsed.title),
        author: Some(parsed.author),
        cover: cover_src.map(|reference| reference.to_string()),
        intro: parsed.intro,
        chapters,
        original: Some(format!("original.{original_extension}")),
        ..Default::default()
    };
    progress("save", 0, 1);
    database
        .import_book(book)
        .map_err(|error| error.to_string())?
        .await
        .map_err(|error| error.to_string())?;
    Ok(book_id.to_owned())
}

fn parse_selected_file(
    path: &Path,
    extension: &str,
    bytes: Vec<u8>,
    options: &LocalImportOptions,
    saved_toc_rules: Option<&txt_toc_rules::TxtTocRulesDocument>,
) -> Result<ParsedBook, String> {
    match extension {
        "txt" => txt::parse_text_file(&path, &bytes, options, saved_toc_rules),
        "epub" => epub::parse_epub_file(&bytes),
        "cbz" => cbz::parse_cbz_file(&path, &bytes),
        _ => unreachable!("extension was checked above"),
    }
}

fn local_identity(
    file: &mut File,
    extension: &str,
    options: &LocalImportOptions,
    rules: Option<&txt_toc_rules::TxtTocRulesDocument>,
    progress: &ImportProgress,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let total = file.metadata().map_err(|error| error.to_string())?.len();
    let mut read = 0;
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        read += count as u64;
        if read % (1024 * 1024) < count as u64 || read == total {
            progress("read", read, total);
        }
    }
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
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
