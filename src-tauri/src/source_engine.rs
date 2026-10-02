use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEngineRequest {
    pub operation: String,
    pub source: Value,
    pub keyword: Option<String>,
    pub page: Option<i32>,
    pub book: Option<Value>,
    pub chapter: Option<Value>,
    pub next_chapter_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineRequestWire<'a> {
    operation: &'a str,
    source: &'a Value,
    keyword: Option<&'a str>,
    page: Option<i32>,
    book: Option<&'a Value>,
    chapter: Option<&'a Value>,
    next_chapter_url: Option<&'a str>,
}

/// 执行桌面书源操作：Rust 通过 JNI 调用 KMP 解析器，并由 Rust Host 提供 HTTP 与存储。
pub async fn execute(request: SourceEngineRequest, data_dir: PathBuf) -> Result<Value, String> {
    let project_dir = source_engine_project_dir()?;
    let classpath_file = project_dir.join("build/source-engine/classpath.txt");
    let quickjs_library = quickjs_native_library(&project_dir);
    if !classpath_file.is_file() || !quickjs_library.is_file() {
        let module = project_dir.clone();
        tokio::task::spawn_blocking(move || prepare_source_engine(&module))
            .await
            .map_err(|error| format!("Failed to prepare Kotlin source engine: {error}"))??;
    }
    let classpath = tokio::fs::read_to_string(&classpath_file)
        .await
        .map_err(|error| format!("Cannot read source-engine classpath: {error}"))?;
    if classpath.trim().is_empty() {
        return Err(
            "Source-engine classpath is empty; prepare the Kotlin source engine first".into(),
        );
    }
    tokio::fs::create_dir_all(&data_dir)
        .await
        .map_err(|error| format!("Cannot create source-engine data directory: {error}"))?;

    let request_json = serde_json::to_string(&EngineRequestWire {
        operation: &request.operation,
        source: &request.source,
        keyword: request.keyword.as_deref(),
        page: request.page,
        book: request.book.as_ref(),
        chapter: request.chapter.as_ref(),
        next_chapter_url: request.next_chapter_url.as_deref(),
    })
    .map_err(|error| format!("Cannot serialize source-engine input: {error}"))?;
    let quickjs_for_thread = quickjs_library.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::source_jni::execute(
            classpath.trim(),
            &quickjs_for_thread,
            &request_json,
            data_dir,
        )
    })
    .await
    .map_err(|error| format!("Embedded source-engine worker failed: {error}"))??;
    let result: Value = serde_json::from_str(&result)
        .map_err(|error| format!("Kotlin source engine returned invalid JSON: {error}"))?;
    if let Some(error) = result.get("__sourceEngineError").and_then(Value::as_str) {
        return Err(format!("Kotlin source engine failed:\n{error}"));
    }
    Ok(result)
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

fn quickjs_native_library(module_dir: &Path) -> PathBuf {
    // QuickJS JNI 产物由独立 Kotlin 模块准备，按 Rust 当前目标定位，不查询源平台的构建目录。
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" | "amd64" | "x64" => "x86_64".to_owned(),
        "aarch64" | "arm64" => "aarch64".to_owned(),
        other => other
            .to_ascii_lowercase()
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || "_.-".contains(ch) {
                    ch
                } else {
                    '_'
                }
            })
            .collect(),
    };
    let library = if cfg!(target_os = "windows") {
        "legado_quickjs.dll"
    } else if cfg!(target_os = "macos") {
        "liblegado_quickjs.dylib"
    } else {
        "liblegado_quickjs.so"
    };
    module_dir
        .join("build/native-jvm")
        .join(format!("{os}-{arch}"))
        .join(library)
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

#[cfg(test)]
mod tests {
    use super::{execute, SourceEngineRequest};
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct FixtureServer {
        address: String,
        stop: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<(String, String, String)>>>,
        thread: Option<JoinHandle<()>>,
    }

    impl FixtureServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
            listener.set_nonblocking(true).expect("set nonblocking");
            let address = listener.local_addr().expect("fixture address").to_string();
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let worker_stop = Arc::clone(&stop);
            let worker_requests = Arc::clone(&requests);
            let thread = thread::spawn(move || {
                let mut handlers = Vec::new();
                while !worker_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        // 各 socket 独立处理；某个客户端若延迟发送 body，不应阻塞同一
                        // fixture 对后续书源请求（例如 JS ajax）的 accept。
                        Ok((stream, _)) => {
                            let requests = Arc::clone(&worker_requests);
                            handlers.push(thread::spawn(move || serve_request(stream, &requests)));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
                for handler in handlers {
                    let _ = handler.join();
                }
            });
            Self {
                address,
                stop,
                requests,
                thread: Some(thread),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.address)
        }
    }

    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn serve_request(mut stream: TcpStream, requests: &Mutex<Vec<(String, String, String)>>) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let Ok(clone) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(clone);
        let mut first_line = String::new();
        if reader.read_line(&mut first_line).is_err() {
            return;
        }
        let mut headers = String::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                return;
            }
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
            headers.push_str(&line);
        }
        let mut body = vec![0; content_length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let request_line = first_line.trim().to_owned();
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .to_owned();
        let body = String::from_utf8_lossy(&body).into_owned();
        requests
            .lock()
            .expect("request capture lock")
            .push((request_line.clone(), headers, body));

        let response_body = match path.as_str() {
            "/search" => "<div class='item'><h3><a href='/book'>Fixture Novel</a></h3><span class='author'>A. Writer</span></div>",
            "/book" => "<h1>Fixture Novel</h1><span class='author'>A. Writer</span><a class='toc' href='/toc'>TOC</a>",
            "/author" => "A. Writer",
            "/toc" => "<ul id='list'><li><a href='/chapter/1'>Chapter One</a></li><li><a href='/chapter/2'>Chapter Two</a></li></ul>",
            // @textNodes 只取选中节点的直接文本节点；这里保留与待测规则相同的 DOM 形状。
            _ if path.starts_with("/chapter/") => "<div class='content'>Fixture chapter body</div>",
            _ => "not found",
        };
        let status = if path == "/missing" {
            "404 Not Found"
        } else {
            "200 OK"
        };
        let cookie_header = if path == "/search" {
            "Set-Cookie: fixture-session=ok; Path=/\r\n"
        } else {
            ""
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n{cookie_header}Content-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn request(
        operation: &str,
        source: &Value,
        keyword: Option<&str>,
        book: Option<&Value>,
        chapter: Option<&Value>,
    ) -> SourceEngineRequest {
        SourceEngineRequest {
            operation: operation.to_owned(),
            source: source.clone(),
            keyword: keyword.map(str::to_owned),
            page: Some(1),
            book: book.cloned(),
            chapter: chapter.cloned(),
            next_chapter_url: None,
        }
    }

    fn temp_data_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "legado-source-engine-smoke-{}-{nonce}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn local_source_runs_search_info_toc_and_content_through_rust_http() {
        let server = FixtureServer::start();
        let base = server.base_url();
        let search_url = format!("{base}/search,") + r#"{"method":"POST","body":"q={{key}}"}"#;
        let source = json!({
            "bookSourceName": "Local fixture",
            "bookSourceUrl": base,
            "bookSourceType": 0,
            "searchUrl": search_url,
            "enabledCookieJar": true,
            "ruleSearch": {
                "bookList": "@css:.item",
                "name": "@css:h3 a@text",
                "author": "@css:.author@text",
                "bookUrl": "@css:h3 a@href"
            },
            "ruleBookInfo": {
                "name": "@css:h1@text",
                // 详情作者通过 QuickJS 的 java.ajax 二次请求，验证脚本网络请求同样由 Rust 执行。
                "author": format!("@js:java.ajax('{base}/author')"),
                "tocUrl": "@css:a.toc@href"
            },
            "ruleToc": {
                "chapterList": "@css:#list li",
                "chapterName": "@css:a@text",
                "chapterUrl": "@css:a@href"
            },
            "ruleContent": { "content": "@css:.content@textNodes" }
        });
        let data_dir = temp_data_dir();

        let search = execute(
            request("search", &source, Some("fixture"), None, None),
            data_dir.clone(),
        )
        .await
        .expect("search result");
        assert_eq!(search["books"][0]["name"], "Fixture Novel");

        let book = json!({
            "bookUrl": format!("{base}/book"),
            "origin": base,
            "originName": "Local fixture",
            "name": "Fixture Novel"
        });
        let book = execute(
            request("bookInfo", &source, None, Some(&book), None),
            data_dir.clone(),
        )
        .await
        .expect("book info result");
        assert_eq!(book["name"], "Fixture Novel");
        assert_eq!(
            book["author"],
            "A. Writer",
            "local fixture requests: {:?}",
            server
                .requests
                .lock()
                .expect("request capture lock")
                .as_slice()
        );
        assert_eq!(book["tocUrl"], format!("{base}/toc"));

        let chapters = execute(
            request("chapters", &source, None, Some(&book), None),
            data_dir.clone(),
        )
        .await
        .expect("chapter list result");
        assert_eq!(chapters.as_array().map(Vec::len), Some(2));
        let chapter = chapters[0].clone();

        let content = execute(
            request("content", &source, None, Some(&book), Some(&chapter)),
            data_dir.clone(),
        )
        .await
        .expect("chapter content result");
        assert!(content
            .as_str()
            .unwrap_or_default()
            .contains("Fixture chapter body"));

        let requests = server.requests.lock().expect("request capture lock");
        let post = requests
            .iter()
            .find(|(line, _, _)| line.starts_with("POST /search "))
            .expect("Rust sent search POST");
        assert!(post
            .1
            .to_ascii_lowercase()
            .contains("content-type: application/x-www-form-urlencoded"));
        assert!(post.2.contains("q=fixture"));
        assert!(requests
            .iter()
            .any(|(line, _, _)| line.starts_with("GET /author ")));
        // 搜索和详情是两个独立 JVM 操作；Cookie 能回来说明 Rust 持久化桥接跨进程生效。
        let book_request = requests
            .iter()
            .find(|(line, _, _)| line.starts_with("GET /book "))
            .expect("Rust sent book-info GET");
        assert!(book_request
            .1
            .to_ascii_lowercase()
            .contains("cookie: fixture-session=ok"));
        drop(requests);
        let _ = std::fs::remove_dir_all(data_dir);
    }
}
