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

use base64::Engine;
use hmac::{Hmac, Mac};
use hyper::header::{HeaderMap, HeaderValue, COOKIE, SET_COOKIE};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::layer::Layer;
use tower::Service;

type HmacSha256 = Hmac<Sha256>;

/// A per-request session handle. Cheaply `Clone` (wraps an `Arc<Mutex<..>>`)
/// so the same handle can be inserted into the request's extensions (for
/// handlers to read/mutate) and retained by SessionLayer (to serialize
/// after the handler returns) without any response-side round-trip.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Mutex<SessionInner>>,
}

// Internal session state protected by a Mutex.
struct SessionInner {
    // The key-value data map stored in the session.
    data: HashMap<String, Value>,
    // Flag indicating if any session key-value data was changed or removed during the request.
    modified: bool,
    /// Set by `cycle_key`; on the next encode, forces a fresh session
    /// identity (not just re-signing the same data) to prevent session fixation attacks.
    cycled: bool,
    // Flag indicating if this session was newly created during the current request.
    new: bool,
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
            })),
        }
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
            inner.modified = true;
        }
    }

    /// Returns `true` if the session contains no application data keys.
    pub fn is_empty(&self) -> bool {
        if let Ok(inner) = self.inner.lock() {
            inner.data.keys().all(|k| k == "_session_key")
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

// Generates a random 128-bit session key encoded as a hexadecimal string.
fn generate_session_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut s = String::with_capacity(32);
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
            // Session cookies should only be sent over HTTPS by default.
            // Local HTTP development can opt out with `.with_secure(false)`.
            secure: true,
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
    ///
    /// This defaults to `true`; set it to `false` only for local HTTP development.
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

        let expires_at_unix = (std::time::SystemTime::now() + self.max_age)
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

        Some(Session {
            inner: Arc::new(Mutex::new(SessionInner {
                data,
                modified: false,
                cycled: false,
                new: false,
            })),
        })
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

/// The tower Layer/Service that does the actual per-request work: parses
/// the `Cookie` request header, decodes+verifies the session cookie via
/// the store, inserts the `Session` into the hyper request's `Extensions`
/// before calling the inner service, and (only if `session.modified` or
/// newly created/cleared) sets `Set-Cookie` on the response afterward by
/// reading the same `Arc`-shared handle it kept.
#[derive(Clone)]
pub struct SessionLayer {
    store: Arc<SignedCookieStore>,
}

impl SessionLayer {
    /// Creates a new `SessionLayer` wrapping the specified store.
    pub fn new(store: SignedCookieStore) -> Self {
        Self {
            store: Arc::new(store),
        }
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
    store: Arc<SignedCookieStore>,
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

        let session =
            if let Some(cookie_val) = extract_session_cookie(req.headers(), &store.cookie_name) {
                store.decode(&cookie_val).unwrap_or_else(Session::new_empty)
            } else {
                Session::new_empty()
            };

        req.extensions_mut().insert(session.clone());

        Box::pin(async move {
            let mut resp = inner.call(req).await?;

            let should_save = {
                if let Ok(inner) = session.inner.lock() {
                    inner.modified || inner.new
                } else {
                    false
                }
            };

            if should_save {
                let cookie_value = store.encode(&session);
                let max_age_secs = store.max_age.as_secs();
                let mut set_cookie_val = format!(
                    "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
                    store.cookie_name, cookie_value, max_age_secs
                );
                if store.secure {
                    set_cookie_val.push_str("; Secure");
                }
                if let Ok(hdr_val) = HeaderValue::from_str(&set_cookie_val) {
                    resp.headers_mut().append(SET_COOKIE, hdr_val);
                }
            }

            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests;
