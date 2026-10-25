//! 引擎接口直接调用目标平台；上下文只随一次原生调用存活。
pub(crate) mod context;
use crate::*;
use serde::{Serialize, de::DeserializeOwned};
use tauri::{AppHandle, Manager, Runtime};

pub struct SourceEngine<R: Runtime> {
    #[cfg(mobile)]
    pub(crate) handle: tauri::plugin::PluginHandle<R>,
    pub(crate) runtime: std::marker::PhantomData<fn() -> R>,
}
pub struct Client<R: Runtime> {
    app: AppHandle<R>,
    context: EngineContext,
}
impl<R: Runtime> Client<R> {
    pub fn new(app: AppHandle<R>, context: EngineContext) -> Self {
        Self { app, context }
    }
}

impl<R: Runtime> SourceEngine<R> {
    async fn call<Q: Serialize + Send + 'static, T: DeserializeOwned + Send + 'static>(
        &self,
        name: &'static str,
        request: Q,
        context: EngineContext,
        _request_type: &'static str,
        _result_type: &'static str,
    ) -> Result<T> {
        let lease = context::Lease::enter(&context, name == "search")?;
        #[cfg(desktop)]
        return tokio::task::spawn_blocking(move || {
            crate::desktop::jvm::call(
                name,
                &request,
                &context,
                &lease.0,
                _request_type,
                _result_type,
            )
        })
        .await
        .map_err(|e| e.to_string())?;
        #[cfg(mobile)]
        {
            let handle = self.handle.clone();
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct NativeRequest<Q> {
                #[serde(flatten)]
                request: Q,
                data_directory: String,
                execution_id: String,
            }
            tokio::task::spawn_blocking(move || {
                let _lease = lease;
                let result = handle
                    .run_mobile_plugin(
                        name,
                        NativeRequest {
                            request,
                            data_directory: context.data_dir.to_string_lossy().into_owned(),
                            execution_id: _lease.0.clone(),
                        },
                    )
                    .map_err(|e| e.to_string());
                context::get(&_lease.0)?;
                result
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

macro_rules! source_api {
    ($($method:ident:$name:literal($request:ident)->$response:ident),+ $(,)?)=>{
        impl<R:Runtime> SourceEngine<R>{$(pub async fn $method(&self,request:$request,context:EngineContext)->Result<$response>{self.call($name,request,context,stringify!($request),stringify!($response)).await})+}
        #[cfg(desktop)] impl JvmEngine{$(pub async fn $method(&self,request:$request,context:EngineContext)->Result<$response>{self.call($name,request,context,stringify!($request),stringify!($response)).await})+}
        impl<R:Runtime> SourceExecutor for Client<R>{$(fn $method<'a>(&'a self,request:$request,cancellation:tokio_util::sync::CancellationToken)->EngineFuture<'a,$response>{
            let mut context=self.context.clone();context.cancellation=Some(cancellation);
            Box::pin(async move{self.app.state::<SourceEngine<R>>().$method(request,context).await})
        })+}
    }
}
source_api! {
    search:"search"(SearchRequest)->SearchPage, explore_kinds:"exploreKinds"(SourceRequest)->ExploreKinds,
    explore:"explore"(ExploreRequest)->SearchPage, detail:"detail"(BookRequest)->Book,
    chapters:"chapters"(BookRequest)->ChapterList, content:"content"(ContentRequest)->ChapterContent,
    resolve_media:"resolveMedia"(MediaRequest)->MediaLocation, http_tts_audio:"httpTtsAudio"(TtsRequest)->AudioData,
    login:"login"(LoginRequest)->LoginState,
}

#[cfg(desktop)]
pub struct JvmEngine;
#[cfg(desktop)]
impl JvmEngine {
    async fn call<Q: Serialize + Send + 'static, T: DeserializeOwned + Send + 'static>(
        &self,
        name: &'static str,
        request: Q,
        context: EngineContext,
        request_type: &'static str,
        result_type: &'static str,
    ) -> Result<T> {
        let lease = context::Lease::enter(&context, name == "search")?;
        tokio::task::spawn_blocking(move || {
            crate::desktop::jvm::call(
                name,
                &request,
                &context,
                &lease.0,
                request_type,
                result_type,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
}
