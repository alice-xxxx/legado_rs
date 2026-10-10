//! CBZ 图片页归档读取与逐页校验。

use std::{io::Cursor, path::Path};

use image::{ImageFormat, ImageReader, Limits};
use zip::ZipArchive;

use super::{
    ParsedAsset, ParsedBook, ParsedChapter,
    archive::{index_zip_members, read_archive_member},
    common::short_hash,
    rewrite::escape_html_attr,
};

pub(super) const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CBZ_EXPANDED_BYTES: u64 = 768 * 1024 * 1024;
const MAX_CBZ_FILES: usize = 100_000;
const MAX_CBZ_PAGES: usize = 20_000;
const MAX_CBZ_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
pub(super) fn parse_cbz_file(path: &Path, bytes: &[u8]) -> Result<ParsedBook, String> {
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

    let title = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("CBZ Book")
        .chars()
        .take(200)
        .collect::<String>();

    let mut assets = Vec::with_capacity(pages.len());
    let mut page_markup = String::new();
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
        page_markup.push_str(&format!(
            r#"<img src="../assets/{id}" alt="{}">"#,
            escape_html_attr(&page_name)
        ));
    }

    // A CBZ archive is one comic consumption unit whose ordered images are
    // pages. Pages are not application chapters/segments and therefore should
    // not pollute the catalog, prefetch or progress model.
    let chapters = vec![ParsedChapter {
        title: title.clone(),
        html: Some(page_markup),
        text: None,
        pdf_page_index: None,
    }];

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
