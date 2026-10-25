//! 数据库写入参数的 JSON 文本解码，不承担源协议解析。
use serde::Deserialize;
pub(super) fn optional<'de, D, T>(input: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Option::<String>::deserialize(input)?
        .map(|text| serde_json::from_str(&text).map_err(serde::de::Error::custom))
        .transpose()
}
pub(super) fn required<'de, D, T>(input: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(&String::deserialize(input)?).map_err(serde::de::Error::custom)
}
