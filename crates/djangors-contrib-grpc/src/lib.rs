//! gRPC integration (`tonic`) for Djangors.
//!
//! This crate bridges [`djangors_core`] and [`tonic`] by:
//! - Sharing [`djangors_core::AppState`] with gRPC services via [`AppStateInterceptor`]
//!   and [`extract_app_state`].
//! - Serving HTTP ([`djangors_core::Router`]) and gRPC ([`tonic::service::Routes`])
//!   together via [`HybridServer`], either on separate ports or multiplexed over a
//!   single listener.

#![deny(missing_docs)]

use std::convert::Infallible;
use std::net::SocketAddr;

use djangors_core::{AppState, Router};
use http_body_util::BodyExt as _;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt as _;

pub use tonic;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Errors returned by [`HybridServer`] and gRPC server operations.
#[derive(Debug, thiserror::Error)]
pub enum GrpcError {
    /// An underlying I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to bind a TCP listener to the requested socket address.
    #[error("failed to bind to {addr}: {source}")]
    Bind {
        /// The socket address that could not be bound.
        addr: SocketAddr,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The HTTP or gRPC server exited with an error.
    #[error("server error: {0}")]
    Server(String),
}

/// Returns `true` if the `content-type` header starts with `"application/grpc"` (case-insensitive).
pub fn is_grpc_request(headers: &hyper::HeaderMap) -> bool {
    headers
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ct| {
            ct.trim_start()
                .as_bytes()
                .get(..16)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"application/grpc"))
        })
}

/// A [`tonic::service::Interceptor`] that inserts a [`djangors_core::AppState`]
/// into every incoming gRPC request's extensions.
#[derive(Clone, Debug)]
pub struct AppStateInterceptor {
    state: AppState,
}

impl AppStateInterceptor {
    /// Create a new interceptor carrying the given [`AppState`].
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Return a reference to the underlying [`AppState`].
    pub fn state(&self) -> &AppState {
        &self.state
    }
}

impl tonic::service::Interceptor for AppStateInterceptor {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        req.extensions_mut().insert(self.state.clone());
        Ok(req)
    }
}

/// Retrieve the [`AppState`] attached to a gRPC request by [`AppStateInterceptor`].
pub fn extract_app_state<T>(req: &tonic::Request<T>) -> Option<&AppState> {
    req.extensions().get::<AppState>()
}

/// A server that runs a Djangors HTTP [`Router`] alongside [`tonic::service::Routes`],
/// either on separate ports or multiplexed on a single port.
#[derive(Clone)]
pub struct HybridServer {
    http_router: Router,
    grpc_routes: tonic::service::Routes,
}

impl HybridServer {
    /// Create a new [`HybridServer`] from an HTTP [`Router`] and gRPC [`tonic::service::Routes`].
    pub fn new(http_router: Router, grpc_routes: tonic::service::Routes) -> Self {
        Self {
            http_router,
            grpc_routes,
        }
    }

    /// Serve HTTP on `http_addr` and gRPC on `grpc_addr` concurrently.
    pub async fn serve_dual(
        self,
        http_addr: SocketAddr,
        grpc_addr: SocketAddr,
    ) -> Result<(), GrpcError> {
        let http_listener = TcpListener::bind(http_addr)
            .await
            .map_err(|source| GrpcError::Bind {
                addr: http_addr,
                source,
            })?;
        let grpc_listener = TcpListener::bind(grpc_addr)
            .await
            .map_err(|source| GrpcError::Bind {
                addr: grpc_addr,
                source,
            })?;

        let http_router = self.http_router;
        let grpc_routes = self.grpc_routes;

        let http_fut = accept_loop(http_listener, move |stream| {
            let router = http_router.clone();
            async move { serve_http_connection(stream, router).await }
        });
        let grpc_fut = accept_loop(grpc_listener, move |stream| {
            let routes = grpc_routes.clone();
            async move { serve_grpc_connection(stream, routes).await }
        });

        tokio::try_join!(http_fut, grpc_fut)?;
        Ok(())
    }

    /// Serve both HTTP and gRPC multiplexed over an already-bound [`TcpListener`].
    ///
    /// Requests whose `content-type` header passes [`is_grpc_request`] are routed
    /// to `grpc_routes`; all other requests are dispatched to `http_router`.
    pub async fn serve_multiplexed_listener(self, listener: TcpListener) -> Result<(), GrpcError> {
        let http_router = self.http_router;
        let grpc_routes = self.grpc_routes;

        accept_loop(listener, move |stream| {
            let router = http_router.clone();
            let routes = grpc_routes.clone();
            async move { serve_multiplexed_connection(stream, router, routes).await }
        })
        .await
    }

    /// Bind `addr` and serve both HTTP and gRPC multiplexed on a single port.
    pub async fn serve_multiplexed(self, addr: SocketAddr) -> Result<(), GrpcError> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|source| GrpcError::Bind { addr, source })?;
        self.serve_multiplexed_listener(listener).await
    }
}

async fn accept_loop<H, Fut>(listener: TcpListener, handler: H) -> Result<(), GrpcError>
where
    H: Fn(TcpStream) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    loop {
        let (stream, _peer) = listener.accept().await?;
        let handler = handler.clone();
        tokio::spawn(async move {
            handler(stream).await;
        });
    }
}

async fn serve_http_connection(stream: TcpStream, router: Router) {
    let io = TokioIo::new(stream);
    let svc = service_fn(move |req: hyper::Request<Incoming>| {
        let router = router.clone();
        async move { Ok::<_, Infallible>(router.dispatch_boxed(req, false).await) }
    });

    let _ = AutoBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(io, svc)
        .await;
}

async fn serve_grpc_connection(stream: TcpStream, routes: tonic::service::Routes) {
    let io = TokioIo::new(stream);
    let svc = service_fn(move |req: hyper::Request<Incoming>| {
        let routes = routes.clone();
        async move { dispatch_grpc(routes, req).await }
    });

    let _ = AutoBuilder::new(TokioExecutor::new())
        .serve_connection(io, svc)
        .await;
}

async fn serve_multiplexed_connection(
    stream: TcpStream,
    router: Router,
    routes: tonic::service::Routes,
) {
    let io = TokioIo::new(stream);
    let svc = service_fn(move |req: hyper::Request<Incoming>| {
        let router = router.clone();
        let routes = routes.clone();
        async move {
            if is_grpc_request(req.headers()) {
                let resp = dispatch_grpc(routes, req).await?;
                Ok::<_, Infallible>(resp.map(|b| b.map_err(|e| -> BoxError { Box::new(e) }).boxed_unsync()))
            } else {
                let resp = router.dispatch_boxed(req, false).await;
                Ok::<_, Infallible>(resp.map(|b| b.map_err(|e| -> BoxError { match e {} }).boxed_unsync()))
            }
        }
    });

    let _ = AutoBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(io, svc)
        .await;
}

async fn dispatch_grpc(
    routes: tonic::service::Routes,
    req: hyper::Request<Incoming>,
) -> Result<hyper::Response<tonic::body::Body>, Infallible> {
    let tonic_req = req.map(tonic::body::Body::new);
    routes.oneshot(tonic_req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::task::{Context, Poll};

    use bytes::Bytes;
    use djangors_core::{Response, Router};
    use http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
    use http_body_util::Empty;
    use tonic::service::Interceptor as _;
    use tower::Service;

    #[test]
    fn is_grpc_request_matches_positive_and_negative_cases() {
        let mut headers = HeaderMap::new();
        assert!(!is_grpc_request(&headers));

        headers.insert(
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("application/grpc"),
        );
        assert!(is_grpc_request(&headers));

        headers.insert(
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("application/grpc+proto"),
        );
        assert!(is_grpc_request(&headers));

        headers.insert(
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("APPLICATION/GRPC; charset=utf-8"),
        );
        assert!(is_grpc_request(&headers));

        headers.insert(
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        assert!(!is_grpc_request(&headers));

        headers.insert(
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain"),
        );
        assert!(!is_grpc_request(&headers));
    }

    #[test]
    fn app_state_interceptor_injects_and_extracts_state() {
        #[derive(Clone, Debug, PartialEq)]
        struct Config {
            app_name: &'static str,
        }

        let state = AppState::new().insert(Config {
            app_name: "djangors-grpc",
        });
        let mut interceptor = AppStateInterceptor::new(state);
        assert!(interceptor.state().contains::<Config>());

        let req = tonic::Request::new(());
        let intercepted = interceptor.call(req).expect("interceptor should succeed");

        let extracted = extract_app_state(&intercepted).expect("AppState should be present");
        let cfg = extracted
            .get::<Config>()
            .expect("Config should be in AppState");
        assert_eq!(cfg.app_name, "djangors-grpc");
    }

    #[derive(Clone)]
    struct EchoGrpcService;

    impl tonic::server::NamedService for EchoGrpcService {
        const NAME: &'static str = "test.Echo";
    }

    impl Service<http::Request<tonic::body::Body>> for EchoGrpcService {
        type Response = http::Response<tonic::body::Body>;
        type Error = Infallible;
        type Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
        >;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: http::Request<tonic::body::Body>) -> Self::Future {
            Box::pin(async {
                let resp = http::Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/grpc")
                    .header("grpc-status", "0")
                    .body(tonic::body::Body::empty())
                    .unwrap();
                Ok(resp)
            })
        }
    }

    #[tokio::test]
    async fn serve_multiplexed_listener_routes_http_and_grpc_over_loopback() {
        let router = Router::new()
            .get("/health", |_req, _params| async {
                Ok(Response::text(StatusCode::OK, "ok"))
            });

        let grpc_routes = tonic::service::Routes::new(EchoGrpcService);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr");

        let server = HybridServer::new(router, grpc_routes);
        let server_task = tokio::spawn(async move {
            let _ = server.serve_multiplexed_listener(listener).await;
        });

        // 1. HTTP/1.1 GET /health -> 200 OK
        let http_stream = TcpStream::connect(addr).await.expect("connect http");
        let (mut http_sender, http_conn) =
            hyper::client::conn::http1::handshake(TokioIo::new(http_stream))
                .await
                .expect("http1 handshake");
        tokio::spawn(async move {
            let _ = http_conn.await;
        });

        let http_req = Request::builder()
            .method(Method::GET)
            .uri("/health")
            .header("host", addr.to_string())
            .body(Empty::<Bytes>::new())
            .unwrap();
        let http_resp = http_sender
            .send_request(http_req)
            .await
            .expect("send http request");
        assert_eq!(http_resp.status(), StatusCode::OK);
        let body = http_resp
            .into_body()
            .collect()
            .await
            .expect("collect http body")
            .to_bytes();
        assert_eq!(&body[..], b"ok");

        // 2. HTTP/2 POST /test.Echo/Ping with content-type: application/grpc -> routed to gRPC service
        let grpc_stream = TcpStream::connect(addr).await.expect("connect grpc");
        let (mut h2_sender, h2_conn) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(grpc_stream))
                .await
                .expect("http2 handshake");
        tokio::spawn(async move {
            let _ = h2_conn.await;
        });

        let grpc_req = Request::builder()
            .method(Method::POST)
            .uri(format!("http://{addr}/test.Echo/Ping"))
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let grpc_resp = h2_sender
            .send_request(grpc_req)
            .await
            .expect("send grpc request");
        assert_eq!(grpc_resp.status(), StatusCode::OK);
        assert_eq!(
            grpc_resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/grpc")
        );
        assert_eq!(
            grpc_resp
                .headers()
                .get("grpc-status")
                .and_then(|v| v.to_str().ok()),
            Some("0")
        );

        server_task.abort();
    }
}
