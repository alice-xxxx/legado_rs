//! 连接 Android JNI 层与 Kotlin/KMP 书源引擎。
//! 桌面 Rust 进程内的 JVM/JNI 边界。
//!
//! Rust 宿主通过此模块在进程内调用 KMP/JVM；Kotlin 的网络与存储请求回调到 Rust Host。
//! JVM 初始化由锁保护；每次执行使用独立的宿主、取消信号和截止时间。

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop {
    use crate::http::SourceHttpRequest;
    use crate::storage::{SourceStorageRequest, execute_source_storage_request};
    use jni::objects::{JClass, JString, JValue};
    use jni::{
        Env, InitArgsBuilder, JNIVersion, JavaVM, NativeMethod, jni_sig, jni_str, native_method,
    };
    use serde_json::{Value, json};
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use tokio::runtime::Runtime;

    static JVM: OnceLock<JavaVM> = OnceLock::new();
    static JVM_INIT_LOCK: Mutex<()> = Mutex::new(());
    static HOST_RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

    // jni-macros 0.22.4 展开的 native_method! 内部仍调用已弃用的 fetch_update。
    // 仅在这张 JNI 方法表局部屏蔽第三方宏展开的 warning。
    #[allow(
        deprecated,
        reason = "jni-macros 0.22.4 expands native_method! with deprecated fetch_update"
    )]
    const NATIVE_METHODS: &[NativeMethod] = &[
        native_method! {
            static fn native_http_request(event_json: JString) -> JString,
        },
        native_method! {
            static fn native_execution_status(execution_id: JString) -> JString,
        },
        native_method! {
            static fn native_storage_request(event_json: JString) -> JString,
        },
        native_method! {
            static fn native_image_request(event_json: JString) -> JString,
        },
        #[cfg(feature = "desktop-runtime")]
        native_method! {
            static fn native_browser_snapshot(request_json: JString) -> JString,
        },
    ];

    /// Native workers have independent host leases; Kotlin serializes rule execution,
    /// while suspending HTTP calls release its rule lane for other sources.
    pub fn execute(
        classpath: &str,
        quickjs_library: &Path,
        method: crate::api::Method,
        request_json: &str,
        data_dir: PathBuf,
        bundled_java_home: PathBuf,
        cancellation: Option<tokio_util::sync::CancellationToken>,
        storage: std::sync::Arc<dyn crate::storage::StorageHost>,
    ) -> Result<String, String> {
        let execution =
            crate::host::ExecutionScope::enter(cancellation, storage, method.is_search())?;
        let kotlin_data_dir = jvm_compatible_path(&data_dir)
            .to_string_lossy()
            .into_owned();
        let vm = get_or_start_vm(classpath, quickjs_library, bundled_java_home)?;
        vm.attach_current_thread(|env| {
            Ok::<_, jni::errors::Error>(
                call_kotlin(env, method, request_json, &kotlin_data_dir, execution.id())
                    .map_err(|error| describe_error(env, error)),
            )
        })
        .map_err(|error| format!("JNI call to Kotlin source engine failed: {error}"))?
    }

    fn get_or_start_vm(
        classpath: &str,
        quickjs_library: &Path,
        bundled_java_home: PathBuf,
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

        let java_home = bundled_java_home;
        if !java_home.is_dir() {
            return Err(format!(
                "Prepared Java runtime is missing: {}",
                java_home.display()
            ));
        }
        // Tauri can return Windows `\\?\` device paths for packaged resources. Rust file I/O
        // accepts them, but HotSpot's Invocation API can pass that prefix into jimage lookup and
        // crash while opening `lib/modules`; normalize every path before it crosses into the JVM.
        let java_home = jvm_compatible_path(&java_home);
        let classpath = jvm_compatible_classpath(classpath)?;
        let quickjs_library = jvm_compatible_path(quickjs_library);
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
        unsafe {
            std::env::set_var("JAVA_HOME", &java_home);
        }
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
                unsafe {
                    std::env::set_var("PATH", path);
                }
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

    fn call_kotlin(
        env: &mut Env<'_>,
        method: crate::api::Method,
        request_json: &str,
        data_dir: &str,
        execution_id: &str,
    ) -> Result<String, jni::errors::Error> {
        use jni::{
            signature::{MethodSignature, RuntimeMethodSignature},
            strings::JNIString,
        };
        let (name, request_type, response_type) = method.signature();
        let dto = env.find_class(jni_str!("io/legado/sourceengine/bridge/NativeDtos"))?;
        let entry = env.find_class(jni_str!(
            "io/legado/sourceengine/bridge/SourceEngineEmbedded"
        ))?;
        let json = env.new_string(request_json)?;
        let decode_signature = RuntimeMethodSignature::from_str(format!(
            "(Ljava/lang/String;)Lio/legado/sourceengine/bridge/{request_type};"
        ))?;
        let request = env
            .call_static_method(
                &dto,
                JNIString::from(format!("decode{request_type}")),
                MethodSignature::from(&decode_signature),
                &[JValue::Object(&json)],
            )?
            .l()?;
        let directory = env.new_string(data_dir)?;
        let execution = env.new_string(execution_id)?;
        let signature = RuntimeMethodSignature::from_str(format!(
            "(Lio/legado/sourceengine/bridge/{request_type};Ljava/lang/String;Ljava/lang/String;){}",
            if response_type == "Book" {
                "Lio/legado/app/data/entities/Book;".to_owned()
            } else {
                format!("Lio/legado/sourceengine/bridge/{response_type};")
            }
        ))?;
        let result = env
            .call_static_method(
                entry,
                JNIString::from(name),
                MethodSignature::from(&signature),
                &[
                    JValue::Object(&request),
                    JValue::Object(&directory),
                    JValue::Object(&execution),
                ],
            )?
            .l()?;
        let response_class = if response_type == "Book" {
            "io/legado/app/data/entities/Book".to_owned()
        } else {
            format!("io/legado/sourceengine/bridge/{response_type}")
        };
        let encode_signature =
            RuntimeMethodSignature::from_str(format!("(L{response_class};)Ljava/lang/String;"))?;
        let output = env
            .call_static_method(
                &dto,
                JNIString::from(format!("encode{response_type}")),
                MethodSignature::from(&encode_signature),
                &[JValue::Object(&result)],
            )?
            .l()?;
        // The checked JNI signature returns java.lang.String.
        unsafe { JString::from_raw(env, output.into_raw()) }.try_to_string(env)
    }

    fn describe_error(env: &mut Env<'_>, error: jni::errors::Error) -> String {
        let message = (|| {
            let Some(exception) = env.exception_occurred() else {
                return Ok::<_, jni::errors::Error>(None);
            };
            env.exception_clear();
            let output = env
                .call_static_method(
                    jni_str!("io/legado/sourceengine/bridge/SourceEngineEmbedded"),
                    jni_str!("describeError"),
                    jni_sig!("(Ljava/lang/Throwable;)Ljava/lang/String;"),
                    &[JValue::Object(&exception)],
                )?
                .l()?;
            Ok(Some(
                unsafe { JString::from_raw(env, output.into_raw()) }.try_to_string(env)?,
            ))
        })();
        let _ = env.exception_clear();
        match message {
            Ok(Some(message)) => format!("Kotlin source engine failed:\n{message}"),
            _ => format!("JNI call to Kotlin source engine failed: {error}"),
        }
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

    fn native_execution_status<'local>(
        env: &mut Env<'local>,
        _class: JClass<'local>,
        id: JString<'local>,
    ) -> Result<JString<'local>, jni::errors::Error> {
        let id = id.try_to_string(env)?;
        env.new_string(if crate::host::execution_cancelled(Some(&id)) {
            "true"
        } else {
            "false"
        })
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

    fn native_image_request<'local>(
        env: &mut Env<'local>,
        _class: JClass<'local>,
        event_json: JString<'local>,
    ) -> Result<JString<'local>, jni::errors::Error> {
        let input = event_json.try_to_string(env)?;
        env.new_string(crate::host::image_json(&input))
    }

    #[cfg(feature = "desktop-runtime")]
    fn native_browser_snapshot<'local>(
        env: &mut Env<'local>,
        _class: JClass<'local>,
        request_json: JString<'local>,
    ) -> Result<JString<'local>, jni::errors::Error> {
        let input = request_json.try_to_string(env)?;
        env.new_string(crate::browser::browser_json(&input))
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
            run_on_host_runtime(async move { crate::host::execute_cancellable_http(request).await })
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
            run_on_host_runtime(async move { execute_source_storage_request(request).await })
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

    fn jvm_compatible_path(path: &Path) -> PathBuf {
        #[cfg(windows)]
        {
            let value = path.to_string_lossy();
            if let Some(value) = value.strip_prefix(r"\\?\") {
                if value
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
                {
                    return PathBuf::from(format!(r"\\{}", &value[4..]));
                }
                // Preserve non-drive device paths; only `\\?\C:\...` has a safe ordinary
                // Win32 spelling that HotSpot can use for its runtime image lookup.
                if value.as_bytes().get(1) == Some(&b':') {
                    return PathBuf::from(value.to_owned());
                }
            }
        }
        path.to_path_buf()
    }

    fn jvm_compatible_classpath(classpath: &str) -> Result<String, String> {
        let paths = std::env::split_paths(classpath)
            .map(|path| jvm_compatible_path(&path))
            .collect::<Vec<_>>();
        std::env::join_paths(paths)
            .map(|classpath| classpath.to_string_lossy().into_owned())
            .map_err(|error| format!("Cannot normalize JVM classpath: {error}"))
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

#[cfg(target_os = "android")]
mod android {
    use crate::host::{http_json, image_json, storage_json};
    use jni::errors::ThrowRuntimeExAndDefault;
    use jni::objects::{JObject, JString};
    use jni::{EnvUnowned, jni_mangle};

    #[jni_mangle("io.legado.sourceengine.android.RustSourceEngineNative")]
    pub fn native_execution_status<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _class: JObject<'local>,
        id: JString<'local>,
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| {
                let id = id.try_to_string(env)?;
                env.new_string(if crate::host::execution_cancelled(Some(&id)) {
                    "true"
                } else {
                    "false"
                })
            })
            .resolve::<ThrowRuntimeExAndDefault>()
    }

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
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| {
                let input = request_json.try_to_string(env)?;
                env.new_string(storage_json(&input))
            })
            .resolve::<ThrowRuntimeExAndDefault>()
    }

    #[jni_mangle("io.legado.sourceengine.android.RustSourceEngineNative")]
    pub fn native_image_request<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _class: JObject<'local>,
        request_json: JString<'local>,
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| {
                let input = request_json.try_to_string(env)?;
                env.new_string(image_json(&input))
            })
            .resolve::<ThrowRuntimeExAndDefault>()
    }
}
