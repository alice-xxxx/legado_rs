//! 桌面 JVM 装载及类型化 JNI 调用；无平台注册和 HTTP 业务。
use crate::{EngineContext, Result};
use jni::{
    Env, InitArgsBuilder, JNIVersion, JavaVM, NativeMethod, jni_sig, jni_str, native_method,
    objects::{JClass, JString, JValue},
    signature::{MethodSignature, RuntimeMethodSignature},
    strings::JNIString,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};
static VM: OnceLock<JavaVM> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

#[allow(deprecated, reason = "JNI 宏内部仍使用已弃用的方法")]
const METHODS: &[NativeMethod] = &[
    native_method! {static fn http(input:JString)->JString},
    native_method! {static fn storage(input:JString)->JString},
    native_method! {static fn image(input:JString)->JString},
    native_method! {static fn cancelled(id:JString)->JString},
    native_method! {static fn browser(input:JString)->JString},
];
pub(crate) fn call<Q: Serialize, T: DeserializeOwned>(
    name: &str,
    request: &Q,
    context: &EngineContext,
    id: &str,
    request_type: &str,
    result_type: &str,
) -> Result<T> {
    let vm = vm(context)?;
    let request = serde_json::to_string(request).map_err(|e| e.to_string())?;
    let directory = crate::platform::java_path(&context.data_dir)
        .to_string_lossy()
        .into_owned();
    let text = vm
        .attach_current_thread(|env| {
            let result = invoke(
                env,
                name,
                &request,
                &directory,
                id,
                request_type,
                result_type,
            );
            Ok::<_, jni::errors::Error>(result.map_err(|error| error_text(env, error)))
        })
        .map_err(|e| e.to_string())??;
    crate::engine::context::get(id)?;
    serde_json::from_str(&text).map_err(|e| format!("引擎返回类型无效：{e}"))
}
fn vm(context: &EngineContext) -> Result<&'static JavaVM> {
    if let Some(vm) = VM.get() {
        return Ok(vm);
    }
    let _lock = LOCK.lock().map_err(|_| "JVM 初始化锁不可用")?;
    if let Some(vm) = VM.get() {
        return Ok(vm);
    }
    let bundled = context
        .resource_dir
        .as_ref()
        .map(|root| root.join("source-engine"));
    let root = match bundled.filter(|p| p.join("classpath.txt").is_file()) {
        Some(root) => root,
        None if cfg!(debug_assertions) => {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/build/source-engine/runtime")
        }
        None => return Err("缺少打包的源引擎运行时".into()),
    };
    let java = crate::platform::java_path(&root.join("jre"));
    let library = crate::platform::jvm_library(&java);
    if !library.is_file() {
        return Err(format!("缺少 JVM 动态库：{}", library.display()));
    }
    let list = std::fs::read_to_string(root.join("classpath.txt")).map_err(|e| e.to_string())?;
    let paths = list
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| crate::platform::java_path(&root.join(line)))
        .collect::<Vec<_>>();
    if paths.is_empty() || paths.iter().any(|p| !p.exists()) {
        return Err("引擎类路径不完整".into());
    }
    let classpath = std::env::join_paths(paths)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    let quickjs = crate::platform::java_path(&root.join("native").join(crate::platform::QUICKJS));
    if !quickjs.is_file() {
        return Err("缺少 QuickJS 动态库".into());
    }
    // 装载器和 HotSpot 使用同一个随应用分发的 JRE。
    unsafe {
        std::env::set_var("JAVA_HOME", &java);
    }
    let args = InitArgsBuilder::new()
        .version(JNIVersion::V1_8)
        .option("-Dfile.encoding=UTF-8")
        .option("-Djava.awt.headless=true")
        .option(format!("-Djava.home={}", java.display()))
        .option(format!("-Djava.class.path={classpath}"))
        .option(format!("-Dlegado.quickjs.lib={}", quickjs.display()))
        .build()
        .map_err(|e| e.to_string())?;
    let vm = JavaVM::with_libjvm(args, || Ok(library)).map_err(|e| e.to_string())?;
    vm.attach_current_thread(|env| {
        let class = env.find_class(jni_str!("io/legado/sourceengine/host/NativeCalls"))?;
        unsafe { env.register_native_methods(class, METHODS) }?;
        Ok::<_, jni::errors::Error>(())
    })
    .map_err(|e| e.to_string())?;
    VM.set(vm).map_err(|_| "JVM 重复初始化")?;
    Ok(VM.get().unwrap())
}
fn invoke(
    env: &mut Env<'_>,
    name: &str,
    input: &str,
    directory: &str,
    id: &str,
    request_type: &str,
    result_type: &str,
) -> jni::errors::Result<String> {
    let codec = env.find_class(jni_str!("io/legado/sourceengine/api/NativeCodec"))?;
    let engine = env.find_class(jni_str!("io/legado/sourceengine/api/Engine"))?;
    let input = env.new_string(input)?;
    let decode = RuntimeMethodSignature::from_str(format!(
        "(Ljava/lang/String;)Lio/legado/sourceengine/api/{request_type};"
    ))?;
    let object = env
        .call_static_method(
            &codec,
            JNIString::from(format!("decode{request_type}")),
            MethodSignature::from(&decode),
            &[JValue::Object(&input)],
        )?
        .l()?;
    let result_class = if result_type == "Book" {
        "io/legado/app/data/entities/Book".to_owned()
    } else {
        format!("io/legado/sourceengine/api/{result_type}")
    };
    let signature = RuntimeMethodSignature::from_str(format!(
        "(Lio/legado/sourceengine/api/{request_type};Ljava/lang/String;Ljava/lang/String;)L{result_class};"
    ))?;
    let directory = env.new_string(directory)?;
    let id = env.new_string(id)?;
    let result = env
        .call_static_method(
            engine,
            JNIString::from(name),
            MethodSignature::from(&signature),
            &[
                JValue::Object(&object),
                JValue::Object(&directory),
                JValue::Object(&id),
            ],
        )?
        .l()?;
    let signature =
        RuntimeMethodSignature::from_str(format!("(L{result_class};)Ljava/lang/String;"))?;
    let value = env
        .call_static_method(
            codec,
            JNIString::from(format!("encode{result_type}")),
            MethodSignature::from(&signature),
            &[JValue::Object(&result)],
        )?
        .l()?;
    // 返回描述符已限定为 String。
    unsafe { JString::from_raw(env, value.into_raw()) }.try_to_string(env)
}
fn error_text(env: &mut Env<'_>, error: jni::errors::Error) -> String {
    let Some(exception) = env.exception_occurred() else {
        return error.to_string();
    };
    env.exception_clear();
    env.call_method(
        exception,
        jni_str!("toString"),
        jni_sig!(() -> JString),
        &[],
    )
    .and_then(|v| v.l())
    .and_then(|v| unsafe { JString::from_raw(env, v.into_raw()) }.try_to_string(env))
    .unwrap_or_else(|_| {
        env.exception_clear();
        error.to_string()
    })
}
fn http<'a>(
    env: &mut Env<'a>,
    _: JClass<'a>,
    input: JString<'a>,
) -> jni::errors::Result<JString<'a>> {
    let input = input.try_to_string(env)?;
    env.new_string(crate::http::native(&input))
}
fn storage<'a>(
    env: &mut Env<'a>,
    _: JClass<'a>,
    input: JString<'a>,
) -> jni::errors::Result<JString<'a>> {
    let input = input.try_to_string(env)?;
    env.new_string(crate::storage::native(&input))
}
fn image<'a>(
    env: &mut Env<'a>,
    _: JClass<'a>,
    input: JString<'a>,
) -> jni::errors::Result<JString<'a>> {
    let input = input.try_to_string(env)?;
    env.new_string(crate::image::native(&input))
}
fn cancelled<'a>(
    env: &mut Env<'a>,
    _: JClass<'a>,
    id: JString<'a>,
) -> jni::errors::Result<JString<'a>> {
    let id = id.try_to_string(env)?;
    env.new_string(crate::engine::context::cancelled(&id).to_string())
}
fn browser<'a>(
    env: &mut Env<'a>,
    _: JClass<'a>,
    input: JString<'a>,
) -> jni::errors::Result<JString<'a>> {
    let input = input.try_to_string(env)?;
    env.new_string(crate::browser::native(&input))
}
