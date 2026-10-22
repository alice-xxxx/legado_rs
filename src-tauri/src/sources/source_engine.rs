//! 通过稳定请求结构调用 Kotlin/KMP 书源引擎。
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Command as StdCommand;
use std::sync::OnceLock;
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
};

#[cfg(all(
    feature = "desktop",
    not(any(target_os = "android", target_os = "ios"))
))]
#[path = "source_browser.rs"]
mod desktop_browser;

/// Register the running desktop application's handle for private one-shot source renderers.
/// The source-browser JNI callback uses a hidden Tauri webview only as a DOM renderer; it never
/// opens a source URL or uses app command/event IPC.
#[cfg(all(
    feature = "desktop",
    not(any(target_os = "android", target_os = "ios"))
))]
pub fn install_browser_app(app: tauri::AppHandle) {
    desktop_browser::install_app(app);
}

#[cfg(all(
    feature = "desktop",
    not(any(target_os = "android", target_os = "ios"))
))]
pub(crate) fn browser_json(request_json: &str) -> String {
    desktop_browser::browser_json(request_json)
}

#[cfg(all(
    feature = "desktop",
    not(any(target_os = "android", target_os = "ios"))
))]
pub(crate) use desktop_browser::{cancel_web_login, complete_web_login, start_web_login};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEngineRequest {
    /// Rust-only execution lease; never part of the source-rule wire format.
    #[serde(skip)]
    pub cancellation: Option<tokio_util::sync::CancellationToken>,
    pub operation: String,
    pub source: Value,
    pub keyword: Option<String>,
    pub credentials: Option<std::collections::HashMap<String, String>>,
    pub action_id: Option<usize>,
    pub media_url: Option<String>,
    pub media_headers: Option<std::collections::HashMap<String, String>>,
    pub page: Option<i32>,
    pub book: Option<Value>,
    pub chapter: Option<Value>,
    pub next_chapter_url: Option<String>,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub(crate) fn decode_browser_html(url: &str) -> Result<Option<String>, String> {
    let Some(data) = url.strip_prefix("data:") else {
        return Ok(None);
    };
    let (format, body) = data
        .split_once(',')
        .ok_or("Source browser HTML is invalid")?;
    let format = format.to_ascii_lowercase();
    if !matches!(
        format.as_str(),
        "text/html"
            | "text/html;charset=utf-8"
            | "text/html;base64"
            | "text/html;charset=utf-8;base64"
    ) {
        return Err("Source browser data must be HTML".into());
    }
    let html = if format.ends_with(";base64") {
        use base64::Engine;
        let body: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|_| "Source browser HTML encoding is invalid")?;
        String::from_utf8(bytes).map_err(|_| "Source browser HTML must be UTF-8")?
    } else {
        percent_encoding::percent_decode_str(body)
            .decode_utf8()
            .map_err(|_| "Source browser HTML must be UTF-8")?
            .into_owned()
    };
    Ok(Some(html))
}

/// Prefer the self-contained runtime from Tauri's resource directory; the Kotlin checkout is a
/// development fallback only and must never be required by an installed desktop package.
pub async fn execute_desktop(
    request: SourceEngineRequest,
    data_dir: PathBuf,
    resource_dir: Option<PathBuf>,
) -> Result<Value, String> {
    if request
        .cancellation
        .as_ref()
        .is_some_and(|token| token.is_cancelled())
    {
        return Err("Source execution cancelled".into());
    }
    let bundled_runtime = resource_dir
        .map(|directory| directory.join("source-engine"))
        .filter(|directory| directory.join("classpath.txt").is_file());

    let (runtime_dir, bundled_java_home) = if let Some(runtime_dir) = bundled_runtime {
        (runtime_dir.clone(), Some(runtime_dir.join("jre")))
    } else {
        let project_dir = source_engine_project_dir()?;
        let runtime_dir = project_dir.join("build/source-engine/runtime");
        let classpath_file = runtime_dir.join("classpath.txt");
        let quickjs_library = quickjs_native_library(&runtime_dir);
        if !classpath_file.is_file()
            || !quickjs_library.is_file()
            || source_engine_inputs_changed(&project_dir, &classpath_file)?
        {
            let module = project_dir.clone();
            tokio::task::spawn_blocking(move || prepare_source_engine(&module))
                .await
                .map_err(|error| format!("Failed to prepare Kotlin source engine: {error}"))??;
        }
        let bundled_java_home = runtime_dir.join("jre");
        (
            runtime_dir,
            bundled_java_home.is_dir().then_some(bundled_java_home),
        )
    };

    let classpath_file = runtime_dir.join("classpath.txt");
    let quickjs_library = quickjs_native_library(&runtime_dir);
    if !classpath_file.is_file() {
        return Err(format!(
            "Source-engine runtime is missing its classpath: {}",
            classpath_file.display()
        ));
    }
    if !quickjs_library.is_file() {
        return Err(format!(
            "Source-engine runtime is missing QuickJS: {}",
            quickjs_library.display()
        ));
    }
    let classpath_entries = tokio::fs::read_to_string(&classpath_file)
        .await
        .map_err(|error| format!("Cannot read source-engine classpath: {error}"))?;
    let classpath_entries = classpath_entries
        .lines()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let path = PathBuf::from(entry);
            if path.is_absolute() {
                path
            } else {
                runtime_dir.join(path)
            }
        })
        .collect::<Vec<_>>();
    if classpath_entries.is_empty() {
        return Err(
            "Source-engine classpath is empty; prepare the Kotlin source engine first".into(),
        );
    }
    let classpath = std::env::join_paths(classpath_entries)
        .map_err(|error| format!("Cannot assemble source-engine classpath: {error}"))?
        .to_string_lossy()
        .into_owned();
    tokio::fs::create_dir_all(&data_dir)
        .await
        .map_err(|error| format!("Cannot create source-engine data directory: {error}"))?;

    let request_json = serde_json::to_string(&request)
        .map_err(|error| format!("Cannot serialize source-engine input: {error}"))?;
    let quickjs_for_thread = quickjs_library.clone();
    let cancellation = request.cancellation.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::source_jni::execute(
            &classpath,
            &quickjs_for_thread,
            &request_json,
            data_dir,
            bundled_java_home,
            cancellation,
        )
    })
    .await
    .map_err(|error| format!("Embedded source-engine worker failed: {error}"))??;
    decode_result(&result)
}

/// The staged classpath is the development runtime's build stamp. Rebuild it
/// when one of the bridge inputs changes so `tauri dev` refreshes the embedded
/// KMP executor after Rust's Cargo watcher restarts the app.
fn source_engine_inputs_changed(project_dir: &Path, classpath_file: &Path) -> Result<bool, String> {
    let staged_at = std::fs::metadata(classpath_file)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("Cannot inspect source-engine runtime stamp: {error}"))?;
    for relative in [
        "kmp-engine/src/commonMain/kotlin/io/legado/sourceengine/bridge/SourceEngineHost.kt",
        "kmp-engine/src/commonMain/kotlin/io/legado/sourceengine/bridge/SourceEngineHostWire.kt",
        "kmp-engine/src/commonMain/kotlin/io/legado/sourceengine/bridge/SourceEngineCallCodec.kt",
        "kmp-engine/src/commonMain/kotlin/io/legado/sourceengine/bridge/WebBookSourceRuleExecutor.kt",
        "kmp-engine/src/commonMain/kotlin/io/legado/app/help/http/CookieStoreBase.kt",
        "kmp-engine/src/commonMain/kotlin/io/legado/app/help/http/CookieStoreProviderShared.kt",
        "kmp-engine/src/jvmMain/kotlin/io/legado/sourceengine/bridge/SourceEngineEmbedded.kt",
        "kmp-engine/src/jvmMain/kotlin/io/legado/sourceengine/bridge/SourceHostThread.kt",
        "kmp-engine/src/jvmMain/kotlin/io/legado/sourceengine/bridge/http/RustHostHttpProvider.kt",
        // Rust 在此类上注册 JNI host 回调，声明变化时必须刷新 staged JAR。
        "kmp-engine/src/jvmMain/kotlin/io/legado/sourceengine/bridge/http/SourceEngineNativeHost.kt",
        "src/native/cpp/jni/jni_bridge.cpp",
        "src/native/quickjs-ng/quickjs.c",
        "src/native/quickjs-ng/quickjs.h",
    ] {
        let input = project_dir.join(relative);
        let modified = match std::fs::metadata(&input) {
            Ok(metadata) => metadata
                .modified()
                .map_err(|error| format!("Cannot inspect source-engine input: {error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot inspect source-engine input: {error}")),
        };
        if modified > staged_at {
            return Ok(true);
        }
    }
    Ok(false)
}

fn source_engine_project_dir() -> Result<PathBuf, String> {
    if let Some(module_dir) = std::env::var_os("LEGADO_SOURCE_ENGINE_PROJECT_DIR") {
        return Ok(PathBuf::from(module_dir));
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(|tauri_dir| tauri_dir.join("kotlin"))
        .filter(|module_dir| module_dir.join("settings.gradle.kts").is_file())
        .ok_or_else(|| "Cannot locate the standalone Kotlin source-engine module".to_owned())
}

fn quickjs_native_library(runtime_dir: &Path) -> PathBuf {
    // Gradle stages one target-matched QuickJS library beside the portable classpath.
    let library = if cfg!(target_os = "windows") {
        "legado_quickjs.dll"
    } else if cfg!(target_os = "macos") {
        "liblegado_quickjs.dylib"
    } else {
        "liblegado_quickjs.so"
    };
    runtime_dir.join("native").join(library)
}

pub(super) fn java_executable() -> Result<PathBuf, String> {
    static JAVA_EXECUTABLE: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    JAVA_EXECUTABLE.get_or_init(find_java_executable).clone()
}

fn find_java_executable() -> Result<PathBuf, String> {
    if let Some(java) = std::env::var_os("LEGADO_JAVA") {
        let java = PathBuf::from(java);
        return match java_major_version(&java) {
            Some(major) if major >= 21 => Ok(java),
            Some(major) => Err(format!(
                "LEGADO_JAVA points to Java {major}; source engine requires Java 21 or newer"
            )),
            None => Err(format!(
                "LEGADO_JAVA does not point to a usable Java executable: {}",
                java.display()
            )),
        };
    }

    let mut detected = Vec::new();
    if let Some(java_home) = std::env::var_os("JAVA_HOME") {
        let java_home = PathBuf::from(java_home);
        let java = java_in_home(&java_home);
        if let Some(major) = java_major_version(&java) {
            if major >= 21 {
                return Ok(java);
            }
            detected.push(format!("{} (Java {major})", java_home.display()));
        }
    }

    if let Some(gradle_home) = gradle_user_home() {
        if let Ok(entries) = std::fs::read_dir(gradle_home.join("jdks")) {
            let mut toolchains = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|home| home.is_dir())
                .filter_map(|home| {
                    let java = java_in_home(&home);
                    java_major_version(&java).map(|major| (major, java, home))
                })
                .collect::<Vec<_>>();
            toolchains
                .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.2.cmp(&right.2)));
            if let Some((_, java, _)) = toolchains.iter().rev().find(|(major, _, _)| *major >= 21) {
                return Ok(java.clone());
            }
            detected.extend(
                toolchains
                    .into_iter()
                    .map(|(major, _, home)| format!("{} (Java {major})", home.display())),
            );
        }
    }

    let path_java = PathBuf::from(if cfg!(windows) { "java.exe" } else { "java" });
    if let Some(major) = java_major_version(&path_java) {
        if major >= 21 {
            return Ok(path_java);
        }
        detected.push(format!("PATH java (Java {major})"));
    }

    let details = if detected.is_empty() {
        String::new()
    } else {
        format!(" Detected: {}.", detected.join(", "))
    };
    Err(format!(
        "The source engine requires Java 21 or newer. Set JAVA_HOME or LEGADO_JAVA to a Java 21+ installation.{details}"
    ))
}

fn java_in_home(java_home: &Path) -> PathBuf {
    java_home
        .join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" })
}

fn gradle_user_home() -> Option<PathBuf> {
    std::env::var_os("GRADLE_USER_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .map(|home| home.join(".gradle"))
        })
}

fn java_major_version(java: &Path) -> Option<u32> {
    if let Some(java_home) = java.parent().and_then(Path::parent) {
        if !java_home.as_os_str().is_empty() {
            if let Ok(release) = std::fs::read_to_string(java_home.join("release")) {
                if let Some(version) = release
                    .lines()
                    .find_map(|line| line.strip_prefix("JAVA_VERSION="))
                {
                    if let Some(major) = parse_java_major(version.trim_matches('"')) {
                        return Some(major);
                    }
                }
            }
        }
    }

    let output = StdCommand::new(java).arg("-version").output().ok()?;
    let details = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    details
        .split('"')
        .find_map(|part| parse_java_major(part.trim()))
}

fn parse_java_major(version: &str) -> Option<u32> {
    let mut parts = version.split('.').filter_map(|part| {
        part.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .ok()
    });
    let first = parts.next()?;
    if first == 1 {
        parts.next()
    } else {
        Some(first)
    }
}

fn prepare_source_engine(module_dir: &Path) -> Result<(), String> {
    // 开发 checkout 首次运行时才构建桌面 JVM 运行时。Tauri 安装包的 JVM 与 native 依赖打包
    // 尚未纳入此流程，因此不要把这个开发期 Gradle fallback 当成已完成的独立分发。
    // 将 Rust 已验证可运行的 JDK 位置传给 Gradle，确保它使用同一个 Java 21 toolchain。
    let java_home = java_executable()?
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "Cannot determine JAVA_HOME for the Kotlin build".to_owned())?
        .to_path_buf();
    #[cfg(windows)]
    let output = {
        let wrapper = module_dir.join("gradlew.bat");
        let escaped_wrapper = wrapper.to_string_lossy().replace('\'', "''");
        let script = format!(
            "& '{escaped_wrapper}' --no-daemon --no-configuration-cache --console=plain prepareDesktopJvmRuntime"
        );
        StdCommand::new("powershell.exe")
            .env("JAVA_HOME", &java_home)
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .current_dir(module_dir)
            .output()
    };
    #[cfg(not(windows))]
    let output = {
        let mut command = StdCommand::new("sh");
        command
            .env("JAVA_HOME", &java_home)
            .arg(module_dir.join("gradlew"))
            .args([
                "--no-daemon",
                "--no-configuration-cache",
                "--console=plain",
                "prepareDesktopJvmRuntime",
            ])
            .current_dir(module_dir)
            .output()
    };

    let output =
        output.map_err(|error| format!("Cannot run Gradle to prepare source engine: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "Gradle could not prepare the Kotlin source engine.\n{}\n{}",
        stdout
            .chars()
            .rev()
            .take(3000)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>(),
        stderr
            .chars()
            .rev()
            .take(3000)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>(),
    ))
}

impl SourceEngineRequest {
    /// 公共请求默认值集中维护，调用方只填写本次操作的参数。
    pub fn new(operation: &str, source: &Value) -> Self {
        Self {
            operation: operation.into(),
            source: source.clone(),
            cancellation: None,
            keyword: None,
            credentials: None,
            action_id: None,
            media_url: None,
            media_headers: None,
            page: None,
            book: None,
            chapter: None,
            next_chapter_url: None,
        }
    }

    /// 书籍类型由当前源决定；保留本地标记，避免换源时带入旧类型。
    pub fn with_book(mut self, mut book: Value) -> Self {
        if book.is_object() {
            let flags =
                book["type"].as_u64().unwrap_or_default() & !(8 | 32 | 64 | 128 | 2048 | 4096);
            // KMP 的 Book.type 是位标记，与书源的单值类型不同。
            let kind = match self.source["bookSourceType"].as_u64() {
                Some(1) => 32,
                Some(2) => 64,
                Some(3) => 8 | 128,
                Some(4) => 2048,
                Some(5) => 4096,
                _ if crate::rss::is_rss_source(&self.source) => 4096,
                _ => 8,
            };
            book["type"] = serde_json::json!(flags | kind);
        }
        self.book = Some(book);
        self
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
pub struct TauriSourceExecutor {
    _app: tauri::AppHandle,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    data_dir: PathBuf,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    resource_dir: Option<PathBuf>,
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl TauriSourceExecutor {
    pub fn new(app: tauri::AppHandle, _data_dir: PathBuf, _resource_dir: Option<PathBuf>) -> Self {
        Self {
            _app: app,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            data_dir: _data_dir,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            resource_dir: _resource_dir,
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile-runtime"))]
impl SourceExecutor for TauriSourceExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                use tauri_plugin_source_engine::{SourceEngineCall, SourceEngineExt};

                // The plugin bridge waits synchronously for the native call.
                // Keep it off Tokio's worker lane so search cancellation can run.
                let app = self._app.clone();
                let cancellation = request.cancellation.clone();
                let data_dir = crate::app_data_root(&app)
                    .map_err(|error| error.to_string())?
                    .join("source-engine");
                let response = tokio::task::spawn_blocking(move || {
                    let mut wire =
                        serde_json::to_value(&request).map_err(|error| error.to_string())?;
                    let _execution = crate::source_host_ffi::ExecutionScope::enter(
                        &mut wire,
                        data_dir,
                        cancellation,
                    )?;
                    let request_json = wire.to_string();
                    app.source_engine()
                        .execute(SourceEngineCall { request_json })
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("Native source worker failed: {error}"))??;
                decode_result(&response.result_json)
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                execute_desktop(request, self.data_dir.clone(), self.resource_dir.clone()).await
            }
        })
    }
}

pub type EngineFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

/// The boundary for executing a source rule. A source definition is only ever
/// supplied to this executor by Rust, never by the WebView.
pub trait SourceExecutor: Send + Sync {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a>;
}

fn decode_result(result: &str) -> Result<Value, String> {
    let result: Value = serde_json::from_str(result)
        .map_err(|error| format!("Kotlin source engine returned invalid JSON: {error}"))?;
    if let Some(error) = result.get("__sourceEngineError").and_then(Value::as_str) {
        return Err(format!("Kotlin source engine failed:\n{error}"));
    }
    Ok(result)
}
