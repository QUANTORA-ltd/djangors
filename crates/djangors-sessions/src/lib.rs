#![deny(missing_docs)]
//! Session engines for the Djangors web framework.
//!
//! Provides a signed-cookie session engine. Session state is serialized to JSON,
//! base64-encoded, signed with HMAC-SHA256 using the application's secret key, and
//! stored directly on the client's browser as a cookie.
//!
//! Key components:
//! - [`Session`]: Thread-safe handle to read, write, and manage request-local session state.
//! - [`SignedCookieStore`]: Serializes, signs, deserializes, and verifies cookies.
//! - [`SessionLayer`]: Tower middleware injecting [`Session`] into request extensions and writing updated cookies to response headers.

use async_trait::async_trait;
use base64::Engine;
use hmac::{Hmac, Mac};
use hyper::header::{HeaderMap, HeaderValue, COOKIE, SET_COOKIE, USER_AGENT};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tower::layer::Layer;
use tower::Service;

pub mod store;
#[cfg(feature = "redis")]
pub use store::RedisStore;
pub use store::{hash_session_key, DatabaseStore, SessionContext, SessionRecord, SessionStore};

type HmacSha256 = Hmac<Sha256>;

/// Errors produced by session stores.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SessionError {
    /// Underlying database or Redis backend error.
    #[error("session backend error: {0}")]
    Backend(String),
    /// Session data serialization or deserialization error.
    #[error("session serialization error: {0}")]
    Serialization(String),
}

/// Controls the lifetime and cookie `Max-Age` behavior of a [`Session`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionExpiry {
    /// Use the store's default `max_age` for both cookie `Max-Age` and server expiry.
    Default,
    /// Browser-session cookie (no `Max-Age` attribute on `Set-Cookie`),
    /// while the server/signed payload still enforces the store's `max_age`.
    BrowserSession,
    /// Persistent cookie and server expiry with an explicit `Max-Age` duration.
    MaxAge(Duration),
}

/// A per-request session handle. Cheaply `Clone` (wraps an `Arc<Mutex<..>>`)
/// so the same handle can be inserted into the request's extensions (for
/// handlers to read/mutate) and retained by SessionLayer (to serialize
/// after the handler returns) without any response-side round-trip.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Mutex<SessionInner>>,
}

// Internal session state protected by a Mutex.
pub(crate) struct SessionInner {
    // The key-value data map stored in the session.
    pub(crate) data: HashMap<String, Value>,
    // Flag indicating if any session key-value data was changed or removed during the request.
    pub(crate) modified: bool,
    /// Set by `cycle_key`; on the next encode, forces a fresh session
    /// identity (not just re-signing the same data) to prevent session fixation attacks.
    pub(crate) cycled: bool,
    // Flag indicating if this session was newly created during the current request.
    pub(crate) new: bool,
    // Per-session expiry configuration.
    pub(crate) expiry: SessionExpiry,
    // Raw key loaded from a server-side store, if any.
    pub(crate) loaded_key: Option<String>,
}

const EXPIRY_BROWSER_ONLY_KEY: &str = "_session_browser_only";
const EXPIRY_MAX_AGE_SECS_KEY: &str = "_session_max_age_secs";

pub(crate) fn expiry_from_data(data: &HashMap<String, Value>) -> SessionExpiry {
    if data
        .get(EXPIRY_BROWSER_ONLY_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        SessionExpiry::BrowserSession
    } else if let Some(secs) = data.get(EXPIRY_MAX_AGE_SECS_KEY).and_then(|v| v.as_u64()) {
        SessionExpiry::MaxAge(Duration::from_secs(secs))
    } else {
        SessionExpiry::Default
    }
}

impl Session {
    /// Creates a new empty session with a unique session key.
    pub fn new_empty() -> Self {
        let mut data = HashMap::new();
        data.insert(
            "_session_key".to_string(),
            Value::String(generate_session_key()),
        );
        Self {
            inner: Arc::new(Mutex::new(SessionInner {
                data,
                modified: false,
                cycled: false,
                new: true,
                expiry: SessionExpiry::Default,
                loaded_key: None,
            })),
        }
    }

    /// Returns the current session key.
    pub fn session_key(&self) -> String {
        if let Ok(inner) = self.inner.lock() {
            if let Some(Value::String(key)) = inner.data.get("_session_key") {
                return key.clone();
            }
        }
        String::new()
    }

    /// Configures per-session expiration.
    ///
    /// - `Some(duration)` sets an explicit `Max-Age` on the cookie and server expiry.
    /// - `None` configures a browser-session cookie (omits `Max-Age` so the browser
    ///   drops it on close) while still enforcing the store's server-side expiry.
    pub fn set_expiry(&self, max_age: Option<Duration>) {
        if let Ok(mut inner) = self.inner.lock() {
            match max_age {
                Some(dur) => {
                    inner.expiry = SessionExpiry::MaxAge(dur);
                    inner.data.remove(EXPIRY_BROWSER_ONLY_KEY);
                    inner.data.insert(
                        EXPIRY_MAX_AGE_SECS_KEY.to_string(),
                        Value::Number(serde_json::Number::from(dur.as_secs())),
                    );
                }
                None => {
                    inner.expiry = SessionExpiry::BrowserSession;
                    inner.data.remove(EXPIRY_MAX_AGE_SECS_KEY);
                    inner
                        .data
                        .insert(EXPIRY_BROWSER_ONLY_KEY.to_string(), Value::Bool(true));
                }
            }
            inner.modified = true;
        }
    }

    /// Returns the current [`SessionExpiry`] mode for this session.
    pub fn expiry(&self) -> SessionExpiry {
        self.inner
            .lock()
            .map(|i| i.expiry)
            .unwrap_or(SessionExpiry::Default)
    }

    /// Gets a deserialized value for the specified key from the session.
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let inner = self.inner.lock().ok()?;
        let val = inner.data.get(key)?;
        serde_json::from_value(val.clone()).ok()
    }

    /// Sets a serializable value for the specified key in the session and marks it as modified.
    pub fn set<T: serde::Serialize>(&self, key: &str, value: T) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Ok(json_val) = serde_json::to_value(value) {
                inner.data.insert(key.to_string(), json_val);
                inner.modified = true;
            }
        }
    }

    /// Removes a key and its value from the session and marks it as modified.
    pub fn remove(&self, key: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.data.remove(key).is_some() {
                inner.modified = true;
            }
        }
    }

    /// Clears all non-internal key-value pairs from the session and marks it as modified.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.data.clear();
            inner.data.insert(
                "_session_key".to_string(),
                Value::String(generate_session_key()),
            );
            inner.expiry = SessionExpiry::Default;
            inner.modified = true;
        }
    }

    /// Returns `true` if the session contains no application data keys.
    pub fn is_empty(&self) -> bool {
        if let Ok(inner) = self.inner.lock() {
            inner.data.keys().all(|k| {
                k == "_session_key" || k == EXPIRY_BROWSER_ONLY_KEY || k == EXPIRY_MAX_AGE_SECS_KEY
            })
        } else {
            true
        }
    }

    /// Rotate session identity in place (auth calls this on login/logout).
    pub fn cycle_key(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.cycled = true;
            inner.modified = true;
        }
    }
}

// Generates a random 256-bit (32-byte) session key encoded as a hexadecimal string.
pub(crate) fn generate_session_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        write!(&mut s, "{:02x}", b).unwrap();
    }
    s
}

/// A signed cookie store for encoding and decoding session data.
pub struct SignedCookieStore {
    // Secret key used to sign and verify HMAC-SHA256 signatures. Derived from settings.SECRET_KEY.
    key: Vec<u8>,
    // Name of the cookie sent to the client (defaults to "djangors_sessionid").
    cookie_name: String,
    // Expiration duration for session cookies (defaults to 2 weeks).
    max_age: Duration,
    // Flag indicating whether the cookie should only be sent over HTTPS (sets the Secure flag).
    secure: bool,
}

impl SignedCookieStore {
    /// Creates a new `SignedCookieStore` using the provided secret key.
    pub fn new(secret_key: &[u8]) -> Self {
        Self {
            key: secret_key.to_vec(),
            cookie_name: "djangors_sessionid".to_string(),
            max_age: Duration::from_secs(14 * 24 * 60 * 60), // 2 weeks
            secure: false,
        }
    }

    /// Configures a custom cookie name.
    pub fn with_cookie_name(mut self, name: String) -> Self {
        self.cookie_name = name;
        self
    }

    /// Configures the cookie max age duration.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Configures whether the cookie should set the `Secure` flag.
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Encodes and signs the session state into a cookie value string.
    pub fn encode(&self, session: &Session) -> String {
        let mut inner = session.inner.lock().unwrap();
        if inner.cycled {
            inner.data.insert(
                "_session_key".to_string(),
                Value::String(generate_session_key()),
            );
            inner.cycled = false;
        }
        let json_str = serde_json::to_string(&inner.data).unwrap();
        let b64_json = base64::engine::general_purpose::STANDARD.encode(json_str.as_bytes());

        let effective_max_age = match inner.expiry {
            SessionExpiry::MaxAge(d) => d,
            SessionExpiry::Default | SessionExpiry::BrowserSession => self.max_age,
        };
        let expires_at_unix = (std::time::SystemTime::now() + effective_max_age)
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        let b64_expiry = base64::engine::general_purpose::STANDARD
            .encode(expires_at_unix.to_string().as_bytes());

        let msg = format!("{}.{}", b64_json, b64_expiry);

        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(msg.as_bytes());
        let mac_result = mac.finalize().into_bytes();
        let b64_mac = base64::engine::general_purpose::STANDARD.encode(mac_result);

        format!("{}.{}.{}", b64_json, b64_expiry, b64_mac)
    }

    /// Returns `None` for missing/malformed/tampered/expired cookies —
    /// never an `Err` that would need special-casing by the layer.
    pub fn decode(&self, cookie_value: &str) -> Option<Session> {
        let parts: Vec<&str> = cookie_value.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let b64_json = parts[0];
        let b64_expiry = parts[1];
        let b64_mac = parts[2];

        // 1. Verify HMAC
        let msg = format!("{}.{}", b64_json, b64_expiry);
        let mac_bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_mac)
            .ok()?;
        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(msg.as_bytes());
        mac.verify_slice(&mac_bytes).ok()?;

        // 2. Verify Expiry
        let expiry_bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_expiry)
            .ok()?;
        let expiry_str = std::str::from_utf8(&expiry_bytes).ok()?;
        let expiry_secs: u64 = expiry_str.parse().ok()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        if expiry_secs <= now {
            return None;
        }

        // 3. Decode JSON
        let json_bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_json)
            .ok()?;
        let data: HashMap<String, Value> = serde_json::from_slice(&json_bytes).ok()?;
        let expiry_mode = expiry_from_data(&data);

        Some(Session {
            inner: Arc::new(Mutex::new(SessionInner {
                data,
                modified: false,
                cycled: false,
                new: false,
                expiry: expiry_mode,
                loaded_key: None,
            })),
        })
    }
}

#[async_trait]
impl SessionStore for SignedCookieStore {
    fn cookie_name(&self) -> &str {
        &self.cookie_name
    }

    fn max_age(&self) -> Duration {
        self.max_age
    }

    fn secure(&self) -> bool {
        self.secure
    }

    async fn load(&self, cookie_value: &str) -> Result<Option<Session>, SessionError> {
        Ok(self.decode(cookie_value))
    }

    async fn save(&self, session: &Session, _ctx: &SessionContext) -> Result<String, SessionError> {
        Ok(self.encode(session))
    }

    async fn delete(&self, _key_or_hash: &str) -> Result<(), SessionError> {
        Ok(())
    }

    async fn for_user(&self, _user_id: i64) -> Result<Vec<SessionRecord>, SessionError> {
        Ok(Vec::new())
    }
}

// Helper function to extract a specific cookie value by name from the Cookie header map.
fn extract_session_cookie(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    for cookie_header in headers.get_all(COOKIE) {
        let cookie_str = cookie_header.to_str().ok()?;
        for cookie in cookie_str.split(';') {
            let mut parts = cookie.trim().splitn(2, '=');
            let name = parts.next()?;
            if name == cookie_name {
                let val = parts.next()?;
                return Some(val.to_string());
            }
        }
    }
    None
}

fn extract_session_context(headers: &HeaderMap) -> SessionContext {
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    SessionContext { ip, user_agent }
}

/// The tower Layer/Service that does the actual per-request work: parses
/// the `Cookie` request header, decodes+verifies the session cookie via
/// the store, inserts the `Session` into the hyper request's `Extensions`
/// before calling the inner service, and (only if `session.modified` or
/// newly created/cleared) sets `Set-Cookie` on the response afterward by
/// reading the same `Arc`-shared handle it kept.
#[derive(Clone)]
pub struct SessionLayer {
    store: Arc<dyn SessionStore>,
}

impl SessionLayer {
    /// Creates a new `SessionLayer` wrapping the specified [`SessionStore`].
    pub fn new(store: impl SessionStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    /// Creates a new `SessionLayer` from an existing `Arc<dyn SessionStore>`.
    pub fn from_arc(store: Arc<dyn SessionStore>) -> Self {
        Self { store }
    }
}

impl<S> Layer<S> for SessionLayer {
    type Service = SessionService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SessionService {
            inner,
            store: self.store.clone(),
        }
    }
}

/// Tower service middleware managing per-request session lifecycle.
#[derive(Clone)]
pub struct SessionService<S> {
    inner: S,
    store: Arc<dyn SessionStore>,
}

impl<S, ReqBody, ResBody> Service<hyper::Request<ReqBody>> for SessionService<S>
where
    S: Service<hyper::Request<ReqBody>, Response = hyper::Response<ResBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        let store = self.store.clone();
        let cookie_val = extract_session_cookie(req.headers(), store.cookie_name());
        let ctx = extract_session_context(req.headers());

        Box::pin(async move {
            let session = match cookie_val {
                Some(val) => store
                    .load(&val)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(Session::new_empty),
                None => Session::new_empty(),
            };

            req.extensions_mut().insert(session.clone());
            req.extensions_mut().insert(store.clone());

            let mut resp = inner.call(req).await?;

            let should_save = {
                if let Ok(guard) = session.inner.lock() {
                    guard.modified || guard.new
                } else {
                    false
                }
            };

            if should_save {
                if let Ok(cookie_value) = store.save(&session, &ctx).await {
                    let mut set_cookie_val = format!(
                        "{}={}; Path=/; HttpOnly; SameSite=Lax",
                        store.cookie_name(),
                        cookie_value
                    );
                    match session.expiry() {
                        SessionExpiry::Default => {
                            set_cookie_val
                                .push_str(&format!("; Max-Age={}", store.max_age().as_secs()));
                        }
                        SessionExpiry::MaxAge(d) => {
                            set_cookie_val.push_str(&format!("; Max-Age={}", d.as_secs()));
                        }
                        SessionExpiry::BrowserSession => {}
                    }
                    if store.secure() {
                        set_cookie_val.push_str("; Secure");
                    }
                    if let Ok(hdr_val) = HeaderValue::from_str(&set_cookie_val) {
                        resp.headers_mut().append(SET_COOKIE, hdr_val);
                    }
                }
            }

            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests;
