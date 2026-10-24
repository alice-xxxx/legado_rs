//! Embedded desktop JVM runtime and artifact loading.
use crate::{ExecutionContext, api::Method};
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

/// Runtime artifacts are prepared by the plugin's Gradle build, never by a running app.
pub(crate) async fn invoke<Q, T>(
    method: Method,
    request: Q,
    context: ExecutionContext,
) -> Result<T, String>
where
    Q: Serialize + Send,
    T: DeserializeOwned,
{
    let ExecutionContext {
        data_dir,
        resource_dir,
        storage,
        cancellation,
    } = context;
    if cancellation
        .as_ref()
        .is_some_and(|token| token.is_cancelled())
    {
        return Err("Source execution cancelled".into());
    }
    let bundled = resource_dir.map(|root| root.join("source-engine"));
    let runtime = if let Some(runtime) = bundled.filter(|root| root.join("classpath.txt").is_file())
    {
        runtime
    } else if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/build/source-engine/runtime")
    } else {
        return Err("The application is missing its prepared source-engine runtime".into());
    };
    let library = if cfg!(target_os = "windows") {
        "legado_quickjs.dll"
    } else if cfg!(target_os = "macos") {
        "liblegado_quickjs.dylib"
    } else {
        "liblegado_quickjs.so"
    };
    let quickjs = runtime.join("native").join(library);
    let java_home = runtime.join("jre");
    if !quickjs.is_file() || !java_home.is_dir() {
        return Err(format!(
            "Source-engine runtime is incomplete: {}",
            runtime.display()
        ));
    }
    let entries = tokio::fs::read_to_string(runtime.join("classpath.txt"))
        .await
        .map_err(|error| format!("Cannot read prepared source-engine classpath: {error}"))?;
    let entries = entries
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|entry| {
            let path = PathBuf::from(entry);
            if path.is_absolute() {
                path
            } else {
                runtime.join(path)
            }
        })
        .collect::<Vec<_>>();
    if entries.is_empty() || entries.iter().any(|entry| !entry.is_file()) {
        return Err(format!(
            "Source-engine classpath is incomplete: {}",
            runtime.display()
        ));
    }
    let classpath = std::env::join_paths(entries)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    tokio::fs::create_dir_all(&data_dir)
        .await
        .map_err(|error| error.to_string())?;
    let input = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let result = tokio::task::spawn_blocking(move || {
        crate::jni_bridge::execute(
            &classpath,
            &quickjs,
            method,
            &input,
            data_dir,
            java_home,
            cancellation,
            storage,
        )
    })
    .await
    .map_err(|error| format!("Source-engine worker failed: {error}"))??;
    serde_json::from_str(&result)
        .map_err(|error| format!("Invalid source-engine response: {error}"))
}
