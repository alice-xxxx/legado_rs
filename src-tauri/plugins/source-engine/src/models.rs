use serde::{Deserialize, Serialize};

/// The exact parser request JSON stays opaque until Kotlin decodes its shared wire format.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEngineCall {
    pub request_json: String,
}

/// The native adapter returns serialized parser output so Rust remains the IPC boundary.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEngineResponse {
    pub result_json: String,
}
