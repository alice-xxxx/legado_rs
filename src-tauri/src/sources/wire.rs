//! Persisted JSON column decoding; source document types are owned by the engine.
use serde::Deserialize;
pub(crate) use tauri_plugin_source_engine::TextOr;
pub(crate) fn decode<'de, D, T>(input: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Option::<String>::deserialize(input)?
        .map(|text| serde_json::from_str(&text).map_err(serde::de::Error::custom))
        .transpose()
}
pub(crate) fn required<'de, D, T>(input: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(&String::deserialize(input)?).map_err(serde::de::Error::custom)
}
