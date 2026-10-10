//! 章节列表准备和正文资源缓存。

use super::*;

struct VideoMediaCandidate {
    source_index: usize,
    label: String,
    url: Option<String>,
    headers: Option<HashMap<String, String>>,
}

struct PreparedVideoMediaOption {
    source_index: usize,
    label: String,
    url: Option<String>,
    headers: Vec<(String, String)>,
    media_format: Option<&'static str>,
    unavailable_reason: Option<&'static str>,
}

enum PreparedChapterMedia {
    Audio {
        url: String,
        headers: Vec<(String, String)>,
        media_format: &'static str,
    },
    Video {
        options: Vec<PreparedVideoMediaOption>,
        selected_index: usize,
    },
}

/// Book-owned typed payloads must stay inside the book's chapter namespace.
/// Never accept another book's resource merely because ResourceRef is valid.
pub(super) fn scoped_chapter_cache_ref(book_id: &str, raw: &str) -> Result<ResourceRef, String> {
    let reference = ResourceRef::new(raw).map_err(|error| error.to_string())?;
    let prefix = format!("books/{book_id}/chapters/");
    if !reference.is_local()
        || !reference.path().starts_with(&prefix)
        || !reference.path().ends_with(".json")
    {
        return Err("Chapter resource does not belong to the requested book".to_owned());
    }
    Ok(reference)
}

impl ApplicationService {
    /// Return the actual semantic format of a reusable chapter cache entry.
    ///
    /// The public book document must never infer content semantics from a JSON
    /// filename: text, rich text, media and PDF page descriptors are all JSON.
    /// Invalid or truncated cache files are treated as a cache miss so Rust can
    /// rebuild them instead of making the WebView diagnose storage failures.
    pub(super) async fn cached_chapter_format(
        &self,
        reference: &ResourceRef,
    ) -> Result<Option<&'static str>, String> {
        if !reference.is_local()
            || !self
                .store
                .regular_file_exists(reference)
                .await
                .map_err(|error| error.to_string())?
        {
            return Ok(None);
        }

        if !reference.path().ends_with(".json") {
            return Ok(None);
        }

        let document = match self.store.read_json_ref(reference).await {
            Ok(document) => document,
            Err(_) => return Ok(None),
        };
        let format = match document.get("kind").and_then(Value::as_str) {
            Some("text") if document.get("text").is_some_and(Value::is_string) => Some("text"),
            Some("richText") if document.get("markup").is_some_and(Value::is_string) => {
                Some("richText")
            }
            Some("media") => {
                let media_type = document.get("mediaType").and_then(Value::as_str);
                let media_format = document.get("format").and_then(Value::as_str);
                let src = document.get("src").and_then(Value::as_str);
                match (media_type, media_format, src) {
                    (Some("audio"), Some("direct" | "hls"), Some(src))
                        if ResourceRef::new(src).is_ok() =>
                    {
                        Some("audio")
                    }
                    (Some("video"), Some("direct" | "hls"), Some(src))
                        if ResourceRef::new(src).is_ok() =>
                    {
                        Some("video")
                    }
                    _ => None,
                }
            }
            Some("pdfPage") => {
                let src = document.get("src").and_then(Value::as_str);
                let page_index = document.get("pageIndex").and_then(Value::as_u64);
                let zoom = document.get("defaultZoom").and_then(Value::as_str);
                if src.is_some_and(|src| {
                    ResourceRef::new(src)
                        .is_ok_and(|reference| reference.is_local())
                }) && page_index.is_some()
                    && matches!(zoom, Some("page-fit" | "page-width" | "actual-size"))
                {
                    Some("pdf")
                } else {
                    None
                }
            }
            _ => None,
        };
        Ok(format)
    }

    pub async fn prepare_chapters(
        &self,
        book_id: &str,
        from_index: usize,
        count: usize,
    ) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        self.prepare_chapters_unlocked(book_id, from_index, count)
            .await
    }

    async fn prepare_chapters_unlocked(
        &self,
        book_id: &str,
        from_index: usize,
        count: usize,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let count = count.clamp(1, 50);
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let (target_chapter_ids, local_book) = {
            let _book_lock = self.book_lock(book_id).await;
            let public_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|error| error.to_string())?;
            let descriptors = public_book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            let end = from_index.saturating_add(count).min(descriptors.len());
            let target_chapter_ids = descriptors
                .iter()
                .skip(from_index.min(end))
                .take(end.saturating_sub(from_index.min(end)))
                .map(|chapter| {
                    chapter["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| "Chapter ID missing".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let local_book = public_book
                .get("sourceId")
                .and_then(Value::as_str)
                .is_none();
            (target_chapter_ids, local_book)
        };

        if local_book {
            let _book_lock = self.book_lock(book_id).await;
            let public_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|error| error.to_string())?;
            let descriptors = public_book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            for chapter_id in &target_chapter_ids {
                let chapter = descriptors
                    .iter()
                    .find(|chapter| chapter["id"].as_str() == Some(chapter_id.as_str()))
                    .ok_or_else(|| "Book catalog changed while checking local content".to_owned())?;
                let src = chapter
                    .get("resource")
                    .and_then(|resource| resource.get("resourceId"))
                    .and_then(Value::as_str)
                    .filter(|src| !src.is_empty())
                    .ok_or_else(|| "Local chapter resource is missing; re-import the original file".to_owned())?;
                let reference = scoped_chapter_cache_ref(book_id, src)?;
                if self.cached_chapter_format(&reference).await?.is_none() {
                    return Err(
                        "Local chapter resource is missing or invalid; re-import the original file"
                            .into(),
                    );
                }
            }
            return Ok(json!({
                "book": self.resource_descriptor(&book_ref),
                "prepared": 0,
                "bookId": book_id,
                "fromIndex": from_index
            }));
        }

        self.prepare_chapter_ids_unlocked(book_id, from_index, target_chapter_ids)
            .await
    }

    pub(super) async fn prepare_chapter_ids_unlocked(
        &self,
        book_id: &str,
        from_index: usize,
        target_chapter_ids: Vec<String>,
    ) -> Result<Value, String> {
        validate_id(book_id, "bookId")?;
        let book_ref = self
            .store
            .book_ref(book_id)
            .map_err(|error| error.to_string())?;
        let (source_id, engine_book, target_chapter_ids) = {
            let _book_lock = self.book_lock(book_id).await;
            let private = self
                .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                .await?;
            let source_id = private["sourceId"]
                .as_str()
                .ok_or_else(|| "Book source ID is missing".to_owned())?
                .to_owned();
            let engine_book = private
                .get("book")
                .cloned()
                .ok_or_else(|| "Book engine data is missing".to_owned())?;
            let public_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|error| error.to_string())?;
            let raw_chapters = private
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
            let descriptors = public_book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            if raw_chapters.len() != descriptors.len() {
                return Err(
                    "Book chapter directory is out of sync; refresh it before reading".into(),
                );
            }
            let mut unique_ids = HashSet::with_capacity(target_chapter_ids.len());
            for chapter_id in &target_chapter_ids {
                if !unique_ids.insert(chapter_id) {
                    return Err("Chapter request contains duplicate chapter IDs".to_owned());
                }
                if !descriptors
                    .iter()
                    .any(|chapter| chapter["id"].as_str() == Some(chapter_id.as_str()))
                {
                    return Err(
                        "Book catalog changed before the requested chapter could be prepared"
                            .into(),
                    );
                }
            }
            (source_id, engine_book, target_chapter_ids)
        };
        let mut prepared = 0usize;
        for chapter_id in target_chapter_ids {
            // Requests for the same uncached chapter share this lock. It stays
            // held over the source call while the book lock remains available
            // to progress saves and unrelated chapter cache commits.
            let _chapter_lock = self.chapter_lock(book_id, &chapter_id).await;
            let (raw_chapter, next_chapter_url, already_ready, saved_media_type) = {
                let _book_lock = self.book_lock(book_id).await;
                let private = self
                    .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                    .await?;
                if private["sourceId"].as_str() != Some(source_id.as_str()) {
                    return Err("Book source changed while preparing a chapter".to_owned());
                }
                if private.get("book") != Some(&engine_book) {
                    return Err("Book metadata changed while preparing a chapter".to_owned());
                }
                let mut public_book = self
                    .store
                    .read_json_ref(&book_ref)
                    .await
                    .map_err(|_| "Book was removed while preparing a chapter".to_owned())?;
                let media_type = public_book
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .filter(|value| matches!(*value, "audio" | "video"))
                    .map(str::to_owned);
                let index = public_book
                    .get("chapters")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "Book chapter directory is missing".to_owned())?
                    .iter()
                    .position(|chapter| chapter["id"].as_str() == Some(chapter_id.as_str()))
                    .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
                let raw_chapters = private
                    .get("chapters")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
                let raw_chapter = raw_chapters
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
                let next_chapter_url = raw_chapters
                    .get(index + 1)
                    .and_then(|chapter| text_at(chapter, &["url", "chapterUrl"]));
                let declared_ref = public_book["chapters"][index]
                    .get("resource")
                    .and_then(|resource| resource.get("resourceId"))
                    .and_then(Value::as_str)
                    .filter(|src| !src.is_empty())
                    .map(|src| scoped_chapter_cache_ref(book_id, src))
                    .transpose()?;
                let mut ready_ref = None;
                if let Some(reference) = declared_ref {
                    // A file's existence cannot establish that its typed payload is
                    // readable. Reuse only a resource validated by Rust, and repair
                    // obsolete descriptor semantics from the actual payload.
                    if let Some(content_format) = self.cached_chapter_format(&reference).await? {
                        let expected_descriptor = serde_json::to_value(
                            crate::models::ChapterResourceDescriptor::for_chapter(
                                &reference,
                                content_format,
                            ),
                        )
                        .map_err(|error| format!(
                            "Cannot encode recovered chapter resource descriptor: {error}"
                        ))?;
                        if public_book["chapters"][index].get("resource")
                            != Some(&expected_descriptor)
                        {
                            public_book["chapters"][index]["resource"] = expected_descriptor;
                            self.store
                                .write_json_ref(&book_ref, &public_book)
                                .await
                                .map_err(|error| error.to_string())?;
                        }
                        ready_ref = Some(reference);
                    }
                }
                (raw_chapter, next_chapter_url, ready_ref.is_some(), media_type)
            };

            // Look up the private source after releasing the book lock. Older
            // book JSON may not yet have a public mediaType, so classify it
            // from BookSourceType as a fallback.
            let source = self.find_source(&source_id).await?;
            let media_type = saved_media_type
                .or_else(|| crate::source_metadata::media_type(&source.source).map(str::to_owned));
            if already_ready && media_type.is_none() {
                continue;
            }

            // Clone the current source metadata for this request. Before
            // publishing the result, verify that the source has not been
            // replaced while the executor was working.
            let source_revision = self.source_revision(&source_id).await?;
            let content = self
                .executor
                .execute(engine_request(
                    if crate::rss::is_legacy_rss_source(&source.source) {
                        "rssContent"
                    } else {
                        "content"
                    },
                    &source.source,
                    None,
                    None,
                    Some(engine_book.clone()),
                    Some(raw_chapter.clone()),
                    next_chapter_url.clone(),
                ))
                .await?;
            let content = content
                .as_str()
                .ok_or_else(|| "Source engine returned non-text chapter content".to_owned())?;
            if content.trim().is_empty() {
                return Err(
                    "Source engine returned empty chapter content; no cache was written".to_owned(),
                );
            }
            let content_base_url = text_at(&raw_chapter, &["url", "chapterUrl"])
                .or_else(|| text_at(&engine_book, &["bookUrl", "url"]));
            let non_media_content_format =
                if looks_like_html(content) { "richText" } else { "text" };
            let media_request = if let Some(media_type) = media_type.as_deref() {
                if media_type == "video" {
                    let (candidates, default_index) = video_media_candidates(content)?;
                    let mut options = Vec::with_capacity(candidates.len());
                    for candidate in candidates {
                        let source_index = candidate.source_index;
                        let label = candidate.label;
                        let Some(media_input) = candidate.url.as_deref() else {
                            options.push(PreparedVideoMediaOption {
                                source_index,
                                label,
                                url: None,
                                headers: Vec::new(),
                                media_format: None,
                                unavailable_reason: Some(VIDEO_ADDRESS_UNAVAILABLE),
                            });
                            continue;
                        };
                        let mut request = engine_request(
                            "resolveMedia",
                            &source.source,
                            None,
                            None,
                            Some(engine_book.clone()),
                            Some(raw_chapter.clone()),
                            next_chapter_url.clone(),
                        );
                        request.media_url = Some(media_input.to_owned());
                        request.media_headers = candidate.headers;
                        let resolved = self.executor.execute(request).await;
                        let (url, headers, media_format, unavailable_reason) = match resolved
                            .ok()
                            .and_then(|resolved| resolved_media_result(&resolved).ok())
                        {
                            Some((url, headers)) if is_hls_media_url(&url) => {
                                let media_id =
                                    chapter_media_variant_id(book_id, &chapter_id, source_index, &url, &headers);
                                if self
                                    .server
                                    .validate_media_mapping(&media_id, &url, &headers)
                                    .is_ok()
                                {
                                    (Some(url), headers, Some(MEDIA_FORMAT_HLS), None)
                                } else {
                                    (None, Vec::new(), None, Some(VIDEO_RESOLUTION_UNAVAILABLE))
                                }
                            }
                            Some((url, headers)) if is_direct_http_media_url(&url) => {
                                let media_id =
                                    chapter_media_variant_id(book_id, &chapter_id, source_index, &url, &headers);
                                if self
                                    .server
                                    .validate_media_mapping(&media_id, &url, &headers)
                                    .is_ok()
                                {
                                    (Some(url), headers, Some(MEDIA_FORMAT_DIRECT), None)
                                } else {
                                    (None, Vec::new(), None, Some(VIDEO_RESOLUTION_UNAVAILABLE))
                                }
                            }
                            Some(_) => (None, Vec::new(), None, Some(VIDEO_ADDRESS_UNAVAILABLE)),
                            None => (None, Vec::new(), None, Some(VIDEO_RESOLUTION_UNAVAILABLE)),
                        };
                        options.push(PreparedVideoMediaOption {
                            source_index,
                            label,
                            url,
                            headers,
                            media_format,
                            unavailable_reason,
                        });
                    }
                    let selected_index = options
                        .iter()
                        .position(|option| {
                            option.source_index == default_index && option.url.is_some()
                        })
                        .or_else(|| options.iter().position(|option| option.url.is_some()))
                        .ok_or_else(|| {
                            "Video source has no playable HTTP media resolution".to_owned()
                        })?;
                    Some(PreparedChapterMedia::Video {
                        options,
                        selected_index,
                    })
                } else {
                    let mut request = engine_request(
                        "resolveMedia",
                        &source.source,
                        None,
                        None,
                        Some(engine_book.clone()),
                        Some(raw_chapter.clone()),
                        next_chapter_url.clone(),
                    );
                    request.media_url = Some(content.trim().to_owned());
                    let resolved = self.executor.execute(request).await?;
                    let (url, headers) = resolved_media_result(&resolved)?;
                    let media_format = if is_hls_media_url(&url) {
                        MEDIA_FORMAT_HLS
                    } else {
                        MEDIA_FORMAT_DIRECT
                    };
                    Some(PreparedChapterMedia::Audio {
                        url,
                        headers,
                        media_format,
                    })
                }
            } else {
                None
            };
            let _sources_lock = self.sources_lock.lock().await;
            let current_source = self
                .read_sources()
                .await?
                .into_iter()
                .find(|candidate| candidate.id == source_id)
                .ok_or_else(|| "Book source was removed while preparing a chapter".to_owned())?;
            if current_source.source != source.source
                || self.source_revision(&source_id).await? != source_revision
            {
                return Err(
                    "Book source changed while preparing a chapter; retry the request".into(),
                );
            }

            let _book_lock = self.book_lock(book_id).await;
            let current_private = self
                .read_private_json(Path::new("books").join(format!("{book_id}.json")))
                .await
                .map_err(|_| "Book was removed while preparing a chapter".to_owned())?;
            if current_private["sourceId"].as_str() != Some(source_id.as_str()) {
                return Err("Book source changed while preparing a chapter".to_owned());
            }
            if current_private.get("book") != Some(&engine_book) {
                return Err("Book metadata changed while preparing a chapter".to_owned());
            }
            let mut current_book = self
                .store
                .read_json_ref(&book_ref)
                .await
                .map_err(|_| "Book was removed while preparing a chapter".to_owned())?;
            let descriptors = current_book
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book chapter directory is missing".to_owned())?;
            let index = descriptors
                .iter()
                .position(|chapter| chapter["id"].as_str() == Some(chapter_id.as_str()))
                .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
            let current_raw_chapters = current_private
                .get("chapters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Book source chapter data is missing".to_owned())?;
            let current_raw = current_raw_chapters
                .get(index)
                .ok_or_else(|| "Book catalog changed while preparing a chapter".to_owned())?;
            if current_raw != &raw_chapter {
                return Err("Book catalog changed while preparing a chapter".to_owned());
            }

            let existing_ref = descriptors[index]
                .get("resource")
                .and_then(|resource| resource.get("resourceId"))
                .and_then(Value::as_str)
                .filter(|src| !src.is_empty())
                .map(|src| scoped_chapter_cache_ref(book_id, src))
                .transpose()?;
            let media_content_format = media_request.as_ref().map(|media| match media {
                PreparedChapterMedia::Audio { .. } => "audio",
                PreparedChapterMedia::Video { .. } => "video",
            });
            let reference = if let Some(media_request) = media_request {
                match media_request {
                    PreparedChapterMedia::Audio {
                        url,
                        headers,
                        media_format,
                    } => {
                        let media_id = chapter_media_id(book_id, &chapter_id, &url, &headers);
                        let media_ref = self
                            .server
                            .register_media_with_id(&media_id, &url, &headers)
                            .await
                            .map_err(|error| error.to_string())?;
                        self.store
                            .write_chapter_media(
                                book_id,
                                &chapter_id,
                                &media_ref,
                                "audio",
                                media_format,
                                &[],
                            )
                            .await
                            .map_err(|error| error.to_string())?
                    }
                    PreparedChapterMedia::Video {
                        options,
                        selected_index,
                    } => {
                        let selected_media_format = options
                            .get(selected_index)
                            .and_then(|option| option.media_format)
                            .ok_or_else(|| {
                                "Video source selected a resolution without a media format"
                                    .to_owned()
                            })?;
                        let mut video_sources = Vec::with_capacity(options.len());
                        let mut selected_media_ref = None;
                        for (option_index, option) in options.into_iter().enumerate() {
                            if let Some(url) = option.url {
                                let media_id = chapter_media_variant_id(
                                    book_id,
                                    &chapter_id,
                                    option.source_index,
                                    &url,
                                    &option.headers,
                                );
                                let media_ref = self
                                    .server
                                    .register_media_with_id(&media_id, &url, &option.headers)
                                    .await
                                    .map_err(|error| error.to_string())?;
                                let selected = option_index == selected_index;
                                if selected {
                                    selected_media_ref = Some(media_ref.clone());
                                }
                                video_sources.push(ChapterVideoSource {
                                    label: option.label,
                                    media_ref: Some(media_ref),
                                    media_format: option.media_format,
                                    unavailable_reason: None,
                                    selected,
                                });
                            } else {
                                video_sources.push(ChapterVideoSource {
                                    label: option.label,
                                    media_ref: None,
                                    media_format: None,
                                    unavailable_reason: Some(
                                        option
                                            .unavailable_reason
                                            .unwrap_or(VIDEO_RESOLUTION_UNAVAILABLE),
                                    ),
                                    selected: false,
                                });
                            }
                        }
                        let selected_media_ref = selected_media_ref.ok_or_else(|| {
                            "Video source selected no playable media resolution".to_owned()
                        })?;
                        self.store
                            .write_chapter_media(
                                book_id,
                                &chapter_id,
                                &selected_media_ref,
                                "video",
                                selected_media_format,
                                &video_sources,
                            )
                            .await
                            .map_err(|error| error.to_string())?
                    }
                }
            } else if let Some(reference) = existing_ref {
                if self.cached_chapter_format(&reference).await?
                    == Some(non_media_content_format)
                {
                    reference
                } else if looks_like_html(content) {
                    self.store
                        .write_chapter_rich_text(
                            book_id,
                            &chapter_id,
                            content,
                            content_base_url.as_deref(),
                        )
                        .await
                        .map_err(|error| error.to_string())?
                } else {
                    self.store
                        .write_chapter_text(book_id, &chapter_id, content)
                        .await
                        .map_err(|error| error.to_string())?
                }
            } else if looks_like_html(content) {
                self.store
                    .write_chapter_rich_text(
                            book_id,
                            &chapter_id,
                            content,
                            content_base_url.as_deref(),
                        )
                    .await
                    .map_err(|error| error.to_string())?
            } else {
                self.store
                    .write_chapter_text(book_id, &chapter_id, content)
                    .await
                    .map_err(|error| error.to_string())?
            };
            if let Some(chapter) = current_book
                .get_mut("chapters")
                .and_then(Value::as_array_mut)
                .and_then(|chapters| chapters.get_mut(index))
            {
                let content_format =
                    media_content_format.unwrap_or(non_media_content_format);
                chapter["resource"] = serde_json::to_value(
                    crate::models::ChapterResourceDescriptor::for_chapter(
                        &reference,
                        content_format,
                    ),
                )
                .map_err(|error| format!("Cannot encode chapter resource descriptor: {error}"))?;
            }
            self.store
                .write_json_ref(&book_ref, &current_book)
                .await
                .map_err(|error| error.to_string())?;
            prepared += 1;
        }
        Ok(
            json!({ "book": self.resource_descriptor(&book_ref), "prepared": prepared, "bookId": book_id, "fromIndex": from_index }),
        )
    }
}

pub(super) fn chapter_id(book_id: &str, stable_url: &str) -> String {
    format!(
        "chapter-{:016x}",
        stable_hash(&format!("{book_id}\0{stable_url}"))
    )
}

/// Opaque media identities include the resolved request, not only chapter ID.
/// A changed upstream URL or private headers must never retarget an earlier
/// media descriptor to a different request.
pub(super) fn chapter_media_id(
    book_id: &str,
    chapter_id: &str,
    upstream_url: &str,
    headers: &[(String, String)],
) -> String {
    let fingerprint = serde_json::to_vec(&(book_id, chapter_id, upstream_url, headers))
        .expect("media request identity is serializable");
    format!("chapter-media-{}", sha256_hex(&fingerprint))
}

pub(super) fn chapter_media_variant_id(
    book_id: &str,
    chapter_id: &str,
    source_index: usize,
    upstream_url: &str,
    headers: &[(String, String)],
) -> String {
    let base = chapter_media_id(book_id, chapter_id, upstream_url, headers);
    if source_index == 0 {
        base
    } else {
        format!("{base}-variant-{source_index}")
    }
}

/// Reduce legacy VideoSource JSON or `name::url` chapter content to a bounded
/// list of named candidates. Every candidate is resolved by KMP before it can
/// be registered or exposed to the chapter HTML.
fn video_media_candidates(content: &str) -> Result<(Vec<VideoMediaCandidate>, usize), String> {
    let content = content.trim();
    if content.starts_with("#BASE:") || content.starts_with("#EXTM3U") {
        return Err("Memory HLS video sources are not supported by the current reader".into());
    }

    if content.starts_with('{') {
        let source = serde_json::from_str::<Value>(content)
            .map_err(|_| "Video source JSON is invalid".to_owned())?;
        let resolutions = source
            .get("resolutions")
            .and_then(Value::as_array)
            .ok_or_else(|| "Video source has no resolution list".to_owned())?;
        if resolutions.len() > MAX_VIDEO_MEDIA_VARIANTS {
            return Err(format!(
                "Video source exceeds the supported limit of {MAX_VIDEO_MEDIA_VARIANTS} resolutions"
            ));
        }
        let index = match source.get("defaultIndex") {
            None => 0,
            Some(value) => value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    "Video source defaultIndex must be a nonnegative integer".to_owned()
                })?,
        };
        if resolutions.get(index).is_none() {
            return Err(
                "Video source default resolution is outside its resolution list".to_owned(),
            );
        }
        let headers = match source.get("headers") {
            None | Some(Value::Null) => None,
            Some(Value::Object(headers)) => Some(
                headers
                    .iter()
                    .map(|(name, value)| {
                        value
                            .as_str()
                            .map(|value| (name.clone(), value.to_owned()))
                            .ok_or_else(|| "Video source headers must be strings".to_owned())
                    })
                    .collect::<Result<HashMap<_, _>, _>>()?,
            ),
            Some(_) => return Err("Video source headers must be an object".into()),
        };
        let candidates = resolutions
            .iter()
            .enumerate()
            .map(|(source_index, resolution)| {
                let label = resolution
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| bounded_video_label(name, source_index))
                    .unwrap_or_else(|| format!("清晰度 {}", source_index + 1));
                let url = resolution
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(str::to_owned);
                VideoMediaCandidate {
                    source_index,
                    label,
                    url,
                    headers: headers.clone(),
                }
            })
            .collect();
        return Ok((candidates, index));
    }

    if !has_http_media_scheme(content) && content.contains("::") {
        let mut candidates = Vec::new();
        for line in content.lines() {
            if let Some((name, url)) = line.split_once("::") {
                let source_index = candidates.len();
                let label = name.trim().chars().take(80).collect::<String>();
                candidates.push(VideoMediaCandidate {
                    source_index,
                    label: if label.is_empty() {
                        format!("清晰度 {}", source_index + 1)
                    } else {
                        label
                    },
                    url: (!url.trim().is_empty()).then(|| url.trim().to_owned()),
                    headers: None,
                });
            }
        }
        if candidates.is_empty() {
            return Err("Video source has no direct URL in its resolution list".into());
        }
        if candidates.len() > MAX_VIDEO_MEDIA_VARIANTS {
            return Err(format!(
                "Video source exceeds the supported limit of {MAX_VIDEO_MEDIA_VARIANTS} resolutions"
            ));
        }
        return Ok((candidates, 0));
    }

    if !has_http_media_scheme(content) {
        return Err("Video source must provide a direct HTTP(S) media URL".into());
    }
    Ok((
        vec![VideoMediaCandidate {
            source_index: 0,
            label: "默认".to_owned(),
            url: Some(content.to_owned()),
            headers: None,
        }],
        0,
    ))
}

pub(super) fn bounded_video_label(label: &str, source_index: usize) -> String {
    let label = label.chars().take(80).collect::<String>();
    if label.trim().is_empty() {
        format!("清晰度 {}", source_index + 1)
    } else {
        label
    }
}

pub(super) fn has_http_media_scheme(value: &str) -> bool {
    value.split_once("://").is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

pub(super) fn resolved_media_result(
    resolved: &Value,
) -> Result<(String, Vec<(String, String)>), String> {
    let media_url = resolved
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "Source engine returned no resolved media URL".to_owned())?
        .to_owned();
    let headers = resolved
        .get("headers")
        .and_then(Value::as_object)
        .ok_or_else(|| "Source engine returned invalid media headers".to_owned())?
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), value.to_owned()))
                .ok_or_else(|| "Source engine returned an invalid media header".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((media_url, headers))
}

pub(super) fn is_direct_http_media_url(value: &str) -> bool {
    reqwest::Url::parse(value).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

pub(super) fn is_hls_media_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .ok()
        .is_some_and(|url| url.path().to_ascii_lowercase().ends_with(".m3u8"))
}
