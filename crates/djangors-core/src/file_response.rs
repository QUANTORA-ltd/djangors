//! File and byte-range HTTP responses (`FileResponse`).
//!
//! Mirrors Django's `FileResponse` with automatic MIME type detection,
//! `Content-Disposition` (`inline` or `attachment`), `Accept-Ranges: bytes`,
//! and HTTP `Range: bytes=start-end` (`206 Partial Content` / `416 Range Not Satisfiable`)
//! support.

use std::path::Path;

use bytes::Bytes;
use futures_util::stream;
use hyper::http::StatusCode;

use crate::error::DjangorsError;
use crate::request::Request;
use crate::response::Response;
use crate::sse::StreamingResponse;

/// Controls how the browser should present a [`FileResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentDisposition {
    /// Display the file inline in the browser if supported (`Content-Disposition: inline`).
    Inline,
    /// Prompt the user to download/save the file (`Content-Disposition: attachment`).
    Attachment,
}

/// An HTTP file response supporting MIME detection, `Content-Disposition`,
/// HTTP byte ranges (`206 Partial Content` / `416 Range Not Satisfiable`),
/// and chunked streaming.
#[derive(Debug, Clone)]
pub struct FileResponse {
    data: Vec<u8>,
    content_type: String,
    filename: Option<String>,
    disposition: ContentDisposition,
}

impl FileResponse {
    /// Creates a [`FileResponse`] by reading a file from `path`.
    ///
    /// Automatically detects the MIME type and filename from `path`.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DjangorsError> {
        let path = path.as_ref();
        let data = std::fs::read(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DjangorsError::NotFound
            } else {
                DjangorsError::Internal(format!("failed to read file {}: {e}", path.display()))
            }
        })?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
        let content_type = detect_mime_type(path).to_string();
        Ok(Self {
            data,
            content_type,
            filename,
            disposition: ContentDisposition::Inline,
        })
    }

    /// Creates a [`FileResponse`] from in-memory bytes.
    ///
    /// Defaults to `application/octet-stream` and `ContentDisposition::Inline`.
    /// Setting a [`filename`](Self::filename) will also update the MIME type if
    /// no explicit [`content_type`](Self::content_type) was set.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            content_type: "application/octet-stream".to_string(),
            filename: None,
            disposition: ContentDisposition::Inline,
        }
    }

    /// Sets the filename used in `Content-Disposition`.
    ///
    /// If the current content type is still the default `application/octet-stream`,
    /// this also infers the MIME type from the filename's extension.
    pub fn filename(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if self.content_type == "application/octet-stream" {
            self.content_type = detect_mime_type(Path::new(&name)).to_string();
        }
        self.filename = Some(name);
        self
    }

    /// Explicitly overrides the `Content-Type` header.
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = content_type.into();
        self
    }

    /// Sets `Content-Disposition` to `attachment` when `true`, or `inline` when `false`.
    pub fn as_attachment(mut self, attachment: bool) -> Self {
        self.disposition = if attachment {
            ContentDisposition::Attachment
        } else {
            ContentDisposition::Inline
        };
        self
    }

    /// Sets the [`ContentDisposition`] mode.
    pub fn disposition(mut self, disposition: ContentDisposition) -> Self {
        self.disposition = disposition;
        self
    }

    /// Returns the detected or configured MIME type.
    pub fn mime_type(&self) -> &str {
        &self.content_type
    }

    /// Returns the total byte length of the file payload.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the file payload is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn content_disposition_header(&self) -> String {
        let kind = match self.disposition {
            ContentDisposition::Inline => "inline",
            ContentDisposition::Attachment => "attachment",
        };
        match &self.filename {
            Some(name) => {
                let safe: String = name
                    .chars()
                    .filter(|&c| c != '\r' && c != '\n' && c != '"' && c != '\\')
                    .collect();
                format!("{kind}; filename=\"{safe}\"")
            }
            None => kind.to_string(),
        }
    }

    /// Builds a `200 OK` [`Response`] with full contents, `Content-Type`,
    /// `Content-Disposition`, `Accept-Ranges: bytes`, and `Content-Length`.
    pub fn into_response(self) -> Response {
        let disp = self.content_disposition_header();
        let len = self.data.len().to_string();
        Response::bytes(StatusCode::OK, &self.content_type, self.data)
            .header("content-disposition", &disp)
            .header("accept-ranges", "bytes")
            .header("content-length", &len)
    }

    /// Builds a [`Response`] honouring the incoming request's `Range` header if present.
    ///
    /// - Without a `Range` header: returns `200 OK` with full body.
    /// - With a valid, satisfiable `Range: bytes=start-end` header: returns `206 Partial Content`
    ///   with `Content-Range: bytes start-end/total`.
    /// - With an unsatisfiable or invalid `Range` header: returns `416 Range Not Satisfiable`
    ///   with `Content-Range: bytes */total`.
    pub fn into_response_for_request(self, req: &Request) -> Response {
        let range_header = match req.header("range").and_then(|v| v.to_str().ok()) {
            Some(h) => h,
            None => return self.into_response(),
        };

        let total = self.data.len();
        let disp = self.content_disposition_header();

        match parse_byte_range(range_header, total) {
            Some((start, end)) => {
                let slice = self.data[start..=end].to_vec();
                let content_len = slice.len().to_string();
                let content_range = format!("bytes {start}-{end}/{total}");
                Response::bytes(StatusCode::PARTIAL_CONTENT, &self.content_type, slice)
                    .header("content-disposition", &disp)
                    .header("accept-ranges", "bytes")
                    .header("content-range", &content_range)
                    .header("content-length", &content_len)
            }
            None => Response::new(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("content-range", &format!("bytes */{total}"))
                .header("accept-ranges", "bytes"),
        }
    }

    /// Converts this [`FileResponse`] into a chunked [`StreamingResponse`].
    ///
    /// `chunk_size` defaults to 64 KiB if `0` is passed.
    pub fn into_streaming_response(self, chunk_size: usize) -> StreamingResponse {
        let chunk_size = if chunk_size == 0 {
            64 * 1024
        } else {
            chunk_size
        };
        let disp = self.content_disposition_header();
        let len = self.data.len().to_string();
        let content_type = self.content_type;

        let chunks: Vec<Result<Bytes, std::convert::Infallible>> = self
            .data
            .chunks(chunk_size)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();

        StreamingResponse::new(StatusCode::OK, stream::iter(chunks))
            .header("content-type", &content_type)
            .header("content-disposition", &disp)
            .header("accept-ranges", "bytes")
            .header("content-length", &len)
    }
}

/// Detects an HTTP MIME type from a file path's extension.
pub fn detect_mime_type(path: &Path) -> &'static str {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_ascii_lowercase(),
        None => return "application/octet-stream",
    };
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" | "log" => "text/plain; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Parses a single HTTP `Range: bytes=...` header into an inclusive `(start, end)` pair.
fn parse_byte_range(header: &str, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    // Only single-range requests are supported; multi-range returns None.
    if spec.contains(',') {
        return None;
    }
    let (start_str, end_str) = spec.split_once('-')?;
    let start_str = start_str.trim();
    let end_str = end_str.trim();

    if start_str.is_empty() {
        // Suffix byte range: bytes=-N
        let suffix: usize = end_str.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        Some((start, total - 1))
    } else {
        let start: usize = start_str.parse().ok()?;
        if start >= total {
            return None;
        }
        let end = if end_str.is_empty() {
            total - 1
        } else {
            let parsed_end: usize = end_str.parse().ok()?;
            if parsed_end < start {
                return None;
            }
            parsed_end.min(total - 1)
        };
        Some((start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::http::{HeaderMap, HeaderValue, Method, Uri};

    #[test]
    fn mime_detection_covers_common_extensions() {
        assert_eq!(detect_mime_type(Path::new("report.pdf")), "application/pdf");
        assert_eq!(detect_mime_type(Path::new("photo.PNG")), "image/png");
        assert_eq!(detect_mime_type(Path::new("data.json")), "application/json");
        assert_eq!(
            detect_mime_type(Path::new("unknown.xyz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn full_response_sets_disposition_and_headers() {
        let resp = FileResponse::from_bytes(b"hello world".to_vec())
            .filename("greeting.txt")
            .as_attachment(true)
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            resp.headers().get("content-disposition").unwrap(),
            "attachment; filename=\"greeting.txt\""
        );
        assert_eq!(resp.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(resp.headers().get("content-length").unwrap(), "11");
        assert_eq!(resp.body(), b"hello world");
    }

    #[test]
    fn range_request_returns_206_and_416() {
        let make_req = |range: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("range", HeaderValue::from_str(range).unwrap());
            Request::new(
                Method::GET,
                Uri::from_static("/file"),
                headers,
                Bytes::new(),
            )
        };

        // Explicit start-end
        let resp = FileResponse::from_bytes(b"0123456789".to_vec())
            .into_response_for_request(&make_req("bytes=2-5"));
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(resp.headers().get("content-range").unwrap(), "bytes 2-5/10");
        assert_eq!(resp.body(), b"2345");

        // Open-ended start-
        let resp = FileResponse::from_bytes(b"0123456789".to_vec())
            .into_response_for_request(&make_req("bytes=7-"));
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(resp.headers().get("content-range").unwrap(), "bytes 7-9/10");
        assert_eq!(resp.body(), b"789");

        // Suffix -N
        let resp = FileResponse::from_bytes(b"0123456789".to_vec())
            .into_response_for_request(&make_req("bytes=-3"));
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(resp.headers().get("content-range").unwrap(), "bytes 7-9/10");
        assert_eq!(resp.body(), b"789");

        // Unsatisfiable range -> 416
        let resp = FileResponse::from_bytes(b"0123456789".to_vec())
            .into_response_for_request(&make_req("bytes=50-60"));
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(resp.headers().get("content-range").unwrap(), "bytes */10");
        assert!(resp.body().is_empty());
    }

    #[test]
    fn from_path_reads_disk_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("djangors_file_resp_{}.csv", std::process::id()));
        std::fs::write(&path, b"a,b\n1,2\n").unwrap();
        let resp = FileResponse::from_path(&path).unwrap().into_response();
        let _ = std::fs::remove_file(&path);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/csv; charset=utf-8"
        );
        assert_eq!(resp.body(), b"a,b\n1,2\n");
    }
}
