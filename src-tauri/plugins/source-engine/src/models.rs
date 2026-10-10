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

/// Private native browser login capabilities. These payloads only cross the Rust/native plugin
/// boundary; the application WebView receives only the opaque session ID.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebStart {
    pub source_id: String,
    pub session_id: String,
    pub source_revision: u64,
    pub restore_epoch: u64,
    pub login_url: String,
    pub cookie_header: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebSession {
    pub source_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebStarted {
    pub started: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebCapture {
    pub login_url: String,
    pub source_revision: u64,
    pub restore_epoch: u64,
    pub cookie_header: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLoginWebCancelled {
    pub cancelled: bool,
}
