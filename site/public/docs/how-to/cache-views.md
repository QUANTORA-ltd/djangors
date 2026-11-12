# How to Cache a View Response

## Problem
You want to cache HTTP GET view responses to reduce database load and speed up response times using in-memory, database, or Redis backends.

## Solution
`djangors_cache::CacheLayer` is a `tower::Layer` that caches GET responses under two strict
security requirements:
1. **Explicit Opt-in**: the response must carry a `CacheableResponse` marker extension (set with `Response::cacheable()` or `.with_extension(CacheableResponse)`).
2. **No `Set-Cookie`**: responses setting cookies (e.g. session cookies) are **never** cached,
   regardless of the opt-in marker.

## Code Example

### 1. Opting a Djangors View into Response Caching

Call `.cacheable()` on any `djangors_core::Response` and wrap your `RouterService` with `CacheLayer`:

```rust,compile
use std::sync::Arc;
use std::time::Duration;
use djangors_cache::{CacheLayer, InMemoryCache};
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_core::router::RouterService;
use tower::ServiceBuilder;

async fn public_pricing_view(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    // Opt this GET response into caching with `.cacheable()`
    Ok(Response::html(StatusCode::OK, "<h1>Public Pricing</h1>").cacheable())
}

fn build_cached_service() {
    let router = Router::new().get("/pricing", public_pricing_view);
    let router_service = RouterService::new(router, false);

    let cache = Arc::new(InMemoryCache::new(10_000));
    let _service: djangors_cache::CacheService<RouterService> = ServiceBuilder::new()
        .layer(CacheLayer::new(cache, Some(Duration::from_secs(300))))
        .service(router_service);
}
```

### 2. Lower-Level Tower Service Usage

`CacheLayer` also works with raw `hyper::Response<Full<Bytes>>` services by inserting `CacheableResponse` into `extensions_mut()` directly:

```rust,compile
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response};
use tower::{service_fn, Layer, Service, ServiceExt};
use djangors_cache::{InMemoryCache, CacheLayer, CacheableResponse};

async fn build_and_call_cached_service() {
    let handler = service_fn(|_req: Request<Full<Bytes>>| async move {
        let mut r = Response::new(Full::new(Bytes::from_static(b"dashboard body")));
        r.extensions_mut().insert(CacheableResponse);
        Ok::<_, std::convert::Infallible>(r)
    });

    let cache = Arc::new(InMemoryCache::new(10_000));
    let mut svc = CacheLayer::new(cache, Some(Duration::from_secs(300))).layer(handler);

    let request = Request::builder()
        .method("GET")
        .uri("/dashboard")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let _response = svc.ready().await.unwrap().call(request).await.unwrap();
}
```
