use bytes::Bytes;
use hyper::http::header::{CONTENT_TYPE, LOCATION};
use hyper::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
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
    body: Bytes,
}

impl Response {
    fn new(status: StatusCode) -> Self {
        Response {
            status,
            headers: HeaderMap::new(),
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
    /// Sets `Content-Type: text/html; charset=utf-8`.
    pub fn html(status: StatusCode, body: impl Into<String>) -> Self {
        let s = body.into();
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

    /// Convert this response into a hyper [`Response`](hyper::Response) backed by
    /// a [`Full`](http_body_util::Full) body.
    pub fn into_hyper(self) -> hyper::Response<http_body_util::Full<Bytes>> {
        let mut res = hyper::Response::new(http_body_util::Full::new(self.body));
        *res.status_mut() = self.status;
        *res.headers_mut() = self.headers;
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

#[cfg(test)]
#[path = "response_tests.rs"]
mod tests;
