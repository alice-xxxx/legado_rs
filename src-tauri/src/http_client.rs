//! Browser-style defaults shared by every Rust HTTP entry point.
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
use reqwest::{Client, ClientBuilder};

// Keep this aligned with Kotlin AppConst.DEFAULT_USER_AGENT.
const USER_AGENT_VALUE: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub(crate) fn builder() -> ClientBuilder {
    builder_with_user_agent(true)
}

pub(crate) fn builder_with_user_agent(include_user_agent: bool) -> ClientBuilder {
    let mut headers = HeaderMap::new();
    if include_user_agent {
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    }
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );
    // reqwest only fills missing headers; explicit source/request headers win.
    // Compression negotiation, connections and HTTP version selection belong to the client.
    Client::builder().default_headers(headers)
}

pub(crate) fn raw_builder() -> ClientBuilder {
    // The KMP bridge handles explicitly requested encodings itself. Byte-range
    // media also needs its original representation and length to remain intact.
    builder().no_gzip().no_brotli().no_deflate().no_zstd()
}
