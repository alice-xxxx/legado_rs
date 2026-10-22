//! 本地 HTTP 启动、访问控制和分发入口；业务处理留给子模块。
pub(super) mod cache;
mod file;
pub(super) mod proxy;
pub(super) mod settings;
mod stream;
#[cfg(test)]
mod tests;
pub(super) mod tts;
use axum::{
    extract::Request,
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use std::io;
use std::{net::SocketAddr, task::Poll};
use tokio::{
    net::{TcpListener, TcpSocket, TcpStream},
    sync::watch,
    task::JoinHandle,
};

pub(super) struct LocalHttp {
    pub base_url: String,
    task: JoinHandle<()>,
    address: SocketAddr,
    listener: watch::Sender<Option<TcpListener>>,
    runtime: tokio::runtime::Handle,
}

struct Listener {
    socket: watch::Receiver<Option<TcpListener>>,
    address: SocketAddr,
}
impl axum::serve::Listener for Listener {
    type Io = TcpStream;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (TcpStream, SocketAddr) {
        loop {
            let receiver = self.socket.clone();
            let accepting = std::future::poll_fn(|cx| {
                receiver
                    .borrow()
                    .as_ref()
                    .map_or(Poll::Pending, |listener| listener.poll_accept(cx))
            });
            tokio::select! {
                result = accepting => match result {
                    Ok(connection) => return connection,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                },
                _ = self.socket.changed() => {}
            }
        }
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.address)
    }
}

impl LocalHttp {
    /// 前台恢复时只替换监听套接字，端口、令牌和正在播放的连接不变。
    pub(super) fn resume(&self) -> io::Result<()> {
        let _runtime = self.runtime.enter();
        let mut result = Ok(());
        self.listener.send_modify(|listener| {
            *listener = None;
            result = (|| {
                let socket = TcpSocket::new_v4()?;
                #[cfg(unix)]
                socket.set_reuseaddr(true)?;
                socket.bind(self.address)?;
                *listener = Some(socket.listen(1024)?);
                Ok(())
            })();
        });
        result
    }
}

/// 各模块负责处理请求，这里只注册入口并统一访问控制。
pub(super) async fn start(
    cache: cache::Cache,
    settings: settings::Settings,
    proxy: proxy::Proxy,
    tts: tts::Tts,
) -> io::Result<LocalHttp> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let prefix = format!("/{}", uuid::Uuid::new_v4().simple());
    let base_url = format!("http://{}{prefix}/", listener.local_addr()?);
    let routes = Router::new()
        .nest("/cache", cache::routes(cache, base_url.clone()))
        .merge(settings.routes())
        .nest("/proxy", proxy.routes())
        .nest("/tts", tts.routes());
    let router = Router::new().nest(&prefix, routes.layer(middleware::from_fn(access)));
    let (listener, receiver) = watch::channel(Some(listener));
    let task = tokio::spawn(async move {
        if let Err(error) = axum::serve(
            Listener {
                socket: receiver,
                address,
            },
            router,
        )
        .await
        {
            eprintln!("本地 HTTP 服务失败：{error}");
        }
    });
    Ok(LocalHttp {
        base_url,
        task,
        address,
        listener,
        runtime: tokio::runtime::Handle::current(),
    })
}

/// 令牌由路由前缀检查；HTTP 仅开放读取和跨域预检。
async fn access(request: Request, next: Next) -> Response {
    let mut response = match *request.method() {
        Method::GET | Method::HEAD => next.run(request).await,
        Method::OPTIONS => StatusCode::NO_CONTENT.into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    };
    let headers = response.headers_mut();
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|mime| mime.starts_with("text/html") || mime == "image/svg+xml")
    {
        headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'none'; img-src 'self' data: http: https:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; media-src 'self' http: https:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"));
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Range, Content-Type"),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Accept-Ranges, Content-Length, Content-Range, Content-Type"),
    );
    response
}

impl Drop for LocalHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}
