//! Idempotency middleware for retry-safe unsafe HTTP requests (`Idempotency-Key`).

use crate::Cache;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, COOKIE, LOCATION};
use hyper::http::request::Parts;
use hyper::{Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tower::{Layer, Service};

const IN_PROGRESS_PREFIX: &[u8] = b"in_progress:";

/// The default request body limit: 10 MiB, matching the router's default.
pub const DEFAULT_MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Derives the caller identity that idempotency keys are scoped to.
type ScopeFn = dyn Fn(&Parts) -> Vec<u8> + Send + Sync;

/// Tower layer that runs an unsafe request (`POST`, `PUT`, `PATCH`, `DELETE`) carrying an
/// `Idempotency-Key` header at most once, and replays the stored response to retries.
///
/// Keys are scoped to the method, path, and caller. By default the caller is identified by
/// the `Authorization` and `Cookie` headers, so two users choosing the same key never see
/// each other's responses; use [`IdempotencyLayer::with_scope`] to scope by something
/// more precise, such as a session or user ID.
///
/// The layer reads the whole request body to hash it, so it enforces its own size limit
/// ([`DEFAULT_MAX_BODY_BYTES`] unless changed with [`IdempotencyLayer::max_body_bytes`])
/// and answers larger bodies with `413 Payload Too Large`.
#[derive(Clone)]
pub struct IdempotencyLayer {
    cache: Arc<dyn Cache>,
    ttl: Duration,
    scope: Arc<ScopeFn>,
    max_body_bytes: usize,
}

impl IdempotencyLayer {
    /// Creates a layer storing records in `cache` for `ttl`. The cache must support
    /// [`Cache::add`] (all built-in backends do).
    pub fn new(cache: Arc<dyn Cache>, ttl: Duration) -> Self {
        Self {
            cache,
            ttl,
            scope: Arc::new(default_scope),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// Sets the largest request body, in bytes, the layer will read. Larger bodies get
    /// `413 Payload Too Large` before the handler runs.
    pub fn max_body_bytes(mut self, limit: usize) -> Self {
        self.max_body_bytes = limit;
        self
    }

    /// Replaces the default caller identity (`Authorization` + `Cookie` headers).
    pub fn with_scope<F>(mut self, scope: F) -> Self
    where
        F: Fn(&Parts) -> Vec<u8> + Send + Sync + 'static,
    {
        self.scope = Arc::new(scope);
        self
    }
}

fn default_scope(parts: &Parts) -> Vec<u8> {
    let mut out = Vec::new();
    for name in [AUTHORIZATION, COOKIE] {
        let value = parts.headers.get(name).map(HeaderValue::as_bytes);
        push_component(&mut out, value.unwrap_or_default());
    }
    out
}

impl<S> Layer<S> for IdempotencyLayer {
    type Service = IdempotencyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        IdempotencyService {
            inner,
            cache: self.cache.clone(),
            ttl: self.ttl,
            scope: self.scope.clone(),
            max_body_bytes: self.max_body_bytes,
        }
    }
}

/// Tower service created by [`IdempotencyLayer`].
#[derive(Clone)]
pub struct IdempotencyService<S> {
    inner: S,
    cache: Arc<dyn Cache>,
    ttl: Duration,
    scope: Arc<ScopeFn>,
    max_body_bytes: usize,
}

#[derive(Serialize, Deserialize)]
struct Record {
    body_hash: String,
    status: u16,
    content_type: Option<String>,
    location: Option<String>,
    body: Vec<u8>,
}

/// Releases the in-progress marker if the request future is dropped before it finishes
/// (for example when the client disconnects), so an honest retry is not blocked until
/// the record expires.
struct InProgressGuard {
    cache: Arc<dyn Cache>,
    key: Option<String>,
}

impl InProgressGuard {
    fn finish(mut self) {
        self.key = None;
    }
}

impl Drop for InProgressGuard {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            let cache = self.cache.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = cache.delete(&key).await;
                });
            }
        }
    }
}

impl<S, B> Service<Request<B>> for IdempotencyService<S>
where
    S: Service<Request<Full<Bytes>>, Response = Response<Full<Bytes>>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    B: http_body::Body<Data = Bytes> + Send + 'static,
    B::Error: std::fmt::Debug,
{
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        // Take the service that was driven to readiness and leave a clone in its place.
        let clone = self.inner.clone();
        let inner = std::mem::replace(&mut self.inner, clone);
        let cache = self.cache.clone();
        let ttl = self.ttl;
        let scope = self.scope.clone();
        let limit = self.max_body_bytes;
        Box::pin(handle(inner, cache, ttl, scope, limit, req))
    }
}

async fn handle<S, B>(
    mut inner: S,
    cache: Arc<dyn Cache>,
    ttl: Duration,
    scope: Arc<ScopeFn>,
    limit: usize,
    req: Request<B>,
) -> Result<Response<Full<Bytes>>, Infallible>
where
    S: Service<Request<Full<Bytes>>, Response = Response<Full<Bytes>>, Error = Infallible>,
    B: http_body::Body<Data = Bytes>,
    B::Error: std::fmt::Debug,
{
    let (parts, body) = req.into_parts();
    let body = match read_limited(&parts, body, limit).await {
        Ok(body) => body,
        Err(response) => return Ok(response),
    };

    let unsafe_method = matches!(
        parts.method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    let key = match parts.headers.get("idempotency-key") {
        Some(key) if unsafe_method => key.as_bytes().to_vec(),
        _ => {
            return inner
                .call(Request::from_parts(parts, Full::new(body)))
                .await
        }
    };
    if key.is_empty() || key.len() > 255 || !key.iter().all(|b| (0x21..=0x7e).contains(b)) {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_idempotency_key",
        ));
    }

    let body_hash = hex_sha256(&body);
    let mut identity = Vec::new();
    push_component(&mut identity, parts.method.as_str().as_bytes());
    push_component(&mut identity, parts.uri.path().as_bytes());
    push_component(&mut identity, &key);
    push_component(&mut identity, &scope(&parts));
    let storage_key = format!("idem:{}", hex_sha256(&identity));

    let mut marker = IN_PROGRESS_PREFIX.to_vec();
    marker.extend_from_slice(body_hash.as_bytes());
    match cache.add(&storage_key, marker, Some(ttl)).await {
        Err(_) => Ok(store_unavailable()),
        Ok(true) => {
            let guard = InProgressGuard {
                cache: cache.clone(),
                key: Some(storage_key.clone()),
            };
            let response = inner
                .call(Request::from_parts(parts, Full::new(body)))
                .await?;
            if response.status().is_server_error() {
                // Let the client retry: the operation may not have happened. Release the
                // key before answering so an immediate retry is not told 409.
                guard.finish();
                let _ = cache.delete(&storage_key).await;
                return Ok(response);
            }
            let (head, response_body) = response.into_parts();
            let response_body = match response_body.collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(never) => match never {},
            };
            let record = Record {
                body_hash,
                status: head.status.as_u16(),
                content_type: header_string(&head.headers, CONTENT_TYPE),
                location: header_string(&head.headers, LOCATION),
                body: response_body.to_vec(),
            };
            let stored = match serde_json::to_vec(&record) {
                Ok(bytes) => cache.set(&storage_key, bytes, Some(ttl)).await.is_ok(),
                Err(_) => false,
            };
            if !stored {
                // The handler already ran; keep the in-progress marker (it expires with
                // `ttl`) so a retry gets 409 instead of running the operation twice.
                guard.finish();
                tracing::error!(
                    key = %storage_key,
                    "idempotency record could not be stored; retries will get 409 until it expires"
                );
                return Ok(Response::from_parts(head, Full::new(response_body)));
            }
            guard.finish();
            Ok(Response::from_parts(head, Full::new(response_body)))
        }
        Ok(false) => match cache.get(&storage_key).await {
            Err(_) => Ok(store_unavailable()),
            Ok(None) => Ok(in_progress()),
            Ok(Some(bytes)) if bytes.starts_with(IN_PROGRESS_PREFIX) => {
                if bytes[IN_PROGRESS_PREFIX.len()..] != *body_hash.as_bytes() {
                    Ok(key_reused())
                } else {
                    Ok(in_progress())
                }
            }
            Ok(Some(bytes)) => match serde_json::from_slice::<Record>(&bytes) {
                Ok(record) if record.body_hash != body_hash => Ok(key_reused()),
                Ok(record) => Ok(replay(record)),
                Err(_) => Ok(store_unavailable()),
            },
        },
    }
}

fn replay(record: Record) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(record.body)));
    *response.status_mut() =
        StatusCode::from_u16(record.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let headers = response.headers_mut();
    for (name, value) in [
        (CONTENT_TYPE, record.content_type),
        (LOCATION, record.location),
    ] {
        if let Some(value) = value.and_then(|v| HeaderValue::from_str(&v).ok()) {
            headers.insert(name, value);
        }
    }
    headers.insert("idempotent-replayed", HeaderValue::from_static("true"));
    response
}

fn header_string(headers: &hyper::HeaderMap, name: hyper::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn push_component(out: &mut Vec<u8>, component: &[u8]) {
    out.extend_from_slice(&(component.len() as u64).to_be_bytes());
    out.extend_from_slice(component);
}

fn in_progress() -> Response<Full<Bytes>> {
    error_response(StatusCode::CONFLICT, "idempotency_in_progress")
}

fn key_reused() -> Response<Full<Bytes>> {
    error_response(StatusCode::UNPROCESSABLE_ENTITY, "idempotency_key_reused")
}

fn store_unavailable() -> Response<Full<Bytes>> {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "idempotency_store_unavailable",
    )
}

/// Reads the body, refusing it with 413 as soon as it is known to exceed `limit`
/// (from `Content-Length`, or while streaming) so oversized bodies are never buffered.
async fn read_limited<B>(
    parts: &Parts,
    body: B,
    limit: usize,
) -> Result<Bytes, Response<Full<Bytes>>>
where
    B: http_body::Body<Data = Bytes>,
{
    let too_large = || error_response(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large");
    let declared = parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > limit as u64) {
        return Err(too_large());
    }
    let mut body = std::pin::pin!(body);
    let mut buf = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame =
            frame.map_err(|_| error_response(StatusCode::BAD_REQUEST, "invalid_request_body"))?;
        if let Ok(data) = frame.into_data() {
            if buf.len() + data.len() > limit {
                return Err(too_large());
            }
            buf.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(buf))
}

fn error_response(status: StatusCode, code: &str) -> Response<Full<Bytes>> {
    let body = serde_json::json!({ "error": { "status": status.as_u16(), "code": code } });
    let mut response = Response::new(Full::new(Bytes::from(body.to_string())));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
