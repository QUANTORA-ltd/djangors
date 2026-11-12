//! Idempotency middleware for retry-safe unsafe HTTP requests.

use crate::Cache;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{header::{AUTHORIZATION, CONTENT_TYPE}, HeaderValue, Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc, task::{Context, Poll}, time::Duration};
use tower::{Layer, Service};

/// Tower layer that replays completed responses for duplicate unsafe requests.
#[derive(Clone)]
pub struct IdempotencyLayer { cache: Arc<dyn Cache>, ttl: Duration }

impl IdempotencyLayer {
    /// Creates an idempotency layer using the supplied cache and record lifetime.
    pub fn new(cache: Arc<dyn Cache>, ttl: Duration) -> Self { Self { cache, ttl } }
}

impl<S> Layer<S> for IdempotencyLayer {
    type Service = IdempotencyService<S>;
    fn layer(&self, inner: S) -> Self::Service { IdempotencyService { inner, cache: self.cache.clone(), ttl: self.ttl } }
}

/// Tower service created by [`IdempotencyLayer`].
#[derive(Clone)]
pub struct IdempotencyService<S> { inner: S, cache: Arc<dyn Cache>, ttl: Duration }

#[derive(Serialize, Deserialize)]
struct Record { body_hash: String, status: u16, content_type: Option<String>, location: Option<String>, body: Vec<u8> }

impl<S, B> Service<Request<B>> for IdempotencyService<S>
where
    S: Service<Request<Full<Bytes>>, Response = Response<Full<Bytes>>, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: http_body::Body<Data = Bytes> + Send + 'static,
    B::Error: Send + Sync + std::fmt::Debug + 'static,
{
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> { self.inner.poll_ready(cx) }
    fn call(&mut self, req: Request<B>) -> Self::Future {
        let mut inner = self.inner.clone(); let cache = self.cache.clone(); let ttl = self.ttl;
        Box::pin(async move {
            let method = req.method().clone();
            let Some(key) = req.headers().get("idempotency-key").cloned() else { return match buffer_request(req).await { Ok(req) => inner.call(req).await, Err(()) => Ok(error_response(StatusCode::BAD_REQUEST, "invalid_request_body")) }; };
            if !matches!(method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE) { return match buffer_request(req).await { Ok(req) => inner.call(req).await, Err(()) => Ok(error_response(StatusCode::BAD_REQUEST, "invalid_request_body")) }; }
            let key = key.to_str().unwrap_or("");
            if key.is_empty() || key.len() > 255 || !key.bytes().all(|b| (0x21..=0x7e).contains(&b)) { return Ok(error_response(StatusCode::BAD_REQUEST, "invalid_idempotency_key")); }
            let (parts, body) = req.into_parts();
            let body = match body.collect().await { Ok(b) => b.to_bytes(), Err(_) => return Ok(error_response(StatusCode::BAD_REQUEST, "invalid_request_body")) };
            let body_hash = hex_hash(&body);
            let auth = parts.headers.get(AUTHORIZATION).map(|h| h.as_bytes()).unwrap_or_default();
            let mut identity = Vec::new();
            for component in [method.as_str().as_bytes(), parts.uri.path().as_bytes(), key.as_bytes(), auth] { identity.extend_from_slice(&(component.len() as u64).to_be_bytes()); identity.extend_from_slice(component); }
            let storage_key = format!("idem:{}", hex_hash(&identity));
            let in_progress = format!("in_progress:{body_hash}").into_bytes();
            match cache.add(&storage_key, in_progress, Some(ttl)).await {
                Err(_) => Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, "idempotency_store_unavailable")),
                Ok(true) => {
                    let request = Request::from_parts(parts, Full::new(body.clone()));
                    let response = inner.call(request).await?;
                    if response.status().is_server_error() { let _ = cache.delete(&storage_key).await; return Ok(response); }
                    let record = Record { body_hash, status: response.status().as_u16(), content_type: response.headers().get(CONTENT_TYPE).and_then(|v|v.to_str().ok()).map(str::to_owned), location: response.headers().get(hyper::header::LOCATION).and_then(|v|v.to_str().ok()).map(str::to_owned), body: response.body().clone().into_inner().unwrap_or_default().to_vec() };
                    match serde_json::to_vec(&record) { Ok(bytes) if cache.set(&storage_key, bytes, Some(ttl)).await.is_ok() => Ok(response), _ => { let _ = cache.delete(&storage_key).await; Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, "idempotency_store_unavailable")) } }
                }
                Ok(false) => match cache.get(&storage_key).await {
                    Err(_) => Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, "idempotency_store_unavailable")),
                    Ok(None) => Ok(error_response(StatusCode::CONFLICT, "idempotency_in_progress")),
                    Ok(Some(bytes)) if bytes.starts_with(b"in_progress:") => Ok(error_response(StatusCode::CONFLICT, "idempotency_in_progress")),
                    Ok(Some(bytes)) => match serde_json::from_slice::<Record>(&bytes) {
                        Ok(record) if record.body_hash != body_hash => Ok(error_response(StatusCode::UNPROCESSABLE_ENTITY, "idempotency_key_reused")),
                        Ok(record) => { let mut response = Response::new(Full::new(Bytes::from(record.body))); *response.status_mut() = StatusCode::from_u16(record.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR); if let Some(v)=record.content_type.and_then(|v|HeaderValue::from_str(&v).ok()){response.headers_mut().insert(CONTENT_TYPE,v);} if let Some(v)=record.location.and_then(|v|HeaderValue::from_str(&v).ok()){response.headers_mut().insert(hyper::header::LOCATION,v);} response.headers_mut().insert("idempotent-replayed",HeaderValue::from_static("true")); Ok(response) },
                        Err(_) => Ok(error_response(StatusCode::SERVICE_UNAVAILABLE, "idempotency_store_unavailable")),
                    }
                }
            }
        })
    }
}

async fn buffer_request<B>(req: Request<B>) -> Result<Request<Full<Bytes>>, ()> where B: http_body::Body<Data=Bytes> + Send, B::Error: std::fmt::Debug {
    let (parts, body) = req.into_parts();
    let bytes = body.collect().await.map_err(|_| ())?.to_bytes();
    Ok(Request::from_parts(parts, Full::new(bytes)))
}

fn error_response(status: StatusCode, code: &str) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(&serde_json::json!({"error":{"code":code}})).unwrap_or_default();
    let mut response = Response::new(Full::new(Bytes::from(body))); *response.status_mut() = status;
    response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json")); response
}

fn hex_hash(data: &[u8]) -> String { Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect() }
