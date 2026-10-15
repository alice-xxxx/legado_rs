//! Browser-style defaults shared by every Rust HTTP entry point.
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
use reqwest::{Client, ClientBuilder};

/// Tauri/Tao installs the process application context before running the app.
/// Initialize once, before any service can send HTTPS on a Tokio worker.
#[cfg(target_os = "android")]
pub(crate) fn init_android_tls() -> Result<(), jni::errors::Error> {
    use jni::{objects::JObject, refs::Reference, JavaVM};

    let android = ndk_context::android_context();
    // SAFETY: Tao owns the process VM and a global application Context reference.
    let vm = unsafe { JavaVM::from_raw(android.vm().cast()) };
    vm.attach_current_thread_for_scope(|env| {
        // Borrow Tao's global reference without taking ownership, then create
        // our own local reference in this frame. The verifier retains its own globals.
        let borrowed = unsafe { JObject::kind_from_raw(android.context().cast()) };
        let context = env.new_local_ref(&borrowed)?;
        rustls_platform_verifier::android::init_with_env(env, context)
    })
}

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
