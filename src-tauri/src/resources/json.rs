//! 物化 JSON 资源引用，并校验可持久化的引用。

use std::net::IpAddr;

use serde_json::Value;

use super::validation::percent_encode_path;
use super::{ResourceError, ResourceRef};

pub(super) fn materialize_html_resource_refs(
    html: &str,
    base_url: &str,
) -> Result<String, ResourceError> {
    let mut materialized = html.to_owned();
    for attribute in ["src", "href", "poster"] {
        for quote in ['"', '\''] {
            materialized = materialize_quoted_html_attribute(&materialized, attribute, quote, base_url)?;
        }
    }
    Ok(materialized)
}

fn materialize_quoted_html_attribute(
    html: &str,
    attribute: &str,
    quote: char,
    base_url: &str,
) -> Result<String, ResourceError> {
    let needle = format!("{attribute}={quote}resource://");
    let mut output = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(offset) = remaining.find(&needle) {
        let value_start = offset + attribute.len() + 2;
        output.push_str(&remaining[..value_start]);
        let value = &remaining[value_start..];
        let Some(value_end) = value.find(quote) else {
            return Err(ResourceError::new("Stored HTML contains an unterminated resource URL"));
        };
        let reference = ResourceRef::new(&value[..value_end])?;
        if !reference.is_local() {
            return Err(ResourceError::new("Stored HTML resource URL must be local"));
        }
        output.push_str(base_url);
        output.push_str(&percent_encode_path(reference.path()));
        remaining = &value[value_end..];
    }

    output.push_str(remaining);
    Ok(output)
}

pub(super) fn materialize_json(value: &mut Value, base_url: &str) -> Result<(), ResourceError> {
    materialize_json_at(value, base_url, None)
}

fn materialize_json_at(
    value: &mut Value,
    base_url: &str,
    property: Option<&str>,
) -> Result<(), ResourceError> {
    match value {
        Value::String(string) if property == Some("markup") => {
            *string = materialize_html_resource_refs(string, base_url)?;
            Ok(())
        }
        Value::String(string)
            if string.starts_with("resource://")
                && property.is_some_and(is_resource_url_property) =>
        {
            let reference = ResourceRef::new(string.as_str())?;
            *string = format!("{base_url}{}", percent_encode_path(reference.path()));
            Ok(())
        }
        Value::Array(values) => {
            for value in values {
                materialize_json_at(value, base_url, property)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values.iter_mut() {
                materialize_json_at(value, base_url, Some(key))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_resource_url_property(property: &str) -> bool {
    property == "src" || property.ends_with("Src")
}

/// Validate an encoded public document with the same persistent-reference
/// rules used by `write_json_ref` and `update_json_ref`.
pub(crate) fn validate_persistable_json_bytes(bytes: &[u8]) -> Result<(), ResourceError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        ResourceError::new(format!("Transaction target is not valid JSON: {error}"))
    })?;
    validate_embedded_refs(&value)
}

pub(super) fn validate_embedded_refs(value: &Value) -> Result<(), ResourceError> {
    validate_embedded_refs_with_key(value, None)
}

fn validate_embedded_refs_with_key(
    value: &Value,
    property: Option<&str>,
) -> Result<(), ResourceError> {
    match value {
        Value::String(string)
            if property.is_some_and(is_resource_url_property)
                && is_runtime_capability_url(string) =>
        {
            Err(ResourceError::new(
                "A per-run resource-server URL cannot be persisted; save its stable resource:// ref",
            ))
        }
        Value::String(string)
            if string.starts_with("resource://")
                || (property.is_some_and(is_resource_url_property) && !string.is_empty()) =>
        {
            ResourceRef::new(string).map(|_| ())
        }
        Value::Array(values) => {
            for value in values {
                validate_embedded_refs_with_key(value, property)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                validate_embedded_refs_with_key(value, Some(key))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_runtime_capability_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host
        .parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or_else(|_| host.eq_ignore_ascii_case("localhost"));
    if !loopback {
        return false;
    }
    let Some(mut segments) = url.path_segments() else {
        return false;
    };
    if segments.next() != Some("r") {
        return false;
    }
    let Some(token) = segments.next() else {
        return false;
    };
    token.len() == 32
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && segments.next().is_some_and(|resource| !resource.is_empty())
}
