//! 设置文档与应用主题。
use super::{ReaderDefaults, SCHEMA_VERSION};
use crate::{backup::state::WebDavBackupConfig, discovery::home_config::HomeConfigDocument};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDocument {
    #[serde(default)]
    pub theme: AppTheme,
    #[serde(default)]
    pub ui_style: UiStyle,
    #[serde(default)]
    pub webdav_backup: WebDavBackupConfig,
    pub schema_version: u32,
    #[serde(default = "default_source_http_timeout_seconds")]
    pub source_http_timeout_seconds: u64,
    pub reader: ReaderDefaults,
    pub home: HomeConfigDocument,
}

impl Default for SettingsDocument {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            source_http_timeout_seconds: default_source_http_timeout_seconds(),
            theme: AppTheme::System,
            ui_style: UiStyle::default(),
            webdav_backup: WebDavBackupConfig::default(),
            reader: ReaderDefaults::default(),
            home: HomeConfigDocument::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppTheme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UiStyle {
    #[default]
    Material,
    Ios,
}

fn default_source_http_timeout_seconds() -> u64 {
    15
}
