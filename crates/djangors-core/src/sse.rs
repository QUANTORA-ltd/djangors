use std::pin::Pin;

use bytes::Bytes;
use futures_util::stream::Stream;
use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::http::header::{CACHE_CONTROL, CONNECTION, CONTENT_TYPE};
use hyper::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

/// A typed Server-Sent Events (SSE) frame supporting `event:`, `id:`, `retry:`, and multi-line `data:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// Event payload (`data:` lines).
    pub data: String,
    /// Optional SSE event name (`event:` line).
    pub event: Option<String>,
    /// Optional SSE event ID (`id:` line).
    pub id: Option<String>,
    /// Optional client reconnect delay in milliseconds (`retry:` line).
    pub retry_ms: Option<u64>,
}

impl SseEvent {
    /// Create a new SSE frame carrying `data`.
    pub fn data(data: impl Into<String>) -> Self {
        Self {
            data: data.into(),
            event: None,
            id: None,
            retry_ms: None,
        }
    }

    /// Set the `event:` name on this SSE frame.
    pub fn event(mut self, name: impl Into<String>) -> Self {
        self.event = Some(name.into());
        self
    }

    /// Set the `id:` field on this SSE frame.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the `retry:` reconnect interval in milliseconds on this SSE frame.
    pub fn retry_ms(mut self, ms: u64) -> Self {
        self.retry_ms = Some(ms);
        self
    }

    /// Serialize this frame into wire-format SSE text ending with `\n\n`.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        if let Some(id) = &self.id {
            out.push_str(&format!("id: {id}\n"));
        }
        if let Some(event) = &self.event {
            out.push_str(&format!("event: {event}\n"));
        }
        if let Some(retry) = self.retry_ms {
            out.push_str(&format!("retry: {retry}\n"));
        }
        for line in self.data.split('\n') {
            out.push_str(&format!("data: {line}\n"));
        }
        out.push('\n');
        out
    }
}

/// A streaming HTTP response backed by an async [`Stream`] of [`Bytes`] chunks.
///
/// Constructed via [`StreamingResponse::new`] or [`StreamingResponse::sse`].
pub struct StreamingResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) stream:
        Pin<Box<dyn Stream<Item = Result<Bytes, std::convert::Infallible>> + Send + Sync>>,
}

impl StreamingResponse {
    /// Create a new generic streaming response.
    pub fn new<S>(status: StatusCode, stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, std::convert::Infallible>> + Send + Sync + 'static,
    {
        Self {
            status,
            headers: HeaderMap::new(),
            stream: Box::pin(stream),
        }
    }

    /// Create a Server-Sent Events (SSE) streaming response from a stream of strings.
    ///
    /// Sets default SSE headers:
    /// - `Content-Type: text/event-stream`
    /// - `Cache-Control: no-cache`
    /// - `Connection: keep-alive`
    ///
    /// Formats each stream item as an SSE data frame (`data: {item}\n\n`).
    pub fn sse<S>(stream: S) -> Self
    where
        S: Stream<Item = String> + Send + Sync + 'static,
    {
        let sse_stream = stream.map(|item| {
            let chunk = format!("data: {item}\n\n");
            Ok::<Bytes, std::convert::Infallible>(Bytes::from(chunk))
        });
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));

        Self {
            status: StatusCode::OK,
            headers,
            stream: Box::pin(sse_stream),
        }
    }

    /// Create an SSE streaming response from a stream of structured [`SseEvent`] frames.
    pub fn sse_events<S>(stream: S) -> Self
    where
        S: Stream<Item = SseEvent> + Send + Sync + 'static,
    {
        let sse_stream =
            stream.map(|ev| Ok::<Bytes, std::convert::Infallible>(Bytes::from(ev.encode())));
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
        Self {
            status: StatusCode::OK,
            headers,
            stream: Box::pin(sse_stream),
        }
    }

    /// Create an SSE streaming response where each item `T: Serialize` is encoded as a JSON `data:` frame.
    pub fn sse_json<T, S>(stream: S) -> Self
    where
        T: serde::Serialize + Send + 'static,
        S: Stream<Item = T> + Send + Sync + 'static,
    {
        let event_stream = stream.filter_map(|item| {
            std::future::ready(serde_json::to_string(&item).ok().map(SseEvent::data))
        });
        Self::sse_events(event_stream)
    }

    /// Create an AI/LLM token streaming SSE response (`data: {"token": ...}\n\n` followed by `data: [DONE]\n\n`).
    ///
    /// Sets `X-Accel-Buffering: no` so reverse proxies (nginx, Envoy) flush tokens immediately.
    pub fn llm_stream<S>(token_stream: S) -> Self
    where
        S: Stream<Item = String> + Send + Sync + 'static,
    {
        let chunks = token_stream
            .map(|tok| {
                let payload = serde_json::json!({ "token": tok }).to_string();
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n")))
            })
            .chain(futures_util::stream::once(async {
                Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(b"data: [DONE]\n\n"))
            }));
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
        headers.insert(
            HeaderName::from_static("x-accel-buffering"),
            HeaderValue::from_static("no"),
        );
        Self {
            status: StatusCode::OK,
            headers,
            stream: Box::pin(chunks),
        }
    }

    /// Add a response header (builder pattern).
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

    /// The HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Convert this streaming response into a hyper [`Response`](hyper::Response) backed by
    /// a [`BoxBody`].
    pub fn into_hyper(self) -> hyper::Response<BoxBody<Bytes, std::convert::Infallible>> {
        let mapped_stream = self.stream.map(|res| res.map(hyper::body::Frame::data));
        let body = BodyExt::boxed(http_body_util::StreamBody::new(mapped_stream));
        let mut res = hyper::Response::new(body);
        *res.status_mut() = self.status;
        *res.headers_mut() = self.headers;
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[test]
    fn sse_event_encodes_all_fields() {
        let ev = SseEvent::data("line1\nline2")
            .event("delta")
            .id("42")
            .retry_ms(1500);
        assert_eq!(
            ev.encode(),
            "id: 42\nevent: delta\nretry: 1500\ndata: line1\ndata: line2\n\n"
        );
    }

    #[tokio::test]
    async fn llm_stream_emits_tokens_and_done_marker() {
        let tokens = futures_util::stream::iter(vec!["Hello".to_string(), " world".to_string()]);
        let resp = StreamingResponse::llm_stream(tokens);
        assert_eq!(resp.headers().get("x-accel-buffering").unwrap(), "no");
        let hyper_resp = resp.into_hyper();
        let bytes = hyper_resp.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains(r#"data: {"token":"Hello"}"#));
        assert!(text.contains(r#"data: {"token":" world"}"#));
        assert!(text.ends_with("data: [DONE]\n\n"));
    }
}
