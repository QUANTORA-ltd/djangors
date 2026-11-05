use std::collections::HashMap;

use bytes::Bytes;
use hyper::http::{Extensions, HeaderMap, HeaderValue, Method, Uri};
use percent_encoding::percent_decode_str;

use crate::state::AppState;

/// An HTTP request with a fully buffered body.
///
/// Constructed by the router from an incoming hyper request before being
/// passed to handlers. All body bytes are read eagerly at construction time.
#[derive(Debug, Clone)]
pub struct Request {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    query_params: HashMap<String, String>,
    state: AppState,
    extensions: Extensions,
}

impl Request {
    /// Create a new `Request` from its component parts.
    ///
    /// The query string (if any) is parsed and URL-decoded at construction
    /// time and will be accessible via `.query(name)`.
    pub fn new(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Self {
        let query_params = Self::parse_query(uri.query().unwrap_or(""));
        Request {
            method,
            uri,
            headers,
            body,
            query_params,
            state: AppState::default(),
            extensions: Extensions::new(),
        }
    }

    /// Retrieve a piece of shared state attached to the app via
    /// [`Router::with_state`](crate::router::Router::with_state), if any was registered for type `T`.
    pub fn state<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.state.get::<T>()
    }

    /// Retrieve shared state of type `T`, failing with a descriptive
    /// [`DjangorsError::Internal`] when it was never attached.
    ///
    /// Missing state is a wiring mistake, not a request-level failure, and
    /// every handler that needs a database was hand-writing the same
    /// `.ok_or_else(...)`. This names the missing type in the message, so the
    /// error points at the `with_state` call that was forgotten:
    ///
    /// ```no_run
    /// # use djangors_core::request::Request;
    /// # use djangors_core::error::DjangorsError;
    /// # struct Database;
    /// # fn demo(req: &Request) -> Result<(), DjangorsError> {
    /// let db = req.require_state::<Database>()?;
    /// # let _ = db;
    /// # Ok(())
    /// # }
    /// ```
    pub fn require_state<T: Send + Sync + 'static>(
        &self,
    ) -> Result<&T, crate::error::DjangorsError> {
        self.state.get::<T>().ok_or_else(|| {
            crate::error::DjangorsError::Internal(format!(
                "state of type `{}` was not attached; add it with Router::with_state",
                std::any::type_name::<T>()
            ))
        })
    }

    /// Attach the app-wide state to this request.
    pub fn with_state(mut self, state: AppState) -> Self {
        self.state = state;
        self
    }

    /// Retrieve a per-request typed value inserted by upstream middleware
    /// (e.g. a session handle), if any was set for type `T`.
    ///
    /// Distinct from [`Request::state`]: state is app-wide and configured
    /// once via [`Router::with_state`](crate::Router::with_state);
    /// extensions are populated fresh per request by tower [`Layer`](tower::Layer)s
    /// wrapping the router (via the incoming hyper request's own
    /// `Extensions`), which is how `Router::dispatch` picks them up.
    pub fn ext<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.extensions.get::<T>()
    }

    /// Replace this request's per-request extensions, typically propagated
    /// from the incoming hyper request by `Router::dispatch`.
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }

    /// Mutable access to this request's per-request extensions.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// The route pattern that matched this request, if the router found one.
    ///
    /// Returns e.g. `/api/results/{id:i64}/approve` rather than
    /// `/api/results/8812/approve`, which makes it safe to use as a metrics
    /// label or log field.
    pub fn matched_route(&self) -> Option<&str> {
        self.ext::<crate::router::MatchedRoute>().map(|m| m.as_str())
    }

    fn parse_query(query: &str) -> HashMap<String, String> {
        let mut params = HashMap::new();
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut parts = pair.splitn(2, '=');
            let key_raw = parts.next().unwrap_or("");
            let val_raw = parts.next().unwrap_or("");
            let key = percent_decode_str(key_raw)
                .decode_utf8()
                .map(|k| k.into_owned())
                .unwrap_or_else(|_| key_raw.to_string());
            let val = percent_decode_str(val_raw)
                .decode_utf8()
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| val_raw.to_string());
            if !key.is_empty() {
                params.insert(key, val);
            }
        }
        params
    }

    /// The HTTP method (GET, POST, etc.).
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The URI path component (e.g. `"/hello/world"`).
    pub fn path(&self) -> &str {
        self.uri.path()
    }

    /// Access a request header by name.
    ///
    /// Returns `None` if the header is not present.
    pub fn header(&self, name: &str) -> Option<&HeaderValue> {
        self.headers.get(name)
    }

    /// Get a decoded query-string parameter by name.
    ///
    /// Returns `None` if the parameter is not present.
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query_params.get(name).map(|s| s.as_str())
    }

    /// Get the raw, undecoded query string from the request URI (e.g. `"q=rust&page=2"`).
    ///
    /// Returns `None` if no query string is present.
    pub fn raw_query(&self) -> Option<&str> {
        self.uri.query()
    }

    /// Access all request headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Return the buffered request body bytes.
    ///
    /// This is a trivially async accessor; the body was fully read during
    /// request construction.
    pub async fn body_bytes(&self) -> &[u8] {
        &self.body
    }

    /// Consume the request and return its component parts.
    pub fn into_parts(self) -> (Method, Uri, HeaderMap, Bytes) {
        (self.method, self.uri, self.headers, self.body)
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
