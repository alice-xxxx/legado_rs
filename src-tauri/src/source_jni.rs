//! 桌面 Rust 进程内的 JVM/JNI 边界。
//!
//! Rust 宿主通过此模块在进程内调用 KMP/JVM；Kotlin 的网络与存储请求回调到 Rust Host。
//! JNI VM 生命周期和请求锁保护进程级 provider 与 QuickJS 状态。

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop {
    use crate::source_http::{execute_source_http_request, SourceHttpRequest};
    use crate::source_storage::{execute_source_storage_request, SourceStorageRequest};
    use jni::objects::{JClass, JString, JValue};
    use jni::{
        jni_sig, jni_str, native_method, Env, InitArgsBuilder, JNIVersion, JavaVM, NativeMethod,
    };
    use serde_json::{json, Value};
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use tokio::runtime::Runtime;

    static JVM: OnceLock<JavaVM> = OnceLock::new();
    static JVM_INIT_LOCK: Mutex<()> = Mutex::new(());
    static EXECUTION_LOCK: Mutex<()> = Mutex::new(());
    static HOST_RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    // 书源执行被 EXECUTION_LOCK 串行化，但 Kotlin 的 OkHttp 会把网络调用派发到自己的
    // worker 线程。因此上下文必须跨 JNI 回调线程可见；作用域锁保证路径不会和另一请求串线。
    static CURRENT_DATA_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

    const NATIVE_METHODS: &[NativeMethod] = &[
        native_method! {
            static fn native_http_request(event_json: JString) -> JString,
        },
        native_method! {
            static fn native_storage_request(event_json: JString) -> JString,
        },
    ];

    struct DataDirScope(Option<PathBuf>);

    impl DataDirScope {
        fn enter(path: PathBuf) -> Self {
            let previous = CURRENT_DATA_DIR
                .write()
                .expect("active app data directory lock")
                .replace(path);
            Self(previous)
        }
    }

    impl Drop for DataDirScope {
        fn drop(&mut self) {
            *CURRENT_DATA_DIR
                .write()
                .expect("active app data directory lock") = self.0.take();
        }
    }

    /// 调用 JVM 中的 Kotlin 入口。Rust 侧锁定整个解析操作，保证进程级 Provider/QuickJS
    /// 状态不会被两个书源请求交错修改。
    pub fn execute(
        classpath: &str,
        quickjs_library: &Path,
        request_json: &str,
        data_dir: PathBuf,
        bundled_java_home: Option<PathBuf>,
    ) -> Result<String, String> {
        let _execution = EXECUTION_LOCK
            .lock()
            .map_err(|_| "Embedded source-engine execution lock is poisoned".to_owned())?;
        let _data_dir = DataDirScope::enter(data_dir);
        let vm = get_or_start_vm(classpath, quickjs_library, bundled_java_home)?;
        vm.attach_current_thread(|env| call_kotlin(env, request_json))
            .map_err(|error| format!("JNI call to Kotlin source engine failed: {error}"))
    }

    fn get_or_start_vm(
        classpath: &str,
        quickjs_library: &Path,
        bundled_java_home: Option<PathBuf>,
    ) -> Result<&'static JavaVM, String> {
        if let Some(vm) = JVM.get() {
            return Ok(vm);
        }
        let _guard = JVM_INIT_LOCK
            .lock()
            .map_err(|_| "Embedded JVM initialization lock is poisoned".to_owned())?;
        if let Some(vm) = JVM.get() {
            return Ok(vm);
        }

        // Packaged desktop apps carry the exact Java image used for this runtime. Development runs
        // still fall back to JAVA_HOME/Gradle toolchains/PATH through source_engine.
        let java_home = match bundled_java_home {
            Some(java_home) if java_home.is_dir() => java_home,
            Some(java_home) => {
                return Err(format!(
                    "Bundled Java runtime is missing: {}",
                    java_home.display()
                ));
            }
            None => java_home()?,
        };
        let libjvm = libjvm_path(&java_home);
        if !libjvm.is_file() {
            return Err(format!(
                "Java 21+ JVM library was not found: {}",
                libjvm.display()
            ));
        }
        // Invocation API reads JAVA_HOME independently of the explicit libjvm path. Keep both
        // sources of JVM discovery on the same validated Java 21+ installation (the machine may
        // have an older Java earlier in its shell environment).
        std::env::set_var("JAVA_HOME", &java_home);
        #[cfg(windows)]
        {
            // Windows 的 jvm.dll 还会加载同目录的 JVM 依赖；把 JDK bin 放入进程搜索路径，
            // 这是在 Rust 进程中加载 JVM 动态库所需的装载环境，不改变书源请求路径。
            let bin = java_home.join("bin");
            let server = java_home.join("bin/server");
            let old_path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![bin, server];
            paths.extend(std::env::split_paths(&old_path));
            if let Ok(path) = std::env::join_paths(paths) {
                std::env::set_var("PATH", path);
            }
        }
        let args = InitArgsBuilder::new()
            .version(JNIVersion::V1_8)
            .option("-Dfile.encoding=UTF-8")
            .option("-Djava.awt.headless=true")
            // The Invocation API loads libjvm directly and has no java.exe launcher to infer
            // the runtime image. Set java.home to the exact JDK paired with that libjvm.
            .option(format!("-Djava.home={}", java_home.display()))
            .option(format!("-Djava.class.path={classpath}"))
            .option(format!(
                "-Dlegado.quickjs.lib={}",
                quickjs_library.display()
            ))
            .build()
            .map_err(|error| format!("Cannot create embedded JVM options: {error}"))?;
        let vm = JavaVM::with_libjvm(args, || Ok(libjvm))
            .map_err(|error| format!("Cannot start embedded JVM: {error}"))?;

        vm.attach_current_thread(|env| {
            let class = env.find_class(jni_str!(
                "io/legado/sourceengine/bridge/http/SourceEngineNativeHost"
            ))?;
            // 在 JVM 中注册当前 Rust Host 的 JNI 函数表，使 KMP 回调复用同一进程的
            // HTTP、Cookie 和存储实现。
            unsafe { env.register_native_methods(class, NATIVE_METHODS) }?;
            Ok::<_, jni::errors::Error>(())
        })
        .map_err(|error| format!("Cannot register Rust JNI host callbacks: {error}"))?;

        JVM.set(vm)
            .map_err(|_| "Embedded JVM was initialized concurrently".to_owned())?;
        Ok(JVM.get().expect("JVM was stored above"))
    }

    fn call_kotlin(env: &mut Env<'_>, request_json: &str) -> Result<String, jni::errors::Error> {
        let class = env.find_class(jni_str!(
            "io/legado/sourceengine/bridge/SourceEngineEmbedded"
        ))?;
        let request = env.new_string(request_json)?;
        let value = env
            .call_static_method(
                class,
                jni_str!("executeJson"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(request.as_ref())],
            )?
            .l()?;
        // call_static_method 已按 String 返回签名检查了类型；把 JObject 包回 JString 时
        // 只改变 Rust 类型标记，不创建/释放新的 JNI 引用。
        unsafe { JString::from_raw(env, value.into_raw()) }.try_to_string(env)
    }

    fn native_http_request<'local>(
        env: &mut Env<'local>,
        _class: JClass<'local>,
        event_json: JString<'local>,
    ) -> Result<JString<'local>, jni::errors::Error> {
        let input = event_json.try_to_string(env)?;
        let response = run_http_event(&input);
        env.new_string(response)
    }

    fn native_storage_request<'local>(
        env: &mut Env<'local>,
        _class: JClass<'local>,
        event_json: JString<'local>,
    ) -> Result<JString<'local>, jni::errors::Error> {
        let input = event_json.try_to_string(env)?;
        let response = run_storage_event(&input);
        env.new_string(response)
    }

    fn run_http_event(event_json: &str) -> String {
        let parsed = serde_json::from_str::<Value>(event_json);
        let response = parsed.map_err(|error| format!("Invalid JNI HTTP event: {error}"));
        let event = match response {
            Ok(event) => event,
            Err(error) => return json!({ "ok": false, "error": error }).to_string(),
        };
        let id = event.get("id").cloned().unwrap_or(Value::Null);
        let result = (|| {
            let request: SourceHttpRequest = serde_json::from_value(
                event
                    .get("request")
                    .cloned()
                    .ok_or_else(|| "JNI HTTP event has no request".to_owned())?,
            )
            .map_err(|error| format!("Invalid JNI HTTP request: {error}"))?;
            run_on_host_runtime(async move { execute_source_http_request(request).await })
        })();
        match result {
            Ok(response) => {
                json!({ "type": "httpResponse", "id": id, "ok": true, "response": response })
                    .to_string()
            }
            Err(error) => {
                json!({ "type": "httpResponse", "id": id, "ok": false, "error": error }).to_string()
            }
        }
    }

    fn run_storage_event(event_json: &str) -> String {
        let parsed = serde_json::from_str::<Value>(event_json);
        let event = match parsed {
            Ok(event) => event,
            Err(error) => {
                return json!({ "ok": false, "error": format!("Invalid JNI storage event: {error}") }).to_string()
            }
        };
        let id = event.get("id").cloned().unwrap_or(Value::Null);
        let result = (|| {
            let request: SourceStorageRequest = serde_json::from_value(event)
                .map_err(|error| format!("Invalid JNI storage request: {error}"))?;
            let data_dir = CURRENT_DATA_DIR
                .read()
                .map_err(|_| "JNI app data directory lock is poisoned".to_owned())?
                .clone()
                .ok_or_else(|| {
                    "JNI storage callback has no active app data directory".to_owned()
                })?;
            run_on_host_runtime(
                async move { execute_source_storage_request(request, &data_dir).await },
            )
        })();
        match result {
            Ok(response) => {
                json!({ "type": "storageResponse", "id": id, "ok": true, "value": response.value })
                    .to_string()
            }
            Err(error) => {
                json!({ "type": "storageResponse", "id": id, "ok": false, "error": error })
                    .to_string()
            }
        }
    }

    fn host_runtime() -> Result<&'static Runtime, String> {
        HOST_RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("legado-source-host")
                    .build()
                    .map_err(|error| format!("Cannot create Rust source host runtime: {error}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// JNI callbacks are synchronous, but a callback can arrive while the caller is a Tokio
    /// blocking worker. Schedule the async I/O on the dedicated Rust host runtime and wait on a
    /// channel; calling Runtime::block_on here could try to nest Tokio runtimes on one thread.
    fn run_on_host_runtime<T, F>(future: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, String>> + Send + 'static,
    {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        host_runtime()?.spawn(async move {
            let _ = sender.send(future.await);
        });
        receiver
            .recv()
            .map_err(|error| format!("Rust source host runtime stopped: {error}"))?
    }

    fn java_home() -> Result<PathBuf, String> {
        home_from_java(&crate::source_engine::java_executable()?)
    }

    fn home_from_java(java: &Path) -> Result<PathBuf, String> {
        // `canonicalize` adds Windows' `\\?\` device-path prefix. HotSpot's Windows path
        // initialization does not consistently accept that prefix for java.home/modules, so keep
        // the ordinary absolute path returned by JAVA_HOME/Gradle; only resolve bare PATH names.
        let resolved = if java.is_absolute() {
            java.to_path_buf()
        } else {
            let output =
                std::process::Command::new(if cfg!(windows) { "where.exe" } else { "which" })
                    .arg(java)
                    .output()
                    .map_err(|error| {
                        format!("Cannot resolve Java executable {}: {error}", java.display())
                    })?;
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("Cannot resolve Java executable {}", java.display()))?
        };
        resolved
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("Cannot determine JAVA_HOME from {}", java.display()))
    }

    fn libjvm_path(java_home: &Path) -> PathBuf {
        #[cfg(target_os = "windows")]
        return java_home.join("bin/server/jvm.dll");
        #[cfg(target_os = "macos")]
        return java_home.join("lib/server/libjvm.dylib");
        #[cfg(all(unix, not(target_os = "macos")))]
        return java_home.join("lib/server/libjvm.so");
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) use desktop::execute;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(super) fn execute(
    _classpath: &str,
    _quickjs_library: &std::path::Path,
    _request_json: &str,
    _data_dir: std::path::PathBuf,
    _bundled_java_home: Option<std::path::PathBuf>,
) -> Result<String, String> {
    Err("Desktop JVM embedding is unavailable on mobile targets".to_owned())
}

#[cfg(target_os = "android")]
mod android {
    use crate::source_host_ffi::{http_json, storage_json};
    use jni::errors::ThrowRuntimeExAndDefault;
    use jni::objects::{JObject, JString};
    use jni::{jni_mangle, EnvUnowned};
    use std::path::Path;

    /// Android KMP calls these native methods directly. They deliberately return the same JSON
    /// envelope as the iOS C ABI, so parser code sees one host protocol on every mobile target.
    #[jni_mangle("io.legado.sourceengine.android.RustSourceEngineNative")]
    pub fn native_http_request<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _class: JObject<'local>,
        request_json: JString<'local>,
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| {
                let input = request_json.try_to_string(env)?;
                env.new_string(http_json(&input))
            })
            .resolve::<ThrowRuntimeExAndDefault>()
    }

    #[jni_mangle("io.legado.sourceengine.android.RustSourceEngineNative")]
    pub fn native_storage_request<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _class: JObject<'local>,
        request_json: JString<'local>,
        app_data_dir: JString<'local>,
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| {
                let input = request_json.try_to_string(env)?;
                let data_dir = app_data_dir.try_to_string(env)?;
                env.new_string(storage_json(&input, Path::new(&data_dir)))
            })
            .resolve::<ThrowRuntimeExAndDefault>()
    }
}
