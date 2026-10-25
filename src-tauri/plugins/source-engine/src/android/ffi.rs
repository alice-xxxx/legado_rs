//! Android JNI 参数转换；每个入口只调用所属能力模块。
use jni::{
    EnvUnowned,
    errors::ThrowRuntimeExAndDefault,
    jni_mangle,
    objects::{JObject, JString},
};
macro_rules! endpoint {
    ($name:ident,$function:path) => {
        #[jni_mangle("io.legado.sourceengine.host.NativeCalls")]
        pub fn $name<'a>(
            mut env: EnvUnowned<'a>,
            _: JObject<'a>,
            input: JString<'a>,
        ) -> JString<'a> {
            env.with_env(|env| {
                let input = input.try_to_string(env)?;
                env.new_string($function(&input))
            })
            .resolve::<ThrowRuntimeExAndDefault>()
        }
    };
}
endpoint!(http, crate::http::native);
endpoint!(storage, crate::storage::native);
endpoint!(image, crate::image::native);
endpoint!(browser, crate::browser::native);
#[jni_mangle("io.legado.sourceengine.host.NativeCalls")]
pub fn cancelled<'a>(mut env: EnvUnowned<'a>, _: JObject<'a>, id: JString<'a>) -> JString<'a> {
    env.with_env(|env| {
        let id = id.try_to_string(env)?;
        env.new_string(crate::engine::context::cancelled(&id).to_string())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}
