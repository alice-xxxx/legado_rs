use super::{
    txt_toc_rules::{TxtTocRule, TxtTocRulesDocument},
    LocalImportOptions,
};
use crate::{
    application::ApplicationService,
    source_engine::SourceEngineRequest,
    source_engine::{EngineFuture, SourceExecutor},
};
use serde_json::{json, Value};
use std::{
    io::{Cursor, Write},
    path::Path,
    sync::Arc,
};

struct Offline;
impl SourceExecutor for Offline {
    fn execute<'a>(&'a self, _: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async { panic!("Local books must not execute source rules") })
    }
}

async fn fixture() -> (tempfile::TempDir, ApplicationService) {
    let dir = tempfile::tempdir().unwrap();
    let service = ApplicationService::open_with_executor(dir.path(), Arc::new(Offline))
        .await
        .unwrap();
    (dir, service)
}

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(
            *name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

async fn import(service: &ApplicationService, file: &Path) -> Value {
    let response = service
        .import_book_file(
            std::fs::File::open(file).unwrap(),
            file.into(),
            LocalImportOptions::default(),
        )
        .await
        .unwrap();
    imported_book(service, &response)
}

fn imported_book(service: &ApplicationService, response: &str) -> Value {
    let id = response;
    let stored = service.database.book(id).unwrap().unwrap();
    serde_json::to_value(stored).unwrap()
}

async fn chapter(service: &ApplicationService, book: &Value, index: usize) -> String {
    let reference =
        crate::resources::ResourceRef::new(book["chapters"][index]["url"].as_str().unwrap())
            .unwrap();
    let response = reqwest::get(service.resources().url_for(&reference))
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.text().await.unwrap()
}

#[tokio::test]
async fn txt_uses_saved_rules_and_reimport_preserves_progress_and_groups() {
    let (dir, service) = fixture().await;
    super::txt_toc_rules::replace_document(
        &service.database,
        &TxtTocRulesDocument {
            schema_version: 1,
            rules: vec![TxtTocRule {
                id: "custom".into(),
                name: "自定义".into(),
                rule: r"^【.+】$".into(),
                example: None,
                serial_number: 0,
                enable: true,
            }],
        },
    )
    .await
    .unwrap();
    let file = dir.path().join("目录规则.txt");
    std::fs::write(&file, "前言\n【开场】\n第一段。\n【结尾】\n第二段。").unwrap();
    let book = import(&service, &file).await;
    let id = book["id"].as_str().unwrap();
    assert_eq!(book["chapters"].as_array().unwrap().len(), 3);
    assert_eq!(book["chapters"][1]["title"], "【开场】");
    assert_eq!(chapter(&service, &book, 1).await, "第一段。");
    service.save_progress(id, json!({"chapterId": "chapter-00002", "chapterIndex": 1, "offset": 3, "updatedAtMs": 1234})).await.unwrap();
    service.database.submit(serde_json::from_value(json!([
        {"operation":"saveShelf","shelf":{"schemaVersion":1,"books":[{"id":id,"groups":["本地"]}],"groups":["本地"]}}
    ])).unwrap()).unwrap().await.unwrap();
    let copy = dir.path().join("另一文件名.txt");
    std::fs::copy(&file, &copy).unwrap();
    let again = import(&service, &copy).await;
    assert_eq!(again["id"], id);
    assert_eq!(again["progress"]["offset"], 3);
    service.database.assert_shelf(&[id]);
    service.database.assert_shelf_groups(id, &["本地"]);
    assert!(dir
        .path()
        .join("cache/text")
        .join(id)
        .join("original.txt")
        .is_file());
    let raw = service.database.book(id).unwrap().unwrap();
    let raw = serde_json::to_value(raw).unwrap();
    assert!(!dir
        .path()
        .join("cache/text")
        .join(id)
        .join(format!("{id}.json"))
        .exists());
    assert!(raw["chapters"][1]["url"]
        .as_str()
        .unwrap()
        .ends_with(".txt"));
    assert!(raw["chapters"][1]["url"]
        .as_str()
        .unwrap()
        .starts_with("resource://cache/text/"));
}

#[tokio::test]
async fn txt_without_headings_splits_utf16_and_rejects_blank_input() {
    let (dir, service) = fixture().await;
    let file = dir.path().join("长文.txt");
    let text = "纯文字内容。".repeat(3000);
    let bytes = [
        vec![0xff, 0xfe],
        text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    ]
    .concat();
    std::fs::write(&file, bytes).unwrap();
    let book = import(&service, &file).await;
    assert!(book["chapters"].as_array().unwrap().len() > 1);
    let mut actual = String::new();
    for index in 0..book["chapters"].as_array().unwrap().len() {
        actual.push_str(&chapter(&service, &book, index).await);
    }
    assert_eq!(actual, text);
    let blank = dir.path().join("空白.txt");
    std::fs::write(&blank, " \n\t").unwrap();
    assert!(service
        .import_book_file(
            std::fs::File::open(&blank).unwrap(),
            blank.clone(),
            LocalImportOptions::default()
        )
        .await
        .is_err());
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/text"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn epub_navigation_splits_shared_xhtml_and_keeps_assets_and_metadata() {
    let (dir, service) = fixture().await;
    let epub = archive(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#),
        ("OPS/book.opf", br#"<package><metadata><title>Test &amp; Book</title><creator>Author</creator><description>Intro</description></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="body" href="body.xhtml" media-type="application/xhtml+xml"/><item id="pic" href="pic.png" media-type="image/png" properties="cover-image"/></manifest><spine><itemref idref="body"/></spine></package>"#),
        ("OPS/nav.xhtml", br#"<html><body><nav xmlns:epub="http://www.idpf.org/2007/ops" epub:type="toc"><ol><li><a href="body.xhtml#one">First</a></li><li><a href="body.xhtml#two">Second</a></li></ol></nav></body></html>"#),
        ("OPS/body.xhtml", br##"<html><head><title>Shared file</title></head><body><h1 id="one">First</h1><p>Alpha <em>emphasis</em></p><img src="pic.png"/><a href="#two">Next</a><h1 id="two">Second</h1><p>Beta</p><script>bad()</script></body></html>"##),
        ("OPS/pic.png", b"asset-fixture"),
    ]);
    let file = dir.path().join("fixture.epub");
    std::fs::write(&file, epub).unwrap();
    let book = import(&service, &file).await;
    assert_eq!(book["title"], "Test & Book");
    assert_eq!(book["intro"], "Intro");
    assert_eq!(book["contentType"], "html");
    assert_eq!(book["chapters"].as_array().unwrap().len(), 2);
    let first = chapter(&service, &book, 0).await;
    let second = chapter(&service, &book, 1).await;
    assert!(first.contains("<em>emphasis</em>") && !first.contains("Beta"));
    assert!(first.contains("data-legado-chapter-id=\"chapter-00002\""));
    assert!(!first.contains("Shared file") && second.contains("Beta") && !second.contains("bad()"));
    let image = regex::Regex::new(r#"<img src="([^"]+)""#)
        .unwrap()
        .captures(&first)
        .unwrap()[1]
        .to_owned();
    assert_eq!(
        reqwest::get(image)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .as_ref(),
        b"asset-fixture"
    );
    assert!(reqwest::get(
        service
            .resources()
            .url_for(&crate::resources::ResourceRef::new(book["cover"].as_str().unwrap()).unwrap())
    )
    .await
    .unwrap()
    .status()
    .is_success());
    service
        .database
        .assert_shelf(&[book["id"].as_str().unwrap()]);
}

#[tokio::test]
async fn cbz_caches_naturally_ordered_webp_pages_in_one_chapter() {
    let (dir, service) = fixture().await;
    let png = |color| {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    };
    let red = png([255, 0, 0, 255]);
    let green = png([0, 255, 0, 255]);
    let blue = png([0, 0, 255, 255]);
    let file = dir.path().join("漫画.cbz");
    std::fs::write(
        &file,
        archive(&[("10.png", &blue), ("2.png", &green), ("1.png", &red)]),
    )
    .unwrap();
    let book = import(&service, &file).await;
    assert_eq!(book["contentType"], "image");
    assert_eq!(book["chapters"].as_array().unwrap().len(), 1);
    let pages = book["chapters"][0]["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 3);
    for (index, color) in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
        .iter()
        .enumerate()
    {
        let url = pages[index]["url"].as_str().unwrap();
        assert!(url.ends_with(&format!("c001_p{:03}.webp", index + 1)));
        let reference = crate::resources::ResourceRef::new(url).unwrap();
        let bytes = reqwest::get(service.resources().url_for(&reference))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(
            image::load_from_memory(&bytes)
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)
                .0,
            *color
        );
    }
}

#[tokio::test]
async fn malformed_archive_leaves_no_library_entry_or_partial_cache() {
    let (dir, service) = fixture().await;
    for extension in ["epub", "cbz"] {
        let file = dir.path().join(format!("broken.{extension}"));
        std::fs::write(&file, b"broken").unwrap();
        assert!(service
            .import_book_file(
                std::fs::File::open(&file).unwrap(),
                file.clone(),
                LocalImportOptions::default()
            )
            .await
            .is_err());
    }
    service.database.assert_shelf(&[]);
    for kind in ["html", "image", "pdf"] {
        assert!(std::fs::read_dir(dir.path().join("cache").join(kind))
            .map_or(true, |dir| dir.count() == 0));
    }
}

#[tokio::test]
async fn epub_ncx_merges_spine_continuations_and_falls_back_when_toc_is_invalid() {
    let opf = br#"<package><metadata><title>NCX book</title></metadata><manifest><item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="a" href="a.xhtml" media-type="application/xhtml+xml"/><item id="b" href="b.xhtml" media-type="application/xhtml+xml"/><item id="c" href="c.xhtml" media-type="application/xhtml+xml"/></manifest><spine toc="toc"><itemref idref="a"/><itemref idref="b"/><itemref idref="c"/></spine></package>"#;
    let build = |toc: &[u8]| {
        archive(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="book.opf"/></rootfiles></container>"#),
        ("book.opf", opf), ("toc.ncx", toc),
        ("a.xhtml", b"<html><body><h1>One</h1><p>Start</p></body></html>"),
        ("b.xhtml", b"<html><body><p>Continuation</p></body></html>"),
        ("c.xhtml", b"<html><body><h1>Two</h1><p>End</p></body></html>"),
    ])
    };
    let valid = build(br#"<ncx><navMap><navPoint><navLabel><text>One</text></navLabel><content src="a.xhtml"/></navPoint><navPoint><navLabel><text>Two</text></navLabel><content src="c.xhtml"/></navPoint></navMap></ncx>"#);
    let parsed = super::epub::parse_epub_file(&valid).unwrap();
    assert_eq!(parsed.chapters.len(), 2);
    assert!(parsed.chapters[0]
        .html
        .as_ref()
        .unwrap()
        .contains("Continuation"));
    let invalid = build(br#"<ncx><navMap><navPoint><navLabel><text>Missing</text></navLabel><content src="missing.xhtml"/></navPoint></navMap></ncx>"#);
    assert_eq!(
        super::epub::parse_epub_file(&invalid)
            .unwrap()
            .chapters
            .len(),
        3
    );
}

#[test]
fn default_txt_rule_keeps_punctuated_titles_and_does_not_match_prose() {
    let parsed = super::txt::parse_text_file(
        Path::new("book.txt"),
        "第1章 疑问？\n第一章正文。\n第2章 惊叹！\n正文二。\nChapter 3 Why?\nBody".as_bytes(),
        &LocalImportOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(parsed.chapters.len(), 3);
    assert_eq!(parsed.chapters[0].title, "第1章 疑问？");
    assert_eq!(parsed.chapters[0].text.as_deref(), Some("第一章正文。"));
    assert_eq!(parsed.chapters[1].title, "第2章 惊叹！");
    assert_eq!(parsed.chapters[2].title, "Chapter 3 Why?");
}

fn pdf_document() -> lopdf::Document {
    use lopdf::{dictionary, Object};
    let mut doc = lopdf::Document::with_version("1.5");
    let pages = doc.new_object_id();
    let kids = (0..2).map(|_| Object::Reference(doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(),0.into(),200.into(),300.into()],
    }))).collect::<Vec<_>>();
    doc.objects.insert(
        pages,
        Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => 2}),
    );
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages});
    doc.trailer.set("Root", catalog);
    doc.trailer.set(
        "ID",
        vec![
            Object::string_literal("fixture-id-one"),
            Object::string_literal("fixture-id-two"),
        ],
    );
    doc
}

#[tokio::test]
async fn pdf_keeps_one_original_and_serves_page_index_and_range_requests() {
    let (dir, service) = fixture().await;
    let file = dir.path().join("pages.pdf");
    pdf_document().save(&file).unwrap();
    let book = import(&service, &file).await;
    assert_eq!(book["chapters"].as_array().unwrap().len(), 0);
    assert!(book["chapters"].as_array().unwrap().is_empty());
    let id = book["id"].as_str().unwrap();
    let folder = dir.path().join("cache/pdf").join(id);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    let raw = service.database.book(id).unwrap().unwrap();
    let raw = serde_json::to_value(raw).unwrap();
    assert_eq!(raw["url"], "original.pdf");
    assert_eq!(raw["pageCount"], 0);
    assert!(raw["chapters"].as_array().unwrap().is_empty());
    std::fs::remove_file(folder.join("original.pdf")).unwrap();
    assert_eq!(import(&service, &file).await["id"], id);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    let response = reqwest::Client::new()
        .get(
            service.resources().url_for(
                &crate::resources::ResourceRef::new(format!(
                    "resource://cache/pdf/{id}/original.pdf"
                ))
                .unwrap(),
            ),
        )
        .header("Range", "bytes=0-4")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"%PDF-");
    drop(service);
    let reopened = ApplicationService::open_with_executor(dir.path(), Arc::new(Offline))
        .await
        .unwrap();
    reopened.database.assert_shelf(&[&id]);
}

#[tokio::test]
async fn encrypted_pdf_is_copied_without_opening_it_or_requesting_a_password() {
    let (dir, service) = fixture().await;
    let file = dir.path().join("encrypted.pdf");
    let mut doc = pdf_document();
    let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V1 {
        document: &doc,
        owner_password: "owner-fixture",
        user_password: "user-fixture",
        permissions: lopdf::Permissions::all(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    doc.save(&file).unwrap();
    let book = import(&service, &file).await;
    let id = book["id"].as_str().unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("cache/pdf").join(id).join("original.pdf")).unwrap(),
        std::fs::read(file).unwrap()
    );
    let raw = service.database.book(id).unwrap().unwrap();
    let raw = serde_json::to_value(raw).unwrap();
    assert_eq!(raw["pageCount"], 0);
    assert!(book["chapters"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn direct_copy_reports_progress_and_imports_into_the_shelf_without_staging() {
    let (dir, service) = fixture().await;
    let source = dir.path().join("book.txt");
    let text = "第一章 开始\n内容。\n第二章 继续\n下一章。";
    std::fs::write(&source, text).unwrap();
    let (sender, mut updates) = tokio::sync::mpsc::unbounded_channel();
    let progress = Arc::new(move |stage: &str, completed: u64, total: u64| {
        sender.send((stage.to_owned(), completed, total)).unwrap();
    });
    let book = super::import_local_book(
        &service.database,
        service.resources(),
        std::fs::File::open(&source).unwrap(),
        source.clone(),
        &LocalImportOptions::default(),
        progress,
    )
    .await
    .unwrap();
    let stages = std::iter::from_fn(|| updates.try_recv().ok()).collect::<Vec<_>>();
    assert!(stages
        .iter()
        .any(|(stage, completed, total)| stage == "copy"
            && completed == total
            && *total == text.len() as u64));
    assert!(stages
        .iter()
        .any(|(stage, completed, total)| stage == "cache" && completed == total && *total > 0));
    assert_eq!(stages.last().unwrap().0, "save");
    let id = service
        .import_book_file(
            std::fs::File::open(&source).unwrap(),
            source.clone(),
            LocalImportOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(id, book);
    service.database.assert_shelf(&[&id]);
    assert_eq!(
        std::fs::read(dir.path().join("cache/text").join(id).join("original.txt")).unwrap(),
        text.as_bytes()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_imports_keep_distinct_books_and_deduplicate_the_same_file() {
    let (dir, service) = fixture().await;
    let one = dir.path().join("one.txt");
    let two = dir.path().join("two.txt");
    std::fs::write(&one, "第一章 开始\n第一本内容。\n第二章 继续\n下一段。").unwrap();
    std::fs::write(&two, "第一章 开始\n第二本内容。\n第二章 继续\n另一段。").unwrap();
    let (first, second, duplicate) = tokio::join!(
        import(&service, &one),
        import(&service, &two),
        import(&service, &one)
    );
    assert_ne!(first["id"], second["id"]);
    assert_eq!(first["id"], duplicate["id"]);
    assert_eq!(chapter(&service, &first, 0).await, "第一本内容。");
    assert_eq!(chapter(&service, &second, 0).await, "第二本内容。");
    service.database.assert_shelf(&[
        first["id"].as_str().unwrap(),
        second["id"].as_str().unwrap(),
    ]);
    for (book, source) in [(&first, &one), (&second, &two)] {
        let original = dir
            .path()
            .join("cache/text")
            .join(book["id"].as_str().unwrap())
            .join("original.txt");
        assert_eq!(
            std::fs::read(original).unwrap(),
            std::fs::read(source).unwrap()
        );
    }
}
