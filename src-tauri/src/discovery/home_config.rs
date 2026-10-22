//! 首页分区设置及来源类别检查。

use serde::{Deserialize, Serialize};

use crate::{
    database::Database,
    discovery::{self, PrivateCategory},
    rss::is_rss_source,
};

const MAX_TABS: usize = 32;
const MAX_SECTIONS_PER_TAB: usize = 64;
const MAX_TITLE_CHARS: usize = 128;

/// Validate sources and categories when the complete settings document changes home layout.
pub(crate) async fn validate_home_sources(
    database: &Database,
    document: &HomeConfigDocument,
) -> Result<(), String> {
    let sources = crate::source_catalog::SourceCatalog::new(database.clone());
    for tab in &document.tabs {
        for section in &tab.sections {
            let source = sources.get(&section.source_id).await?;
            if !source.enabled {
                return Err("A home section source is disabled".to_owned());
            }
            if is_rss_source(&source.source) {
                return Err("RSS subscriptions cannot be used as discovery sections".to_owned());
            }
            let category: PrivateCategory = database
                .discovery_category(&section.source_id, &section.category_id)
                .map_err(|error| error.to_string())?
                .ok_or("Unknown or expired discovery category ID")?;
            if category.kind != "engine" || category.url.is_none() {
                return Err("A home section must use a navigable discovery category".to_owned());
            }
        }
    }
    Ok(())
}

/// Validate the stored JSON shape without consulting source or category data.
/// Backup code uses the same bounds and schema contract as the runtime loader.
pub(crate) fn validate_home_document(document: &HomeConfigDocument) -> Result<(), String> {
    validate_structure(document)
}

pub(crate) fn normalize_home_document(document: &mut HomeConfigDocument) {
    normalize_orders(document);
}

fn validate_structure(document: &HomeConfigDocument) -> Result<(), String> {
    if document.tabs.is_empty() || document.tabs.len() > MAX_TABS {
        return Err(format!("Home must have between 1 and {MAX_TABS} tabs"));
    }
    let mut tab_ids = std::collections::HashSet::new();
    let mut tab_titles = std::collections::HashSet::new();
    let mut section_ids = std::collections::HashSet::new();
    for tab in &document.tabs {
        validate_key(&tab.id, "tab ID")?;
        if !tab_ids.insert(tab.id.as_str()) {
            return Err("Home tab IDs must be unique".to_owned());
        }
        validate_title(&tab.title, "tab title")?;
        if !tab_titles.insert(tab.title.trim().to_lowercase()) {
            return Err("Home tab titles must be unique".to_owned());
        }
        if tab.sections.len() > MAX_SECTIONS_PER_TAB {
            return Err(format!(
                "A home tab can have at most {MAX_SECTIONS_PER_TAB} sections"
            ));
        }
        let infinite_grid_count = tab
            .sections
            .iter()
            .filter(|section| section.style == 2)
            .count();
        if infinite_grid_count > 1 {
            return Err("A home tab can have only one infinite grid section".to_owned());
        }
        for section in &tab.sections {
            validate_key(&section.id, "section ID")?;
            if !section_ids.insert(section.id.as_str()) {
                return Err("Home section IDs must be unique".to_owned());
            }
            validate_title(&section.title, "section title")?;
            discovery::validate_source_id(&section.source_id)?;
            discovery::validate_category_id(&section.category_id)?;
            if section.style > 3 {
                return Err("Unsupported home section style".to_owned());
            }
        }
    }
    Ok(())
}

fn validate_key(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("Invalid {name}"));
    }
    Ok(())
}

fn validate_title(value: &str, name: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("Invalid {name}"));
    }
    Ok(())
}

fn normalize_orders(document: &mut HomeConfigDocument) {
    document.tabs.sort_by_key(|tab| tab.sort_order);
    for (tab_index, tab) in document.tabs.iter_mut().enumerate() {
        tab.sort_order = tab_index as u32;
        tab.sections.sort_by_key(|section| section.sort_order);
        if let Some(index) = tab.sections.iter().position(|section| section.style == 2) {
            let section = tab.sections.remove(index);
            tab.sections.push(section);
        }
        for (section_index, section) in tab.sections.iter_mut().enumerate() {
            section.sort_order = section_index as u32;
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HomeConfigDocument {
    pub tabs: Vec<HomeTab>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HomeTab {
    pub id: String,
    pub title: String,
    pub sort_order: u32,
    pub sections: Vec<HomeSection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HomeSection {
    pub id: String,
    pub title: String,
    pub source_id: String,
    pub category_id: String,
    pub style: u8,
    pub sort_order: u32,
    #[serde(default)]
    pub cover_video: bool,
}

impl Default for HomeConfigDocument {
    fn default() -> Self {
        Self {
            tabs: vec![HomeTab {
                id: "tab-home".to_owned(),
                title: "主页".to_owned(),
                sort_order: 0,
                sections: Vec::new(),
            }],
        }
    }
}
