//! 稳定引用与响应中的展示地址转换。
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceRef(String);
impl ResourceRef {
    pub fn new(value: impl AsRef<str>) -> Result<Self, ResourceError> {
        let value = value.as_ref();
        if let Some(path) = value.strip_prefix("resource://") {
            super::validation::validate_public_path(path)?;
            return Ok(Self(value.to_owned()));
        }
        let url = reqwest::Url::parse(value).map_err(|_| ResourceError::new("无效资源地址"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ResourceError::new(
                "资源地址必须为无账号密码的 HTTP(S) 地址",
            ));
        }
        Ok(Self(url.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn path(&self) -> &str {
        self.0.strip_prefix("resource://").unwrap_or("")
    }
    pub fn is_local(&self) -> bool {
        self.0.starts_with("resource://")
    }
}
impl TryFrom<String> for ResourceRef {
    type Error = ResourceError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<ResourceRef> for String {
    fn from(value: ResourceRef) -> Self {
        value.0
    }
}
impl fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug)]
pub struct ResourceError(String);
impl ResourceError {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}
impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ResourceError {}
impl From<std::io::Error> for ResourceError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

pub(super) fn html_urls(html: &str, base: &str) -> Result<String, ResourceError> {
    static LINKS: std::sync::OnceLock<Result<regex::Regex, regex::Error>> = std::sync::OnceLock::new();
    let links = LINKS.get_or_init(|| {
        regex::Regex::new(r#"(?i)\b(src|href|poster)\s*=\s*(["'])(resource://[^"']+)["']"#)
    }).as_ref().map_err(|error| ResourceError::new(error.to_string()))?;
    let mut error = None;
    let result = links.replace_all(
        html,
        |capture: &regex::Captures<'_>| match ResourceRef::new(&capture[3]) {
            Ok(reference) => format!(
                "{}={}{}{}{}",
                &capture[1],
                &capture[2],
                base,
                super::validation::percent_encode_path(reference.path()),
                &capture[2]
            ),
            Err(failure) => {
                error = Some(failure);
                String::new()
            }
        },
    );
    match error {
        Some(error) => Err(error),
        None => Ok(result.into_owned()),
    }
}
