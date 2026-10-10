//! PDF 页数、元数据与密码保护文档解析。

use std::path::Path;

use lopdf::{Dictionary, Document as PdfDocument, LoadOptions, Object};

use super::{ParsedAsset, ParsedBook, ParsedChapter, common::short_hash};

pub(super) const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PDF_DECOMPRESSED_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 10_000;
pub(super) fn parse_pdf_file(
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
