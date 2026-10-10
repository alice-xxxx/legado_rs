//! 维护搜索结果加入书架时使用的书籍目录身份。
//! Pure reconciliation for refreshed chapter catalogs.
//!
//! The source engine owns raw chapter objects. This module reads only the
//! chapter URL and title fields needed to reconcile identity, then returns
//! frontend-safe descriptors. Callers own source execution, operation gates,
//! per-book locks, persistence, and cache-file cleanup.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::models::{ChapterDescriptor, ProgressSummary};

#[derive(Clone, Debug)]
pub struct CatalogRefreshPlan {
    /// Processed chapter metadata only. Raw engine objects never appear here.
    pub chapters: Vec<ChapterDescriptor>,
    pub latest_chapter: Option<String>,
    pub progress: ProgressSummary,
    pub added_count: usize,
    pub matched_count: usize,
    pub progress_relocated: bool,
}

#[derive(Clone, Debug)]
struct ChapterIdentity {
    url: Option<String>,
    normalized_url: Option<String>,
    normalized_title: String,
}

/// Reconcile a newly parsed source catalog against its previous processed
/// descriptors and private raw chapter list.
///
/// URL equality preserves all query and fragment components. Unique raw-title
/// matching is a fallback for URL changes only within this same book/source
/// refresh; it is disabled when two URLs point at the same route but differ in
/// query or fragment, because those parts can distinguish separate chapters.
/// Refresh is rejected when either catalog has duplicate normalized URLs or
/// persisted chapter IDs are duplicated; the caller must retain old resources
/// rather than guess which cached chapter belongs to an ambiguous entry.
/// The caller supplies IDs of verified typed chapter resources, so this
/// function stays independent of filesystem access.
pub fn reconcile_catalog(
    book_id: &str,
    old_raw_chapters: &[Value],
    old_chapters: &[ChapterDescriptor],
    new_raw_chapters: &[Value],
    current_progress: &ProgressSummary,
    cached_chapter_ids: &HashSet<String>,
    refreshed_at_ms: u64,
) -> Result<CatalogRefreshPlan, String> {
    validate_id(book_id, "bookId")?;
    validate_unique_urls(old_raw_chapters, "Existing")?;
    validate_unique_urls(new_raw_chapters, "Refreshed")?;
    // Protect the last known catalog even if its public descriptors are
    // missing or out of sync with the private engine data. A transient empty
    // engine result must never erase either representation.
    if (!old_raw_chapters.is_empty() || !old_chapters.is_empty()) && new_raw_chapters.is_empty() {
        return Err("Refreshed catalog is empty; keeping the existing catalog".to_owned());
    }
    let mut old_ids = HashSet::with_capacity(old_chapters.len());
    for chapter in old_chapters {
        validate_id(&chapter.id, "chapterId")?;
        if !old_ids.insert(chapter.id.as_str()) {
            return Err("Existing catalog contains duplicate chapter IDs".to_owned());
        }
    }

    let old_identities = old_raw_chapters
        .iter()
        .take(old_chapters.len())
        .enumerate()
        .map(|(index, raw)| {
            identity(
                raw,
                old_chapters
                    .get(index)
                    .map(|chapter| chapter.title.as_str()),
                index,
            )
        })
        .collect::<Vec<_>>();
    let new_identities = new_raw_chapters
        .iter()
        .enumerate()
        .map(|(index, raw)| identity(raw, None, index))
        .collect::<Vec<_>>();

    let old_title_counts = count_titles(&old_identities);
    let new_title_counts = count_titles(&new_identities);
    let mut old_to_new = vec![None; old_chapters.len()];
    let mut new_to_old = vec![None; new_raw_chapters.len()];
    let mut old_used = vec![false; old_chapters.len()];

    for (new_index, new_identity) in new_identities.iter().enumerate() {
        let exact_url_match = find_old_match(
            new_index,
            new_identity,
            &old_identities,
            &old_used,
            UrlMatch::Exact,
        );
        let normalized_url_match = exact_url_match.or_else(|| {
            find_old_match(
                new_index,
                new_identity,
                &old_identities,
                &old_used,
                UrlMatch::Normalized,
            )
        });
        let match_index = normalized_url_match.or_else(|| {
            find_unique_title_match(
                new_identity,
                &old_identities,
                &old_used,
                &old_title_counts,
                &new_title_counts,
            )
        });

        if let Some(old_index) = match_index {
            old_used[old_index] = true;
            old_to_new[old_index] = Some(new_index);
            new_to_old[new_index] = Some(old_index);
        }
    }

    let mut chapters = Vec::with_capacity(new_raw_chapters.len());
    let mut kept_ids = HashSet::new();
    for (index, raw) in new_raw_chapters.iter().enumerate() {
        let url = text_at(raw, &["url", "chapterUrl"]).unwrap_or_else(|| format!("@{index}"));
        let title = text_at(raw, &["title", "chapterName", "name"])
            .or_else(|| {
                old_chapters
                    .get(new_to_old[index]?)
                    .map(|old| old.title.clone())
            })
            .unwrap_or_else(|| format!("Chapter {}", index + 1));
        let matched = new_to_old[index].and_then(|old_index| old_chapters.get(old_index));
        let id = matched
            .map(|old| old.id.clone())
            .unwrap_or_else(|| chapter_id(book_id, &url));
        validate_id(&id, "chapterId")?;
        if !kept_ids.insert(id.clone()) {
            return Err("Refreshed catalog produces duplicate chapter IDs".to_owned());
        }

        // Title matching may preserve a chapter's identity and reading progress
        // across URL changes, but a different content URL cannot reuse its body.
        // Compare normalized complete URLs (including query and fragment).
        let unchanged_url = new_to_old[index]
            .and_then(|old_index| old_identities.get(old_index))
            .and_then(|old| old.normalized_url.as_deref())
            .is_some_and(|old_url| {
                new_identities[index].normalized_url.as_deref() == Some(old_url)
            });
        let resource = matched
            .filter(|old| unchanged_url && cached_chapter_ids.contains(&old.id))
            .and_then(|old| old.resource.clone());
        chapters.push(ChapterDescriptor {
            id,
            title,
            index,
            resource,
        });
    }

    let matched_count = new_to_old.iter().filter(|old| old.is_some()).count();
    let added_count = chapters.len().saturating_sub(matched_count);
    let latest_chapter = chapters.last().map(|chapter| chapter.title.clone());
    let progress = reconcile_progress(
        current_progress,
        old_chapters,
        &chapters,
        &old_to_new,
        refreshed_at_ms,
    );
    let progress_relocated = progress.chapter_index != current_progress.chapter_index
        || progress.chapter_id != current_progress.chapter_id;

    Ok(CatalogRefreshPlan {
        chapters,
        latest_chapter,
        progress,
        added_count,
        matched_count,
        progress_relocated,
    })
}

fn reconcile_progress(
    progress: &ProgressSummary,
    old_chapters: &[ChapterDescriptor],
    new_chapters: &[ChapterDescriptor],
    old_to_new: &[Option<usize>],
    refreshed_at_ms: u64,
) -> ProgressSummary {
    if new_chapters.is_empty() {
        return ProgressSummary {
            chapter_id: None,
            chapter_index: 0,
            offset: 0,
            updated_at_ms: refreshed_at_ms,
        };
    }

    let old_anchor = progress
        .chapter_id
        .as_deref()
        .and_then(|chapter_id| {
            old_chapters
                .iter()
                .enumerate()
                .filter(|(_, chapter)| chapter.id == chapter_id)
                .min_by_key(|(index, _)| index.abs_diff(progress.chapter_index))
                .map(|(index, _)| index)
        })
        .or_else(|| {
            (progress.chapter_index < old_chapters.len()).then_some(progress.chapter_index)
        });

    // A progress ID can already match a new descriptor even if an older
    // private catalog has been partially repaired or was unavailable.
    let direct_match = progress.chapter_id.as_deref().and_then(|chapter_id| {
        new_chapters
            .iter()
            .find(|chapter| chapter.id == chapter_id)
            .map(|chapter| chapter.index)
    });
    let preserved_match = old_anchor.and_then(|old_index| {
        old_to_new
            .get(old_index)
            .copied()
            .flatten()
            .filter(|new_index| *new_index < new_chapters.len())
    });

    if let Some(new_index) = direct_match.or(preserved_match) {
        let chapter = &new_chapters[new_index];
        return ProgressSummary {
            chapter_id: Some(chapter.id.clone()),
            chapter_index: new_index,
            offset: progress.offset,
            updated_at_ms: progress.updated_at_ms,
        };
    }

    // If the reading chapter disappeared, keep the nearest surviving old
    // chapter; ties prefer the previous chapter to avoid skipping ahead.
    let nearest_survivor = old_anchor.and_then(|anchor| {
        old_to_new
            .iter()
            .enumerate()
            .filter_map(|(old_index, new_index)| new_index.map(|new_index| (old_index, new_index)))
            .min_by_key(|(old_index, _)| {
                (old_index.abs_diff(anchor), usize::from(*old_index > anchor))
            })
            .map(|(_, new_index)| new_index)
    });
    let new_index = nearest_survivor.unwrap_or_else(|| {
        progress
            .chapter_index
            .min(new_chapters.len().saturating_sub(1))
    });
    let chapter = &new_chapters[new_index];
    ProgressSummary {
        chapter_id: Some(chapter.id.clone()),
        chapter_index: new_index,
        offset: 0,
        updated_at_ms: refreshed_at_ms,
    }
}

fn identity(raw: &Value, fallback_title: Option<&str>, index: usize) -> ChapterIdentity {
    let url = text_at(raw, &["url", "chapterUrl"]);
    let title = text_at(raw, &["title", "chapterName", "name"])
        .or_else(|| fallback_title.map(str::to_owned))
        .unwrap_or_else(|| format!("Chapter {}", index + 1));
    ChapterIdentity {
        normalized_url: url.as_deref().map(normalize_url),
        url,
        normalized_title: normalize_title(&title),
    }
}

fn count_titles(identities: &[ChapterIdentity]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for identity in identities {
        *counts.entry(identity.normalized_title.clone()).or_insert(0) += 1;
    }
    counts
}

fn validate_unique_urls(chapters: &[Value], catalog_name: &str) -> Result<(), String> {
    let mut urls = HashSet::with_capacity(chapters.len());
    for chapter in chapters {
        if let Some(url) = text_at(chapter, &["url", "chapterUrl"]) {
            // Use the same identity normalization as matching, and keep query
            // and fragment components intact. Equivalent URL spellings must
            // not create two entries that can map to one stable chapter ID.
            if !urls.insert(normalize_url(&url)) {
                return Err(format!(
                    "{catalog_name} catalog contains duplicate chapter URLs"
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum UrlMatch {
    Exact,
    Normalized,
}

fn find_old_match(
    new_index: usize,
    new: &ChapterIdentity,
    old: &[ChapterIdentity],
    used: &[bool],
    kind: UrlMatch,
) -> Option<usize> {
    let target = match kind {
        UrlMatch::Exact => new.url.as_ref()?,
        UrlMatch::Normalized => new.normalized_url.as_ref()?,
    };
    old.iter()
        .enumerate()
        .filter(|(index, identity)| {
            !used.get(*index).copied().unwrap_or(true)
                && match kind {
                    UrlMatch::Exact => identity.url.as_ref() == Some(target),
                    UrlMatch::Normalized => identity.normalized_url.as_ref() == Some(target),
                }
        })
        .min_by_key(|(old_index, _)| old_index.abs_diff(new_index))
        .map(|(index, _)| index)
}

fn find_unique_title_match(
    new: &ChapterIdentity,
    old: &[ChapterIdentity],
    used: &[bool],
    old_title_counts: &HashMap<String, usize>,
    new_title_counts: &HashMap<String, usize>,
) -> Option<usize> {
    if old_title_counts.get(&new.normalized_title) != Some(&1)
        || new_title_counts.get(&new.normalized_title) != Some(&1)
    {
        return None;
    }
    old.iter()
        .enumerate()
        .find(|(old_index, identity)| {
            !used.get(*old_index).copied().unwrap_or(true)
                && identity.normalized_title == new.normalized_title
                && !same_route_but_different_suffix(identity.url.as_deref(), new.url.as_deref())
        })
        .map(|(index, _)| index)
}

fn normalize_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalize_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|parsed| parsed.to_string())
        .unwrap_or_else(|_| url.trim().to_owned())
}

fn same_route_but_different_suffix(old: Option<&str>, new: Option<&str>) -> bool {
    let (Some(old), Some(new)) = (old, new) else {
        return false;
    };
    let (Ok(old), Ok(new)) = (reqwest::Url::parse(old), reqwest::Url::parse(new)) else {
        let old_route = old.split(['?', '#']).next().unwrap_or(old);
        let new_route = new.split(['?', '#']).next().unwrap_or(new);
        return old_route == new_route && old != new;
    };
    old.scheme() == new.scheme()
        && old.username() == new.username()
        && old.password() == new.password()
        && old.host_str() == new.host_str()
        && old.port() == new.port()
        && old.path() == new.path()
        && (old.query() != new.query() || old.fragment() != new.fragment())
}

fn text_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    })
}

fn chapter_id(book_id: &str, stable_url: &str) -> String {
    let value = format!("{book_id}\0{stable_url}");
    let hash = value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("chapter-{hash:016x}")
}

fn validate_id(id: &str, field: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("Invalid {field}"));
    }
    Ok(())
}
