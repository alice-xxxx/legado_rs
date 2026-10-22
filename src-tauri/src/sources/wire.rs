//! 原生引擎中以文本传输的结构字段；数据库只接收已解析的类型。
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TextOr<T> {
    Text(String),
    Data(T),
}
impl<T: Serialize> Serialize for TextOr<T> {
    fn serialize<S: serde::Serializer>(&self, output: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => output.serialize_str(text),
            Self::Data(value) => output
                .serialize_str(&serde_json::to_string(value).map_err(serde::ser::Error::custom)?),
        }
    }
}
impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for TextOr<T> {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(input)?;
        if let serde_json::Value::String(text) = value {
            return match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => serde_json::from_value(value)
                    .map(Self::Data)
                    .map_err(serde::de::Error::custom),
                Err(_) => Ok(Self::Text(text)),
            };
        }
        serde_json::from_value(value)
            .map(Self::Data)
            .map_err(serde::de::Error::custom)
    }
}
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
