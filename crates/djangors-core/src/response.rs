use bytes::Bytes;
use hyper::http::header::{CONTENT_TYPE, LOCATION};
use hyper::http::{Extensions, HeaderMap, HeaderName, HeaderValue, StatusCode};
use serde::Serialize;

use crate::error::DjangorsError;

/// An HTTP response with a fully buffered body.
///
/// Constructed via builder-style helpers and then converted into a hyper
/// response with [`Response::into_hyper`].
#[derive(Debug, Clone)]
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    extensions: Extensions,
    body: Bytes,
}

impl Response {
    /// Creates an empty [`Response`] with the given HTTP status code.
    pub fn new(status: StatusCode) -> Self {
        Response {
            status,
            headers: HeaderMap::new(),
            extensions: Extensions::new(),
            body: Bytes::new(),
        }
    }

    /// Create a plain-text response.
    ///
    /// Sets `Content-Type: text/plain; charset=utf-8`.
    pub fn text(status: StatusCode, body: &str) -> Self {
        let mut res = Response::new(status);
        res.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        res.body = Bytes::copy_from_slice(body.as_bytes());
        res
    }

    /// Create a response from raw bytes with a specified Content-Type.
    pub fn bytes(status: StatusCode, content_type: &str, body: Vec<u8>) -> Self {
        let mut res = Response::new(status);
        if let Ok(val) = HeaderValue::from_str(content_type) {
            res.headers.insert(CONTENT_TYPE, val);
        }
        res.body = Bytes::from(body);
        res
    }

    /// Create an HTML response.
    ///
    /// Sets `Content-Type: text/html; charset=utf-8`. When `DJANGORS_LIVERELOAD_PORT`
    /// is set in the environment (by `dj run`), a small live-reload script is inserted
    /// before `</body>`. Fragments without `</body>` (htmx partials, snippets) are left
    /// untouched, so swapping them in never opens extra sockets.
    pub fn html(status: StatusCode, body: impl Into<String>) -> Self {
        let mut s = body.into();
        if let Some(snippet) = live_reload_snippet() {
            if !s.contains("__djangors_livereload") {
                if let Some(idx) = s.rfind("</body>") {
                    s.insert_str(idx, snippet);
                }
            }
        }
        let mut res = Response::new(status);
        res.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        res.body = Bytes::copy_from_slice(s.as_bytes());
        res
    }

    /// Create a JSON response from pre-serialized UTF-8 JSON bytes without copying.
    ///
    /// Sets `Content-Type: application/json; charset=utf-8`.
    pub fn json_bytes(status: StatusCode, body: Vec<u8>) -> Self {
        let mut res = Response::new(status);
        res.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        res.body = Bytes::from(body);
        res
    }

    /// Create an RFC 9457 Problem Details response from pre-serialized UTF-8 JSON bytes.
    ///
    /// Sets `Content-Type: application/problem+json; charset=utf-8`.
    pub fn problem_json_bytes(status: StatusCode, body: Vec<u8>) -> Self {
        let mut res = Response::new(status);
        res.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json; charset=utf-8"),
        );
        res.body = Bytes::from(body);
        res
    }

    /// Create a JSON response from a serializable value.
    ///
    /// Sets `Content-Type: application/json; charset=utf-8`. Returns
    /// `DjangorsError::Internal` if serialization fails.
    pub fn json<T: Serialize>(status: StatusCode, value: &T) -> Result<Self, DjangorsError> {
        let bytes =
            serde_json::to_vec(value).map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(Self::json_bytes(status, bytes))
    }

    /// Create a 302 Found redirect response.
    ///
    /// Sets the `Location` header to the given URI.
    pub fn redirect(location: &str) -> Self {
        let mut res = Response::new(StatusCode::FOUND);
        res.headers.insert(
            LOCATION,
            HeaderValue::from_str(location)
                .expect("redirect location must be a valid header value"),
        );
        res
    }

    /// Add a response header (builder pattern).
    ///
    /// Overwrites any existing value for the same header name.
    ///
    /// # Panics
    /// Panics if `name` or `value` contain invalid characters for HTTP headers.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).expect("header name must be valid HTTP token");
        let header_value = HeaderValue::from_str(value).expect("header value must be valid ASCII");
        self.headers.insert(header_name, header_value);
        self
    }

    fn htmx_header(mut self, name: &'static str, value: &str) -> Self {
        match HeaderValue::from_str(value) {
            Ok(value) => { self.headers.insert(HeaderName::from_static(name), value); }
            Err(_) => tracing::warn!(header = name, "skipping invalid htmx response header value"),
        }
        self
    }

    /// Adds the `HX-Trigger` response header.
    pub fn hx_trigger(self, event: &str) -> Self { self.htmx_header("hx-trigger", event) }
    /// Adds the `HX-Redirect` response header.
    pub fn hx_redirect(self, url: &str) -> Self { self.htmx_header("hx-redirect", url) }
    /// Adds `HX-Refresh: true`.
    pub fn hx_refresh(self) -> Self { self.htmx_header("hx-refresh", "true") }
    /// Adds the `HX-Push-Url` response header.
    pub fn hx_push_url(self, url: &str) -> Self { self.htmx_header("hx-push-url", url) }
    /// Adds the `HX-Retarget` response header.
    pub fn hx_retarget(self, selector: &str) -> Self { self.htmx_header("hx-retarget", selector) }
    /// Adds the `HX-Reswap` response header.
    pub fn hx_reswap(self, strategy: &str) -> Self { self.htmx_header("hx-reswap", strategy) }

    /// Attach a typed extension to this response (builder pattern).
    ///
    /// Extensions are preserved when converting into a `hyper::Response` via
    /// [`Response::into_hyper`] or [`Response::into_hyper_boxed`], allowing
    /// outer Tower middleware (such as `djangors_cache::CacheLayer` checking
    /// for `CacheableResponse`) to inspect markers set by a view handler.
    pub fn with_extension<T: Clone + Send + Sync + 'static>(mut self, val: T) -> Self {
        self.extensions.insert(val);
        self
    }

    /// Mark this response as safe to cache by `djangors_cache::CacheLayer`
    /// (builder pattern).
    ///
    /// Equivalent to `.with_extension(djangors_cache::CacheableResponse)`.
    /// Responses carrying a `Set-Cookie` header are still never cached by
    /// `CacheLayer`.
    #[cfg(feature = "cache")]
    pub fn cacheable(self) -> Self {
        self.with_extension(djangors_cache::CacheableResponse)
    }

    /// Convert this response into a hyper [`Response`](hyper::Response) backed by
    /// a [`Full`](http_body_util::Full) body.
    pub fn into_hyper(self) -> hyper::Response<http_body_util::Full<Bytes>> {
        let mut res = hyper::Response::new(http_body_util::Full::new(self.body));
        *res.status_mut() = self.status;
        *res.headers_mut() = self.headers;
        *res.extensions_mut() = self.extensions;
        res
    }

    /// Convert this response into a hyper [`Response`](hyper::Response) backed by
    /// a [`BoxBody`](http_body_util::combinators::BoxBody).
    pub fn into_hyper_boxed(
        self,
    ) -> hyper::Response<http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>>
    {
        use http_body_util::BodyExt;
        let full =
            http_body_util::Full::new(self.body).map_err(|e: std::convert::Infallible| match e {});
        let mut res = hyper::Response::new(full.boxed());
        *res.status_mut() = self.status;
        *res.headers_mut() = self.headers;
        *res.extensions_mut() = self.extensions;
        res
    }

    /// Create an SSE streaming response from a stream of strings.
    pub fn sse<S>(stream: S) -> crate::sse::StreamingResponse
    where
        S: futures_util::stream::Stream<Item = String> + Send + Sync + 'static,
    {
        crate::sse::StreamingResponse::sse(stream)
    }

    /// The HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The response extensions.
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Mutable access to the response extensions.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// The response body as bytes.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl From<Response> for hyper::Response<http_body_util::Full<Bytes>> {
    fn from(resp: Response) -> Self {
        resp.into_hyper()
    }
}

/// The live-reload `<script>` for `dj run`, built once from `DJANGORS_LIVERELOAD_PORT`.
fn live_reload_snippet() -> Option<&'static str> {
    static SNIPPET: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SNIPPET
        .get_or_init(|| {
            let port: u16 = std::env::var("DJANGORS_LIVERELOAD_PORT")
                .ok()?
                .trim()
                .parse()
                .ok()?;
            Some(format!(
                "<script id=\"__djangors_livereload\">(function(){{\
                 var ws=new WebSocket('ws://'+location.hostname+':{port}/__djangors/live-reload');\
                 ws.onmessage=function(e){{if(e.data==='reload')window.location.reload();}};\
                 ws.onclose=function(){{setTimeout(function(){{window.location.reload();}},600);}};\
                 }})();</script>"
            ))
        })
        .as_deref()
}

#[cfg(test)]
#[path = "response_tests.rs"]
mod tests;
