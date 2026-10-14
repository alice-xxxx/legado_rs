//! Explicit downloads materialize resources; opening details never calls this.
use super::*;
use std::collections::VecDeque;

impl ApplicationService {
    pub(super) async fn cache_chapter_files_unlocked(
        &self,
        book_id: &str,
        chapter_id: &str,
        signal: &tokio::sync::watch::Receiver<TaskSignal>,
    ) -> Result<bool, String> {
        let _chapter = self.chapter_lock(book_id, chapter_id).await;
        let book_ref = self.store.book_ref(book_id).map_err(|e| e.to_string())?;
        let before = self
            .store
            .read_json_ref(&book_ref)
            .await
            .map_err(|e| e.to_string())?;
        let chapter = before["chapters"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|chapter| chapter["id"].as_str() == Some(chapter_id))
            .ok_or("Downloaded chapter no longer exists")?
            .clone();
        let source_revision = if let Some(source) = before["sourceId"].as_str() {
            Some(self.source_revision(source).await?)
        } else {
            None
        };
        let kind = before["contentType"].as_str().unwrap_or("text");
        let mut markup = None;
        let mut roots = Vec::new();
        if kind == "image" {
            for page in chapter["pages"].as_array().into_iter().flatten() {
                if let Some(url) = page["url"].as_str() {
                    roots.push(url.to_owned());
                }
            }
        } else if matches!(kind, "audio" | "video") {
            if let Some(url) = chapter["url"].as_str() {
                roots.push(url.to_owned());
            }
            for source in chapter["sources"].as_array().into_iter().flatten() {
                if let Some(url) = source["url"].as_str().or_else(|| source["src"].as_str()) {
                    roots.push(url.to_owned());
                }
            }
        } else if kind == "html" {
            if let Some(raw) = chapter["resource"]["resourceId"].as_str() {
                let reference = ResourceRef::new(raw).map_err(|e| e.to_string())?;
                let document = self
                    .store
                    .read_json_ref(&reference)
                    .await
                    .map_err(|e| e.to_string())?;
                let body = document["markup"].as_str().unwrap_or_default().to_owned();
                let regex = regex::Regex::new(r#"resource://media/[^\s"'<>]+"#)
                    .map_err(|e| e.to_string())?;
                roots.extend(
                    regex
                        .find_iter(&body)
                        .map(|value| value.as_str().to_owned()),
                );
                markup = Some(body);
            }
        }
        // Native files already on disk need no HTTP round trip.
        roots.retain(|url| url.starts_with("resource://media/"));
        if roots.is_empty() {
            return Ok(true);
        }
        let mut names = HashMap::<String, String>::new();
        let mut queue = VecDeque::new();
        for root in &roots {
            let reference = ResourceRef::new(root).map_err(|e| e.to_string())?;
            let url = self.server.url_for(&reference);
            if !names.contains_key(&url) {
                queue.push_back(url.clone());
                names.insert(url, String::new());
            }
        }
        let client = crate::http_client::raw_builder()
            .build()
            .map_err(|e| e.to_string())?;
        let mut payloads = Vec::new();
        let mut files = Vec::new();
        let uri_attributes = regex::Regex::new(r#"URI="([^"]+)""#).map_err(|e| e.to_string())?;
        while let Some(url) = queue.pop_front() {
            if signal.borrow().cancelled || signal.borrow().paused {
                return Ok(false);
            }
            let mut response = client
                .get(&url)
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?
                .error_for_status()
                .map_err(|e| e.without_url().to_string())?;
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_lowercase();
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                if signal.borrow().cancelled || signal.borrow().paused {
                    return Ok(false);
                }
                if bytes.len().saturating_add(chunk.len()) > 512 * 1024 * 1024 {
                    return Err("A downloaded resource exceeds 512 MiB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            if bytes.is_empty() {
                return Err("Downloaded resource is empty".into());
            }
            let playlist = bytes.starts_with(b"#EXTM3U");
            let extension = if playlist {
                "m3u8"
            } else if content_type.contains("image/png") {
                "png"
            } else if content_type.contains("image/webp") {
                "webp"
            } else if content_type.contains("image/gif") {
                "gif"
            } else if content_type.contains("image/") {
                "jpg"
            } else if content_type.contains("audio/aac") {
                "aac"
            } else if content_type.contains("audio/mp4") {
                "m4a"
            } else if content_type.contains("audio/ogg") {
                "ogg"
            } else if content_type.contains("audio/wav") {
                "wav"
            } else if content_type.contains("audio/") {
                "mp3"
            } else if content_type.contains("video/mp4") {
                "mp4"
            } else if content_type.contains("webm") {
                "webm"
            } else if kind == "audio" {
                "mp3"
            } else {
                "ts"
            };
            let stable_id = url
                .strip_prefix(self.server.base_url())
                .ok_or("Download escaped the resource server")?;
            let name = format!(
                "{}-{}.{}",
                chapter_id,
                sha256_hex(stable_id.as_bytes()),
                extension
            );
            names.insert(url.clone(), name.clone());
            let mut dependencies = Vec::new();
            if playlist {
                let body = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
                if !body.contains("#EXT-X-ENDLIST") && !body.contains("#EXT-X-STREAM-INF") {
                    return Err(
                        "Live playlists cannot be saved as a complete offline chapter".into(),
                    );
                }
                for line in body.lines() {
                    let line = line.trim();
                    let refs = if line.starts_with('#') {
                        uri_attributes
                            .captures_iter(line)
                            .map(|c| c[1].to_owned())
                            .collect::<Vec<_>>()
                    } else if line.is_empty() {
                        Vec::new()
                    } else {
                        vec![line.to_owned()]
                    };
                    for raw in refs {
                        let child = reqwest::Url::parse(&url)
                            .and_then(|base| base.join(&raw))
                            .map_err(|e| e.to_string())?
                            .to_string();
                        if !child.starts_with(self.server.base_url()) {
                            return Err("Playlist dependency escaped the resource server".into());
                        }
                        if !names.contains_key(&child) {
                            if names.len() >= 8192 {
                                return Err("Offline playlist has too many dependencies".into());
                            }
                            names.insert(child.clone(), String::new());
                            queue.push_back(child.clone());
                        }
                        dependencies.push((raw, child));
                    }
                }
            }
            if playlist {
                payloads.push((url, name, bytes, dependencies));
            } else {
                self.store
                    .write_asset(book_id, &name, &bytes)
                    .await
                    .map_err(|e| e.to_string())?;
                files.push(name);
            }
        }
        for (_, name, bytes, dependencies) in payloads {
            let bytes = if dependencies.is_empty() {
                bytes
            } else {
                let mut body = String::from_utf8(bytes).map_err(|e| e.to_string())?;
                // Long URLs first avoids one dependency replacing a prefix of another.
                let mut dependencies = dependencies;
                dependencies.sort_by_key(|(raw, _)| std::cmp::Reverse(raw.len()));
                for (raw, child) in dependencies {
                    body =
                        body.replace(&raw, names.get(&child).ok_or("Missing offline dependency")?);
                }
                body.into_bytes()
            };
            self.store
                .write_asset(book_id, &name, &bytes)
                .await
                .map_err(|e| e.to_string())?;
            files.push(name);
        }
        let replacements = roots
            .iter()
            .map(|root| {
                let reference = ResourceRef::new(root).map_err(|e| e.to_string())?;
                let url = self.server.url_for(&reference);
                let name = names.get(&url).ok_or("Missing downloaded resource")?;
                Ok((
                    root.clone(),
                    self.store
                        .asset_ref(book_id, name)
                        .map_err(|e| e.to_string())?
                        .to_string(),
                ))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        let mut next_chapter = chapter.clone();
        rewrite_downloaded_refs(&mut next_chapter, &replacements);
        if let Some(mut body) = markup {
            for (from, to) in &replacements {
                body = body.replace(from, to);
            }
            let (reference, document) = self
                .store
                .chapter_rich_text_document(book_id, chapter_id, &body)
                .map_err(|e| e.to_string())?;
            self.store
                .write_json_ref(&reference, &document)
                .await
                .map_err(|e| e.to_string())?;
            next_chapter["resource"] = serde_json::to_value(
                crate::models::ChapterResourceDescriptor::for_chapter(&reference, "richText"),
            )
            .map_err(|e| e.to_string())?;
        }
        let roots = replacements
            .values()
            .filter_map(|url| url.rsplit('/').next())
            .collect::<HashSet<_>>();
        files.retain(|file| !roots.contains(file.as_str()));
        if !files.is_empty() {
            next_chapter["files"] = json!(files);
        }
        let _sources = self.sources_lock.lock().await;
        if let Some(source) = before["sourceId"].as_str() {
            if Some(self.source_revision(source).await?) != source_revision {
                return Err("Source changed during download".into());
            }
        }
        let _book = self.book_lock(book_id).await;
        if signal.borrow().cancelled || signal.borrow().paused {
            return Ok(false);
        }
        self.store
            .update_json_ref(&book_ref, move |mut current| {
                if current["bookInstanceId"] != before["bookInstanceId"]
                    || current["catalogGeneration"] != before["catalogGeneration"]
                {
                    return Err("Catalog changed during download".into());
                }
                let chapters = current["chapters"]
                    .as_array_mut()
                    .ok_or("Content has no chapters")?;
                let target = chapters
                    .iter_mut()
                    .find(|c| c["id"].as_str() == Some(chapter_id))
                    .ok_or("Chapter was removed")?;
                if *target != chapter {
                    return Err("Chapter changed during download".into());
                }
                *target = next_chapter;
                Ok(current)
            })
            .await
            .map(|_| true)
            .map_err(|e| e.to_string())
    }
}

fn rewrite_downloaded_refs(value: &mut Value, replacements: &HashMap<String, String>) {
    match value {
        Value::String(url) => {
            if let Some(next) = replacements.get(url) {
                *url = next.clone();
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite_downloaded_refs(value, replacements);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if !matches!(key.as_str(), "engineurl" | "engineData" | "engineTitle") {
                    rewrite_downloaded_refs(value, replacements);
                }
            }
        }
        _ => {}
    }
}
