//! Android 插件注册与进程环境初始化。
mod ffi;
use crate::*;
pub(crate) fn init<R: tauri::Runtime, C: serde::de::DeserializeOwned>(
    _: &tauri::AppHandle<R>,
    api: tauri::plugin::PluginApi<R, C>,
) -> Result<SourceEngine<R>> {
    let android = ndk_context::android_context();
    // Tao 持有进程 JVM 和全局 Context；这里只建立当前帧的局部引用。
    let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) };
    vm.attach_current_thread_for_scope(|env| {
        use jni::refs::Reference;
        let borrowed = unsafe { jni::objects::JObject::kind_from_raw(android.context().cast()) };
        let context = env.new_local_ref(&borrowed)?;
        rustls_platform_verifier::android::init_with_env(env, context)
    })
    .map_err(|e| e.to_string())?;
    let handle = api
        .register_android_plugin("io.foliver.sourceengine", "SourceEnginePlugin")
        .map_err(|e| e.to_string())?;
    crate::mobile::install(handle.clone());
    Ok(SourceEngine {
        handle,
        runtime: std::marker::PhantomData,
    })
}
