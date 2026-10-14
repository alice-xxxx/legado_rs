//! CBZ 图片页归档读取与逐页校验。

use std::{io::Cursor, path::Path};

use image::{ImageFormat, ImageReader, Limits};
use quick_xml::{events::Event, Reader};
use std::collections::HashMap;
use zip::ZipArchive;

use super::{
    archive::{index_zip_members, read_archive_member},
    ParsedAsset, ParsedBook, ParsedChapter,
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

    let fallback_title = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("CBZ Book")
        .chars()
        .take(200)
        .collect::<String>();
    let metadata = members
        .keys()
        .find(|name| {
            name.rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("ComicInfo.xml"))
        })
        .and_then(|name| read_archive_member(&mut archive, &members, name, 1024 * 1024, "CBZ").ok())
        .and_then(|bytes| comic_metadata(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default();
    let chapter_title = metadata
        .get("Title")
        .cloned()
        .unwrap_or_else(|| fallback_title.clone());
    let title = match metadata.get("Series") {
        Some(series) if series != &chapter_title => format!("{series} · {chapter_title}"),
        Some(series) => series.clone(),
        None => fallback_title,
    }
    .chars()
    .take(200)
    .collect();

    let mut assets = Vec::with_capacity(pages.len());
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
        let decoded = decoder
            .decode()
            .map_err(|_| format!("CBZ page image is invalid or exceeds decoder limits: {name}"))?;

        let webp = if format == ImageFormat::WebP {
            image_bytes
        } else {
            let mut webp = Cursor::new(Vec::new());
            decoded
                .write_to(&mut webp, ImageFormat::WebP)
                .map_err(|error| format!("Cannot cache CBZ page as WebP: {error}"))?;
            webp.into_inner()
        };
        let id = format!("c001_p{:03}.webp", index + 1);
        if index == 0 {
            cover_asset_id = Some(id.clone());
        }
        assets.push(ParsedAsset {
            id: id.clone(),
            bytes: webp,
        });
    }

    // A CBZ archive is one comic consumption unit whose ordered images are
    // pages. Pages are not application chapters/segments and therefore should
    // not pollute the catalog, prefetch or progress model.
    let chapters = vec![ParsedChapter {
        title: chapter_title,
        html: None,
        text: None,
        pdf_page_index: None,
    }];

    Ok(ParsedBook {
        title,
        author: metadata
            .get("Writer")
            .cloned()
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect(),
        intro: metadata.get("Summary").cloned(),
        chapters,
        assets,
        cover_asset_id,
        pdf_asset_id: None,
    })
}

fn comic_metadata(xml: &str) -> Option<HashMap<String, String>> {
    let mut reader = Reader::from_str(xml);
    let mut values = HashMap::new();
    let mut field = String::new();
    let mut text = String::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(element)
                if matches!(
                    element.local_name().as_ref(),
                    "Title" | "Series" | "Writer" | "Summary"
                ) =>
            {
                field = element.local_name().as_ref().to_owned();
                text.clear();
            }
            Event::Text(value) if !field.is_empty() => {
                text.push_str(&quick_xml::escape::unescape(value.as_ref()).ok()?)
            }
            Event::CData(value) if !field.is_empty() => text.push_str(value.as_ref()),
            Event::GeneralRef(value) if !field.is_empty() => {
                text.push_str(&quick_xml::escape::unescape(&format!("&{};", value.as_ref())).ok()?)
            }
            Event::End(element) if element.local_name().as_ref() == field => {
                if !text.trim().is_empty() {
                    values.insert(field.clone(), text.trim().to_owned());
                }
                field.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Some(values)
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
