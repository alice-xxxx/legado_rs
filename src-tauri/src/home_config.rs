//! Rust-owned home navigation and section configuration.
//!
//! The browser edits processed IDs and display labels. Source category URLs
//! stay in discovery's private mapping and are resolved only by Rust.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    application::ApplicationService,
    discovery::{self, PrivateCategory},
    models::CURRENT_SCHEMA_VERSION,
    resources::{ResourceRef, ResourceStore},
    rss::is_rss_source,
};

const HOME_DOCUMENT: &str = "home-tabs";
const MAX_TABS: usize = 32;
const MAX_SECTIONS_PER_TAB: usize = 64;
const MAX_TITLE_CHARS: usize = 128;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HomeConfigDocument {
    pub schema_version: u32,
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
    pub source_name: String,
    pub category_id: String,
    pub category_name: String,
    pub style: u8,
    pub sort_order: u32,
    #[serde(default)]
    pub cover_video: bool,
}

/// Ensure the home configuration resource exists and return its stable ref.
pub async fn home_config_resource(store: &ResourceStore) -> Result<ResourceRef, String> {
    let reference = store
        .reading_ref(HOME_DOCUMENT)
        .map_err(|error| error.to_string())?;
    store
        .update_json_ref(&reference, |current| {
            let document = match current {
                Value::Null => HomeConfigDocument::default(),
                value => decode_document(value)?,
            };
            serde_json::to_value(document).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

/// Validate and persist a complete processed home configuration.
///
/// Every section must refer to an enabled, non-RSS source and a category ID
/// already issued by discovery. Source names and category titles are refreshed
/// from Rust-owned records; URLs and rules are rejected by the JSON model.
pub async fn save_home_config(
    service: &ApplicationService,
    input: Value,
) -> Result<ResourceRef, String> {
    let mut document = decode_document(input)?;
    validate_structure(&document)?;

    for tab in &mut document.tabs {
        for section in &mut tab.sections {
            let source = service.source_record(&section.source_id).await?;
            if !source.enabled {
                return Err("A home section source is disabled".to_owned());
            }
            if is_rss_source(&source.source) {
                return Err("RSS subscriptions cannot be used as discovery sections".to_owned());
            }
            let category: PrivateCategory =
                discovery::private_category(service, &section.source_id, &section.category_id)
                    .await?;
            if category.kind != "engine" || category.url.is_none() {
                return Err("A home section must use a navigable discovery category".to_owned());
            }
            section.source_name = bounded_name(&source.name, "来源");
            section.category_name = bounded_name(&category.title, "分类");
        }
    }
    normalize_orders(&mut document);

    let reference = service
        .resource_store()
        .reading_ref(HOME_DOCUMENT)
        .map_err(|error| error.to_string())?;
    let value = serde_json::to_value(document).map_err(|error| error.to_string())?;
    service
        .resource_store()
        .write_json_ref(&reference, &value)
        .await
        .map_err(|error| error.to_string())?;
    Ok(reference)
}

impl Default for HomeConfigDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            tabs: vec![HomeTab {
                id: "tab-home".to_owned(),
                title: "主页".to_owned(),
                sort_order: 0,
                sections: Vec::new(),
            }],
        }
    }
}

fn decode_document(value: Value) -> Result<HomeConfigDocument, String> {
    let mut document: HomeConfigDocument = serde_json::from_value(value)
        .map_err(|error| format!("Invalid home configuration: {error}"))?;
    if document.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported home configuration schema version {}",
            document.schema_version
        ));
    }
    validate_structure(&document)?;
    normalize_orders(&mut document);
    Ok(document)
}

/// Validate the stored JSON shape without consulting source or category data.
/// Backup code uses the same bounds and schema contract as the runtime loader.
pub(crate) fn validate_home_document(value: &Value) -> Result<(), String> {
    decode_document(value.clone()).map(|_| ())
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
            validate_title(&section.source_name, "source name")?;
            validate_title(&section.category_name, "category name")?;
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

fn bounded_name(value: &str, fallback: &str) -> String {
    let bounded = value
        .trim()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect::<String>();
    if bounded.is_empty() {
        fallback.to_owned()
    } else {
        bounded
    }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{json, Value};

    use crate::{
        application::{ApplicationService, EngineFuture, SourceExecutor},
        source_engine::SourceEngineRequest,
    };

    use super::{home_config_resource, save_home_config};

    struct Categories;

    impl SourceExecutor for Categories {
        fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
            Box::pin(async move {
                if request.operation != "exploreKinds" {
                    return Err(format!("unexpected operation {}", request.operation));
                }
                Ok(json!([{
                    "title": "科幻",
                    "type": "text",
                    "url": "https://source.example/explore?kind=scifi",
                    "style": {"cols": 2, "url": "must not project"}
                }]))
            })
        }
    }

    #[tokio::test]
    async fn home_config_validates_private_category_and_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let executor = Arc::new(Categories);
        let service = ApplicationService::open_with_executor(directory.path(), executor.clone())
            .await
            .unwrap();
        let imported = service
            .import_sources(
                &json!([{
                    "bookSourceName": "来源",
                    "bookSourceUrl": "https://source.example",
                    "bookSourceType": 0
                }])
                .to_string(),
            )
            .await
            .unwrap();
        let source_id = imported["sources"][0]["id"].as_str().unwrap().to_owned();
        crate::discovery::list_categories(&service, &source_id, Some(false))
            .await
            .unwrap();
        let category_ref = service.resource_store().discovery_ref(&source_id).unwrap();
        let category_document = service
            .resource_store()
            .read_json_ref(&category_ref)
            .await
            .unwrap();
        let category_id = category_document["categories"][0]["categoryId"]
            .as_str()
            .unwrap()
            .to_owned();

        let home_ref = home_config_resource(service.resource_store())
            .await
            .unwrap();
        let initial = service
            .resource_store()
            .read_json_ref(&home_ref)
            .await
            .unwrap();
        assert_eq!(initial["tabs"][0]["title"], "主页");
        let saved_ref = save_home_config(
            &service,
            json!({
                "schemaVersion": 1,
                "tabs": [{
                    "id": "tab-main",
                    "title": "首页",
                    "sortOrder": 0,
                    "sections": [{
                        "id": "section-featured",
                        "title": "推荐",
                        "sourceId": source_id,
                        "sourceName": "名称由Rust刷新",
                        "categoryId": category_id,
                        "categoryName": "分类名由Rust刷新",
                        "style": 0,
                        "sortOrder": 0,
                        "coverVideo": false
                    }]
                }]
            }),
        )
        .await
        .unwrap();
        let saved = service
            .resource_store()
            .read_json_ref(&saved_ref)
            .await
            .unwrap();
        assert_eq!(saved["tabs"][0]["sections"][0]["sourceName"], "来源");
        assert_eq!(saved["tabs"][0]["sections"][0]["categoryName"], "科幻");
        assert!(saved.to_string().contains("category-"));
        assert!(!saved.to_string().contains("https://source.example/explore"));
        assert!(!saved.to_string().contains("must not project"));

        let restarted = ApplicationService::open_with_executor(directory.path(), executor)
            .await
            .unwrap();
        let after_restart = restarted
            .resource_store()
            .read_json_ref(&saved_ref)
            .await
            .unwrap();
        assert_eq!(after_restart, saved);

        let mut injected_url = saved.clone();
        injected_url["tabs"][0]["sections"][0]["categoryUrl"] =
            Value::String("https://private.example/rule".to_owned());
        assert!(save_home_config(&restarted, injected_url).await.is_err());
        let mut unsupported_version = saved.clone();
        unsupported_version["schemaVersion"] = json!(2);
        assert!(save_home_config(&restarted, unsupported_version)
            .await
            .is_err());
        let _ = tokio::fs::remove_dir_all(directory.path()).await;
    }

    #[test]
    fn default_config_keeps_one_tab_and_infinite_grid_is_last() {
        let mut value = serde_json::json!({
            "schemaVersion": 1,
            "tabs": [{
                "id": "tab-one", "title": "主页", "sortOrder": 5,
                "sections": [
                    {"id":"section-grid","title":"网格","sourceId":"source-a","sourceName":"A","categoryId":"category-a","categoryName":"A","style":2,"sortOrder":0,"coverVideo":false},
                    {"id":"section-row","title":"横排","sourceId":"source-a","sourceName":"A","categoryId":"category-a","categoryName":"A","style":0,"sortOrder":1,"coverVideo":false}
                ]
            }]
        });
        let mut decoded: super::HomeConfigDocument = serde_json::from_value(value.clone()).unwrap();
        super::normalize_orders(&mut decoded);
        assert_eq!(decoded.tabs[0].sections[1].id, "section-grid");
        value["tabs"][0]["sections"][0]["sourceUrl"] = json!("https://private.example");
        assert!(serde_json::from_value::<super::HomeConfigDocument>(value).is_err());
    }
}
