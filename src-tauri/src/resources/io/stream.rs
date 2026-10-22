//! 共用 HTTP 流响应；普通字节流和可定位读流使用同一套发送逻辑。
use axum::{
    body::{Body, Bytes},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{Stream, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, SeekFrom};
use tokio::sync::watch;
use tokio_util::io::ReaderStream;

pub(super) fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// 不知道最终长度的读流，只发布已写完的数据；结束和取消都会唤醒等待者。
#[derive(Clone, Copy, Default)]
pub(super) struct Progress {
    pub length: u64,
    pub finished: bool,
    pub cancelled: bool,
}

pub(super) fn growing<R>(
    reader: R,
    progress: watch::Receiver<Progress>,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send
where
    R: AsyncRead + Unpin + Send,
{
    futures_util::stream::try_unfold(
        (reader, progress, 0),
        |(mut reader, mut progress, mut offset)| async move {
            loop {
                let state = *progress.borrow_and_update();
                if state.cancelled {
                    return Ok(None);
                }
                if offset < state.length {
                    let mut bytes = vec![0; (state.length - offset).min(64 * 1024) as usize];
                    reader.read_exact(&mut bytes).await?;
                    offset += bytes.len() as u64;
                    return Ok(Some((Bytes::from(bytes), (reader, progress, offset))));
                }
                if state.finished || progress.changed().await.is_err() {
                    return Ok(None);
                }
            }
        },
    )
}

/// HEAD 不读取流；响应持有调用方资源，发送结束或断开后释放。
pub(super) fn body<S, E, T>(method: &Method, stream: S, keep_alive: T) -> Body
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
    T: Send + 'static,
{
    if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from_stream(stream.map(move |chunk| {
            let _keep_alive = &keep_alive;
            chunk
        }))
    }
}

/// 已知长度且可定位的读流，统一处理单区间 Range 和响应头。
pub(super) async fn seekable<R, T>(
    mut reader: R,
    size: u64,
    content_type: &str,
    method: &Method,
    headers: &HeaderMap,
    keep_alive: T,
) -> Result<Response, StatusCode>
where
    R: AsyncRead + AsyncSeek + Unpin + Send + 'static,
    T: Send + 'static,
{
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT_RANGES, "bytes");
    let (start, length) = if let Some(value) = headers.get(header::RANGE) {
        let ranges = value
            .to_str()
            .ok()
            .and_then(|value| http_range_header::parse_range_header(value).ok())
            .and_then(|ranges| match ranges.validate(size) {
                Ok(ranges) => Some(ranges),
                // 尾部范围超过长度时，返回整个读流的有效范围。
                Err(http_range_header::RangeUnsatisfiableError::FileSuffixOutOfBounds) => {
                    Some(vec![0..=size.saturating_sub(1)])
                }
                Err(_) => None,
            });
        let Some(range) = ranges
            .as_deref()
            .filter(|ranges| ranges.len() == 1 && size > 0)
            .and_then(|ranges| ranges.first())
        else {
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{size}"))],
            )
                .into_response());
        };
        let (start, end) = (*range.start(), *range.end());
        response = response
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
        (start, end - start + 1)
    } else {
        (0, size)
    };
    reader
        .seek(SeekFrom::Start(start))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let body = body(method, ReaderStream::new(reader.take(length)), keep_alive);
    response
        .header(header::CONTENT_LENGTH, length)
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
