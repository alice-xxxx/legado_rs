//! Native method identifiers and the typed application-facing executor contract.
use crate::*;
use std::{future::Future, pin::Pin};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub(crate) enum Method {
    Search,
    ExploreKinds,
    Explore,
    Detail,
    Chapters,
    Content,
    ResolveMedia,
    HttpTtsAudio,
    Login,
}
impl Method {
    pub fn signature(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Search => ("search", "SearchRequest", "SearchPage"),
            Self::ExploreKinds => ("exploreKinds", "SourceRequest", "ExploreKinds"),
            Self::Explore => ("explore", "ExploreRequest", "SearchPage"),
            Self::Detail => ("detail", "BookRequest", "Book"),
            Self::Chapters => ("chapters", "BookRequest", "ChapterList"),
            Self::Content => ("content", "ContentRequest", "ChapterContent"),
            Self::ResolveMedia => ("resolveMedia", "MediaRequest", "MediaLocation"),
            Self::HttpTtsAudio => ("httpTtsAudio", "TtsRequest", "AudioData"),
            Self::Login => ("login", "LoginRequest", "LoginState"),
        }
    }
    pub fn is_search(self) -> bool {
        matches!(self, Self::Search)
    }
}

macro_rules! source_methods {
    () => {
        pub async fn search(
            &self,
            request: crate::SearchRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::SearchPage> {
            self.invoke(crate::api::Method::Search, request, context)
                .await
        }
        pub async fn explore_kinds(
            &self,
            request: crate::SourceRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::ExploreKinds> {
            self.invoke(crate::api::Method::ExploreKinds, request, context)
                .await
        }
        pub async fn explore(
            &self,
            request: crate::ExploreRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::SearchPage> {
            self.invoke(crate::api::Method::Explore, request, context)
                .await
        }
        pub async fn detail(
            &self,
            request: crate::BookRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::Book> {
            self.invoke(crate::api::Method::Detail, request, context)
                .await
        }
        pub async fn chapters(
            &self,
            request: crate::BookRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::ChapterList> {
            self.invoke(crate::api::Method::Chapters, request, context)
                .await
        }
        pub async fn content(
            &self,
            request: crate::ContentRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::ChapterContent> {
            self.invoke(crate::api::Method::Content, request, context)
                .await
        }
        pub async fn resolve_media(
            &self,
            request: crate::MediaRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::MediaLocation> {
            self.invoke(crate::api::Method::ResolveMedia, request, context)
                .await
        }
        pub async fn http_tts_audio(
            &self,
            request: crate::TtsRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::AudioData> {
            self.invoke(crate::api::Method::HttpTtsAudio, request, context)
                .await
        }
        pub async fn login(
            &self,
            request: crate::LoginRequest,
            context: crate::ExecutionContext,
        ) -> crate::Result<crate::LoginState> {
            self.invoke(crate::api::Method::Login, request, context)
                .await
        }
    };
}
pub(crate) use source_methods;

pub type EngineFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, String>> + Send + 'a>>;

pub trait SourceExecutor: Send + Sync {
    fn search<'a>(
        &'a self,
        request: SearchRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, SearchPage>;
    fn explore_kinds<'a>(
        &'a self,
        request: SourceRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, ExploreKinds>;
    fn explore<'a>(
        &'a self,
        request: ExploreRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, SearchPage>;
    fn detail<'a>(
        &'a self,
        request: BookRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, Book>;
    fn chapters<'a>(
        &'a self,
        request: BookRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, ChapterList>;
    fn content<'a>(
        &'a self,
        request: ContentRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, ChapterContent>;
    fn resolve_media<'a>(
        &'a self,
        request: MediaRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, MediaLocation>;
    fn http_tts_audio<'a>(
        &'a self,
        request: TtsRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, AudioData>;
    fn login<'a>(
        &'a self,
        request: LoginRequest,
        cancellation: CancellationToken,
    ) -> EngineFuture<'a, LoginState>;
}
