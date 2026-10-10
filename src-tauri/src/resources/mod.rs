//! 资源引用及其模块入口。
//! 持久化稳定的 resource:// 标识，仅在响应时生成运行时 URL。

use std::fmt;

use serde::Serialize;

mod json;
mod media_proxy;
mod reader_html;
mod server;
mod store;
mod validation;

pub(crate) use json::validate_persistable_json_bytes;
pub use server::ResourceServer;
use server::byte_response;
pub use store::ResourceStore;
pub(crate) use store::ResourceStoreWriterGuard;

use validation::validate_public_path;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceRef(String);

/// A browser-safe video choice embedded in a processed chapter. Upstream URLs
/// and request headers stay in the private media registry; unavailable choices
/// carry only a fixed, display-safe reason.
pub(crate) struct ChapterVideoSource {
    pub label: String,
    pub media_ref: Option<ResourceRef>,
    pub media_format: Option<&'static str>,
    pub unavailable_reason: Option<&'static str>,
    pub selected: bool,
}

impl ResourceRef {
    /// Construct a public stable reference (`resource://...`) or a direct
    /// external HTTP(S) resource URL. Other URL schemes are never accepted.
    pub fn new(reference: impl AsRef<str>) -> Result<Self, ResourceError> {
        let reference = reference.as_ref();
        if let Some(path) = reference.strip_prefix("resource://") {
            validate_public_path(path)?;
            return Ok(Self(format!("resource://{path}")));
        }
        let url = reqwest::Url::parse(reference)
            .map_err(|_| ResourceError::new("Resource URL must be a valid HTTP(S) URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ResourceError::new(
                "Only credential-free HTTP(S) resource URLs are allowed",
            ));
        }
        Ok(Self(url.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn path(&self) -> &str {
        // External references have no local filesystem path.
        self.0.strip_prefix("resource://").unwrap_or("")
    }

    pub fn is_local(&self) -> bool {
        self.0.starts_with("resource://")
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ResourceRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ResourceRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug)]
pub struct ResourceError {
    message: String,
}

impl ResourceError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ResourceError {}

impl From<std::io::Error> for ResourceError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl ResourceRef {
    pub(super) fn from_validated_path(path: &str) -> Self {
        Self(format!("resource://{path}"))
    }
}
