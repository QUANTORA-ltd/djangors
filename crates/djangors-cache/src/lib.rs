#![deny(missing_docs)]
//! Raw-byte caches, common backends, and explicitly opted-in response caching.
//!
//! This crate provides a unified interface for key-value caching:
//! - [`Cache`]: The core trait defining raw-byte get, set, and delete operations.
//! - [`CacheExt`]: Extension trait providing higher-level helpers like `get_or_set` and JSON serialization/deserialization.
//! - Implementations:
//!   - [`InMemoryCache`]: Fast in-memory backend using Moka cache.
//!   - [`DatabaseCache`]: SQL-database-backed cache using `djangors_cache_entries` table.
//!   - `RedisCache`: Distributed backend powered by Redis.
//! - Tower Middleware: [`CacheLayer`] and [`CacheService`] intercept GET requests, serving and caching responses
//!   explicitly marked with the [`CacheableResponse`] extension.

use async_trait::async_trait;
use chrono::Utc;
use http_body_util::Full;
use hyper::{Request, Response};
use moka::future::Cache as MokaCache;
use serde::{de::DeserializeOwned, Serialize};
use std::{convert::Infallible, future::Future, sync::Arc, time::Duration};
use thiserror::Error;
use tower::{Layer, Service};

mod idempotency;
pub use idempotency::{IdempotencyLayer, IdempotencyService};

/// Errors encountered during cache access or value serialization.
#[derive(Debug, Error)]
pub enum CacheError {
    /// An error reported by the underlying cache backend.
    #[error("cache backend error: {0}")]
    Backend(String),
    /// An error serializing or deserializing cached values.
    #[error("cache serialization error: {0}")]
    Serialization(String),
    /// The selected backend does not support atomic insert-if-absent.
    #[error("cache backend does not support atomic add")]
    Unsupported(&'static str),
}

/// An object-safe cache of raw byte values.
#[async_trait]
pub trait Cache: Send + Sync {
    /// Retrieves the raw byte value associated with `key` if present and unexpired.
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError>;
    /// Stores `value` under `key` with an optional time-to-live (`ttl`).
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>)
        -> Result<(), CacheError>;
    /// Deletes any entry associated with `key`.
    async fn delete(&self, key: &str) -> Result<(), CacheError>;
    /// Atomically stores `value` only when `key` is absent. Returns `true` when stored.
    async fn add(
        &self,
        _key: &str,
        _value: Vec<u8>,
        _ttl: Option<Duration>,
    ) -> Result<bool, CacheError> {
        Err(CacheError::Unsupported("add"))
    }
}

/// Extension trait providing convenient `get_or_set` helpers on any [`Cache`].
#[async_trait]
pub trait CacheExt: Cache {
    /// Retrieves `key` if cached; otherwise executes `f`, caches the result under `key`, and returns it.
    async fn get_or_set<F, Fut>(
        &self,
        key: &str,
        ttl: Option<Duration>,
        f: F,
    ) -> Result<Vec<u8>, CacheError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<Vec<u8>, CacheError>> + Send,
    {
        if let Some(value) = self.get(key).await? {
            return Ok(value);
        }
        let value = f().await?;
        self.set(key, value.clone(), ttl).await?;
        Ok(value)
    }

    /// Retrieves and deserializes `key` from JSON; otherwise executes `f`, serializes to JSON, caches, and returns `T`.
    async fn get_or_set_json<T, F, Fut>(
        &self,
        key: &str,
        ttl: Option<Duration>,
        f: F,
    ) -> Result<T, CacheError>
    where
        T: Serialize + DeserializeOwned + Send,
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, CacheError>> + Send,
    {
        let bytes = self
            .get_or_set(key, ttl, || async {
                let value = f().await?;
                serde_json::to_vec(&value).map_err(|e| CacheError::Serialization(e.to_string()))
            })
            .await?;
        serde_json::from_slice(&bytes).map_err(|e| CacheError::Serialization(e.to_string()))
    }
}
impl<T: Cache + ?Sized> CacheExt for T {}

/// Helper function to retrieve or compute a template fragment or raw byte cache entry.
pub async fn get_or_set_fragment<F, Fut>(
    cache: &dyn Cache,
    key: &str,
    ttl: Option<Duration>,
    f: F,
) -> Result<Vec<u8>, CacheError>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<Vec<u8>, CacheError>> + Send,
{
    cache.get_or_set(key, ttl, f).await
}

/// An in-memory cache backend powered by Moka.
#[derive(Clone)]
pub struct InMemoryCache {
    inner: MokaCache<String, MemoryEntry>,
    add_lock: Arc<tokio::sync::Mutex<()>>,
}
// Internally wraps cached bytes and their absolute expiration timestamp inside the Moka cache backend.
#[derive(Clone)]
struct MemoryEntry {
    // The raw cached byte payload.
    value: Vec<u8>,
    // Optional point in time when the entry becomes stale.
    expires_at: Option<std::time::Instant>,
}
impl InMemoryCache {
    /// Creates a new `InMemoryCache` instance with `max_capacity` elements limit.
    pub fn new(max_capacity: u64) -> Self {
        Self {
            inner: MokaCache::builder().max_capacity(max_capacity).build(),
            add_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}
impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new(10_000)
    }
}
#[async_trait]
impl Cache for InMemoryCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        Ok(self.inner.get(key).await.and_then(|entry| {
            if entry
                .expires_at
                .is_none_or(|e| e > std::time::Instant::now())
            {
                Some(entry.value)
            } else {
                None
            }
        }))
    }
    async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        self.inner
            .insert(
                key.to_owned(),
                MemoryEntry {
                    value,
                    expires_at: ttl.map(|t| std::time::Instant::now() + t),
                },
            )
            .await;
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        self.inner.invalidate(key).await;
        Ok(())
    }
    async fn add(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<bool, CacheError> {
        let _guard = self.add_lock.lock().await;
        if self.inner.get(key).await.is_some_and(|entry| {
            entry
                .expires_at
                .is_some_and(|e| e <= std::time::Instant::now())
        }) {
            self.inner.invalidate(key).await;
        }
        let entry = self
            .inner
            .entry(key.to_owned())
            .or_insert_with(async move {
                MemoryEntry {
                    value,
                    expires_at: ttl.map(|t| std::time::Instant::now() + t),
                }
            })
            .await;
        Ok(entry.is_fresh())
    }
}

/// A database-backed cache backend storing entries in `djangors_cache_entries`.
#[derive(Clone)]
pub struct DatabaseCache {
    db: djangors_db::Database,
}
impl DatabaseCache {
    /// Creates a new `DatabaseCache` backend using `db`.
    pub fn new(db: djangors_db::Database) -> Self {
        Self { db }
    }
    // Lazily ensures that the database cache table `djangors_cache_entries` exists.
    // This is run before every cache operation to guarantee the table is set up.
    async fn ensure_table(&self) -> Result<(), CacheError> {
        let mut conn = self.db.conn();
        let dialect = conn.dialect();
        let bytea = dialect.bytea_type();
        let timestamp = dialect.timestamp_type();
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS djangors_cache_entries (key TEXT PRIMARY KEY, value {bytea} NOT NULL, expires_at {timestamp})"
        );
        conn.execute(&sql, &[])
            .await
            .map(|_| ())
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
}
#[async_trait]
impl Cache for DatabaseCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql = format!("SELECT value, expires_at FROM djangors_cache_entries WHERE key = {p1}");
        let params = vec![djangors_db::BindValue::Text(key.to_string())];
        let row_opt = conn
            .fetch_optional(&sql, &params)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))?;
        if let Some(row) = row_opt {
            let value = row
                .try_bytes(0)
                .map_err(|e| CacheError::Backend(e.to_string()))?
                .unwrap_or_default();
            let expiry = row
                .try_datetime(1)
                .map_err(|e| CacheError::Backend(e.to_string()))?;
            if expiry.is_none_or(|e| e > Utc::now()) {
                return Ok(Some(value));
            }
            self.delete(key).await?;
        }
        Ok(None)
    }
    async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        self.ensure_table().await?;
        let expires = ttl.map(|t| Utc::now() + chrono::Duration::from_std(t).unwrap_or_default());
        let mut conn = self.db.conn();
        let dialect = conn.dialect();
        let p1 = dialect.placeholder(1);
        let p2 = dialect.placeholder(2);
        let p3 = dialect.placeholder(3);
        let sql = format!(
            "INSERT INTO djangors_cache_entries (key, value, expires_at) VALUES ({p1}, {p2}, {p3}) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, expires_at = EXCLUDED.expires_at"
        );
        let expires_param = match expires {
            Some(dt) => djangors_db::BindValue::DateTime(dt),
            None => djangors_db::BindValue::Null(djangors_db::NullKind::DateTime),
        };
        let params = vec![
            djangors_db::BindValue::Text(key.to_string()),
            djangors_db::BindValue::Bytes(value),
            expires_param,
        ];
        conn.execute(&sql, &params)
            .await
            .map(|_| ())
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql = format!("DELETE FROM djangors_cache_entries WHERE key = {p1}");
        let params = vec![djangors_db::BindValue::Text(key.to_string())];
        conn.execute(&sql, &params)
            .await
            .map(|_| ())
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn add(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<bool, CacheError> {
        self.ensure_table().await?;
        let expires = ttl.map(|t| Utc::now() + chrono::Duration::from_std(t).unwrap_or_default());
        let mut conn = self.db.conn();
        let dialect = conn.dialect();
        let p1 = dialect.placeholder(1);
        let p2 = dialect.placeholder(2);
        let p3 = dialect.placeholder(3);
        let cleanup = format!("DELETE FROM djangors_cache_entries WHERE key = {p1} AND expires_at IS NOT NULL AND expires_at <= {p2}");
        conn.execute(
            &cleanup,
            &[
                djangors_db::BindValue::Text(key.to_string()),
                djangors_db::BindValue::DateTime(Utc::now()),
            ],
        )
        .await
        .map_err(|e| CacheError::Backend(e.to_string()))?;
        let insert = format!("INSERT INTO djangors_cache_entries (key, value, expires_at) VALUES ({p1}, {p2}, {p3}) ON CONFLICT (key) DO NOTHING");
        let expiry =
            expires
                .map(djangors_db::BindValue::DateTime)
                .unwrap_or(djangors_db::BindValue::Null(
                    djangors_db::NullKind::DateTime,
                ));
        let rows = conn
            .execute(
                &insert,
                &[
                    djangors_db::BindValue::Text(key.to_string()),
                    djangors_db::BindValue::Bytes(value),
                    expiry,
                ],
            )
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))?;
        Ok(rows > 0)
    }
}

#[cfg(feature = "redis")]
/// A Redis-backed cache implementation.
#[derive(Clone)]
pub struct RedisCache {
    client: redis::Client,
}
#[cfg(feature = "redis")]
impl RedisCache {
    /// Creates a new `RedisCache` connecting to the given Redis URL string.
    pub fn new(url: &str) -> Result<Self, CacheError> {
        redis::Client::open(url)
            .map(|client| Self { client })
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, CacheError> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
}
#[cfg(feature = "redis")]
#[async_trait]
impl Cache for RedisCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let mut c = self.connection().await?;
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut c)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        let mut c = self.connection().await?;
        let mut cmd = redis::cmd("SET");
        cmd.arg(key).arg(value);
        if let Some(t) = ttl {
            cmd.arg("PX").arg(t.as_millis() as u64);
        }
        cmd.query_async::<()>(&mut c)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        let mut c = self.connection().await?;
        redis::cmd("DEL")
            .arg(key)
            .query_async::<()>(&mut c)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn add(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<bool, CacheError> {
        let mut c = self.connection().await?;
        let mut cmd = redis::cmd("SET");
        cmd.arg(key).arg(value).arg("NX");
        if let Some(ttl) = ttl {
            cmd.arg("PX").arg(ttl.as_millis().max(1) as u64);
        }
        let result: Option<String> = cmd
            .query_async(&mut c)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))?;
        Ok(result.is_some())
    }
}

/// Request/response extension marker indicating that a response is cacheable by [`CacheLayer`].
#[derive(Clone, Copy, Debug)]
pub struct CacheableResponse;

/// Tower middleware layer for caching responses marked with [`CacheableResponse`].
#[derive(Clone)]
pub struct CacheLayer {
    // The cache implementation where response payloads are stored.
    cache: Arc<dyn Cache>,
    // Default time-to-live duration for responses cached by this layer.
    ttl: Option<Duration>,
}
impl CacheLayer {
    /// Creates a new `CacheLayer` using `cache` and optional default `ttl`.
    pub fn new(cache: Arc<dyn Cache>, ttl: Option<Duration>) -> Self {
        Self { cache, ttl }
    }
}
impl<S> Layer<S> for CacheLayer {
    type Service = CacheService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        CacheService {
            inner,
            cache: self.cache.clone(),
            ttl: self.ttl,
        }
    }
}

/// Tower service middleware that intercepts GET requests and serves/caches responses.
#[derive(Clone)]
pub struct CacheService<S> {
    // The wrapped inner Tower service.
    inner: S,
    // The cache backend used to store HTTP responses.
    cache: Arc<dyn Cache>,
    // Expiration duration applied to newly cached responses.
    ttl: Option<Duration>,
}
impl<S, ReqBody> Service<Request<ReqBody>> for CacheService<S>
where
    S: Service<Request<ReqBody>, Response = Response<Full<bytes::Bytes>>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response<Full<bytes::Bytes>>;
    type Error = Infallible;
    type Future =
        std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }
    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        let cache = self.cache.clone();
        let ttl = self.ttl;
        Box::pin(async move {
            if req.method() != hyper::Method::GET {
                return inner.call(req).await;
            }
            let key = format!("{}?{}", req.uri().path(), req.uri().query().unwrap_or(""));
            if let Some(bytes) = cache.get(&key).await.unwrap_or(None) {
                if let Ok(cached) = serde_json::from_slice::<CachedResponse>(&bytes) {
                    return Ok(cached.into_response());
                }
            }
            let response = inner.call(req).await?;
            if response.extensions().get::<CacheableResponse>().is_some()
                && !response.headers().contains_key(hyper::header::SET_COOKIE)
            {
                let cached = CachedResponse::from_response(&response);
                if let Ok(bytes) = serde_json::to_vec(&cached) {
                    let _ = cache.set(&key, bytes, ttl).await;
                }
            }
            Ok(response)
        })
    }
}
// Internal representation of a cached response, storing status code, headers,
// and body bytes in a serializable format.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedResponse {
    // The HTTP status code as a raw u16.
    status: u16,
    // The HTTP headers, stored as name-value byte pairs.
    headers: Vec<(String, Vec<u8>)>,
    // The raw response body bytes.
    body: Vec<u8>,
}
impl CachedResponse {
    fn from_response(r: &Response<Full<bytes::Bytes>>) -> Self {
        Self {
            status: r.status().as_u16(),
            headers: r
                .headers()
                .iter()
                .map(|(n, v)| (n.to_string(), v.as_bytes().to_vec()))
                .collect(),
            body: r.body().clone().into_inner().unwrap_or_default().to_vec(),
        }
    }
    fn into_response(self) -> Response<Full<bytes::Bytes>> {
        let mut r = Response::builder()
            .status(self.status)
            .body(Full::new(bytes::Bytes::from(self.body)))
            .unwrap();
        for (n, v) in self.headers {
            if let (Ok(n), Ok(v)) = (
                n.parse::<hyper::header::HeaderName>(),
                hyper::header::HeaderValue::from_bytes(&v),
            ) {
                r.headers_mut().append(n, v);
            }
        }
        r.extensions_mut().insert(CacheableResponse);
        r
    }
}

/// Distributed lock backed by Redis `SET key token NX PX ttl` and Lua-script release.
#[cfg(feature = "redis")]
pub struct RedisLock;

#[cfg(feature = "redis")]
impl RedisLock {
    /// Attempts to acquire a lock on `key` for `ttl`.
    ///
    /// Returns `Ok(Some(guard))` if acquired, or `Ok(None)` if another caller
    /// already holds `key`. `ttl` must exceed the expected work duration so the
    /// lock does not expire mid-operation.
    pub async fn acquire(
        client: &redis::Client,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<RedisLockGuard>, CacheError> {
        use rand::RngCore;
        let mut bytes = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();

        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))?;
        let ttl_ms = ttl.as_millis().max(1) as u64;
        let res: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(ttl_ms)
            .query_async(&mut conn)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))?;

        if res.is_some() {
            Ok(Some(RedisLockGuard {
                client: client.clone(),
                key: key.to_string(),
                token,
            }))
        } else {
            Ok(None)
        }
    }
}

/// Guard representing an acquired [`RedisLock`].
///
/// Call [`release`](Self::release) when done to delete the key only if the
/// ownership token still matches.
#[cfg(feature = "redis")]
#[derive(Debug, Clone)]
pub struct RedisLockGuard {
    client: redis::Client,
    key: String,
    token: String,
}

#[cfg(feature = "redis")]
impl RedisLockGuard {
    /// Returns the locked Redis key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the ownership token stored under `key`.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Releases the lock using a Lua script that deletes `key` only if its
    /// stored value still equals this guard's token.
    ///
    /// Returns `Ok(true)` if the lock was deleted, or `Ok(false)` if it had
    /// already expired or been replaced by another owner.
    pub async fn release(self) -> Result<bool, CacheError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))?;
        let script = redis::Script::new(
            "if redis.call('get', KEYS[1]) == ARGV[1] then return redis.call('del', KEYS[1]) else return 0 end",
        );
        let deleted: i64 = script
            .key(&self.key)
            .arg(&self.token)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))?;
        Ok(deleted == 1)
    }
}

#[cfg(test)]
mod tests;
