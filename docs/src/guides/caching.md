# Caching

Djangors provides a unified key-value caching system. It supports multiple storage backends, flexible Time-To-Live (TTL) configuration, JSON serialization helpers, and opt-in HTTP response caching middleware.

---

## The `Cache` Trait

At the core of the caching system is the `Cache` trait from `djangors_cache`. It defines the essential operations for a raw-byte key-value store:

```rust,illustrative
use async_trait::async_trait;
use std::time::Duration;
use djangors::cache::CacheError;

#[async_trait]
pub trait Cache: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError>;
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<(), CacheError>;
    async fn delete(&self, key: &str) -> Result<(), CacheError>;
}
```

### High-level Helpers via `CacheExt`

To simplify working with cached structures, Djangors provides `CacheExt` which automatically extends any implementation of `Cache` with:

* **`get_or_set`**: Looks up a key; if missing, executes a closure to compute the raw bytes, caches them, and returns the result.
* **`get_or_set_json`**: Automatically handles JSON serialization and deserialization using `serde_json`.

```rust,illustrative
use djangors::cache::CacheExt;
use std::time::Duration;

let value = cache.get_or_set_json("my_key", Some(Duration::from_secs(60)), || async {
    // Heavy computation returning Result<MyStruct, CacheError>
    Ok(MyStruct { data: 42 })
}).await?;
```

### The free function `get_or_set_fragment`

`get_or_set_fragment` is the same compute-if-missing pattern as a standalone
function for call sites that do not have a concrete `Cache` impl, only the
`dyn Cache` trait object (for example, an async handler receiving the store as
`Arc<dyn Cache>`):

```rust,compile
# use std::sync::Arc;
# use djangors_cache::{Cache, CacheError, InMemoryCache, get_or_set_fragment};
# use std::time::Duration;
# fn main() {}
# async fn run() -> Result<(), CacheError> {
let store: Arc<dyn Cache> = Arc::new(InMemoryCache::new(10_000));
let html = get_or_set_fragment(&*store, "template:home", Some(Duration::from_secs(300)), || async {
    // ... any work producing Vec<u8> ...
    Ok(b"<h1>Hello</h1>".to_vec())
}).await?;
# let _ = html;
# Ok(())
# }
```

`get_or_set` (the `CacheExt` method) takes `&self`; `get_or_set_fragment` takes
`&dyn Cache` — otherwise they are identical in behaviour (both use
compute-if-missing with per-call TTL, and both are best-effort, non-atomic under
concurrency).

---

## Supported Backends

Djangors includes three built-in implementations of the `Cache` trait:

### 1. In-Memory Cache (`InMemoryCache`)
Powered by the `moka` crate under the hood, `InMemoryCache` provides a high-performance, thread-safe in-memory cache featuring size-based eviction.
* **Best for**: Single-process deployments, dev environments, or local caching of immutable data (e.g. templates).
* **Usage**:
  ```rust,illustrative
  use djangors::cache::InMemoryCache;
  let cache = InMemoryCache::new(10_000); // Caps capacity at 10,000 items
  ```

### 2. Database Cache (`DatabaseCache`)
Uses the application's primary database (via a `djangors_cache_entries` table) to persist cache values.
* **Table Schema**:
  * `key`: `TEXT PRIMARY KEY`
  * `value`: `BYTEA` (Postgres) / `BLOB` (SQLite)
  * `expires_at`: `TIMESTAMPTZ`
* **Best for**: Distributed deployments that want to share a cache without setting up a dedicated Redis instance.
* **Usage**:
  ```rust,illustrative
  use djangors::cache::DatabaseCache;
  let cache = DatabaseCache::new(db.clone());
  ```

### 3. Redis Cache (`RedisCache`)
Provides a fast, distributed cache backend backed by a Redis server.
* **Best for**: High-traffic, highly concurrent production deployments.
* **Activation**: Enabled via the `redis` crate feature flag.
* **Usage**:
  ```rust,illustrative
  use djangors::cache::RedisCache;
  let cache = RedisCache::new("redis://127.0.0.1:6379/0")?;
  ```

---

## HTTP Response Caching (`CacheLayer`)

Djangors offers a Tower middleware middleware layer (`CacheLayer`) that automates serving and recording HTTP responses.

### Opt-In-Only Architecture

Unlike some frameworks that cache responses globally, `CacheLayer` is strictly **opt-in per response**. 

A response is only cached if:
1. The incoming request is a `GET` request.
2. The response contains the `CacheableResponse` marker extension.
3. The response does **NOT** contain a `Set-Cookie` header.

#### Why is this opt-in?
HTTP response caching is notoriously prone to security leaks:
* If user-specific pages (e.g. dashboards displaying personal profiles) were cached globally, subsequent users would receive the cached data of the previous user.
* By enforcing that responses must explicitly carry the `CacheableResponse` extension and lack `Set-Cookie` headers, Djangors prevents caching sensitive sessions, CSRF tokens, or user-specific state.

### Using the Cache Middleware

To wire up response caching in your application, wrap your `RouterService` in `CacheLayer` and mark cacheable responses with `.cacheable()` (or `.with_extension(CacheableResponse)`):

```rust,compile
use djangors::cache::{CacheLayer, InMemoryCache};
use djangors::core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors::core::router::RouterService;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceBuilder;

// 1. Mark specific responses as cacheable in your view handler
async fn my_public_view(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "Hello, Cached World!").cacheable())
}

fn wire_cache() {
    // 2. Set up the CacheLayer with your preferred cache backend
    let cache_backend = Arc::new(InMemoryCache::new(1000));
    let cache_layer = CacheLayer::new(cache_backend, Some(Duration::from_secs(300)));

    let router = Router::new().get("/public", my_public_view);
    let _service: djangors::cache::CacheService<RouterService> = ServiceBuilder::new()
        .layer(cache_layer)
        .service(RouterService::new(router, false));
}
```
