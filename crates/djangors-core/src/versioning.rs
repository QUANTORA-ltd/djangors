//! API versioning extractors and deprecation response middleware.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::Full;
use hyper::http::{HeaderMap, HeaderName, HeaderValue};
use hyper::StatusCode;
use tower::layer::Layer;
use tower::Service;

use crate::error::DjangorsError;
use crate::extract::FromRequest;
use crate::request::Request;

/// Strategy for extracting the requested API version from an HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// Extract from the first path segment (e.g. `/v2/books` -> `"2"`).
    UrlPrefix,
    /// Extract from a request header (e.g. `Header("X-Api-Version")`).
    Header(&'static str),
    /// Extract from a parameter on the `Accept` media type
    /// (e.g. `MediaType("version")` for `Accept: application/json; version=2`).
    MediaType(&'static str),
}

/// Configuration for API version extraction and deprecation headers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Versioning {
    /// Where to read the requested version from.
    pub source: VersionSource,
    /// Optional fallback version when the client does not specify one.
    pub default_version: Option<String>,
    /// Supported version identifiers. When non-empty, any other version is rejected.
    pub supported: Vec<String>,
    /// Deprecated versions paired with an optional HTTP-date `Sunset` header value.
    pub deprecated: Vec<(String, Option<String>)>,
}

impl Versioning {
    /// Creates a new [`Versioning`] configuration using `source`.
    pub fn new(source: VersionSource) -> Self {
        Self {
            source,
            default_version: None,
            supported: Vec::new(),
            deprecated: Vec::new(),
        }
    }

    /// Sets the default version used when the request omits a version.
    pub fn default_version(mut self, version: impl Into<String>) -> Self {
        self.default_version = Some(version.into());
        self
    }

    /// Sets the list of supported version identifiers.
    pub fn supported(mut self, versions: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.supported = versions.into_iter().map(Into::into).collect();
        self
    }

    /// Marks `version` as deprecated, with an optional `Sunset` HTTP date string.
    pub fn deprecate(mut self, version: impl Into<String>, sunset: Option<&str>) -> Self {
        self.deprecated
            .push((version.into(), sunset.map(ToString::to_string)));
        self
    }

    fn extract_version(&self, path: &str, headers: &HeaderMap) -> Option<String> {
        let explicit = match self.source {
            VersionSource::UrlPrefix => extract_url_prefix_version(path),
            VersionSource::Header(name) => extract_header_version(headers, name),
            VersionSource::MediaType(param) => extract_media_type_version(headers, param),
        };
        explicit.or_else(|| self.default_version.clone())
    }

    fn is_supported(&self, version: &str) -> bool {
        self.supported.is_empty() || self.supported.iter().any(|s| s == version)
    }

    fn deprecation_info(&self, version: &str) -> Option<Option<&str>> {
        self.deprecated
            .iter()
            .find(|(v, _)| v == version)
            .map(|(_, sunset)| sunset.as_deref())
    }
}

fn extract_url_prefix_version(path: &str) -> Option<String> {
    let first = path.trim_start_matches('/').split('/').next()?;
    if first.is_empty() {
        return None;
    }
    // Only `v<digit>...` segments are versions (`v2`, `v2.1`, `v2024-01-01`). Anything else,
    // like `/books`, has no version in the URL, so the default version applies.
    let version = first
        .strip_prefix('v')
        .or_else(|| first.strip_prefix('V'))
        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))?;
    Some(version.to_string())
}

fn extract_header_version(headers: &HeaderMap, header_name: &str) -> Option<String> {
    let name = HeaderName::from_bytes(header_name.as_bytes()).ok()?;
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn extract_media_type_version(headers: &HeaderMap, param_name: &str) -> Option<String> {
    let accept = headers.get(hyper::header::ACCEPT)?.to_str().ok()?;
    for media_range in accept.split(',') {
        for part in media_range.split(';').skip(1) {
            if let Some((k, v)) = part.split_once('=') {
                if k.trim().eq_ignore_ascii_case(param_name) {
                    let cleaned = v.trim().trim_matches('"').trim();
                    if !cleaned.is_empty() {
                        return Some(cleaned.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Request extractor that resolves and validates the requested API version
/// against the [`Versioning`] configuration stored in router state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiVersion(pub String);

#[async_trait::async_trait]
impl FromRequest for ApiVersion {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let versioning = req.state::<Versioning>().ok_or_else(|| {
            DjangorsError::Internal("Versioning state is not configured".to_string())
        })?;

        let version = versioning
            .extract_version(req.path(), req.headers())
            .ok_or_else(|| {
                DjangorsError::api(
                    StatusCode::BAD_REQUEST,
                    "unsupported_version",
                    "API version is missing and no default version is configured",
                )
            })?;

        if !versioning.is_supported(&version) {
            return Err(DjangorsError::api(
                StatusCode::BAD_REQUEST,
                "unsupported_version",
                format!("API version '{version}' is not supported"),
            ));
        }

        Ok(ApiVersion(version))
    }
}

/// Tower layer that adds `Deprecation: true` and, when configured, `Sunset` (RFC 8594)
/// response headers when a request targets a deprecated API version.
///
/// `Deprecation: true` is the widely deployed draft form of the header; RFC 9745 later
/// changed its value to a date (`@<unix seconds>`). Clients that check for the header's
/// presence handle both.
#[derive(Debug, Clone)]
pub struct DeprecationLayer {
    versioning: Versioning,
}

impl DeprecationLayer {
    /// Creates a [`DeprecationLayer`] from a [`Versioning`] configuration.
    pub fn new(versioning: Versioning) -> Self {
        Self { versioning }
    }
}

impl From<Versioning> for DeprecationLayer {
    fn from(versioning: Versioning) -> Self {
        Self::new(versioning)
    }
}

impl<S> Layer<S> for DeprecationLayer {
    type Service = DeprecationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        DeprecationService {
            inner,
            versioning: self.versioning.clone(),
        }
    }
}

/// Tower service produced by [`DeprecationLayer`].
#[derive(Debug, Clone)]
pub struct DeprecationService<S> {
    inner: S,
    versioning: Versioning,
}

impl<S, B> Service<hyper::Request<B>> for DeprecationService<S>
where
    S: Service<hyper::Request<B>, Response = hyper::Response<Full<Bytes>>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = hyper::Response<Full<Bytes>>;
    type Error = Infallible;
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: hyper::Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let sunset = self
            .versioning
            .extract_version(req.uri().path(), req.headers())
            .and_then(|v| {
                self.versioning
                    .deprecation_info(&v)
                    .map(|s| s.map(ToString::to_string))
            });

        Box::pin(async move {
            let mut resp = inner.call(req).await?;
            if let Some(sunset_opt) = sunset {
                resp.headers_mut().insert(
                    HeaderName::from_static("deprecation"),
                    HeaderValue::from_static("true"),
                );
                if let Some(date) = sunset_opt {
                    if let Ok(val) = HeaderValue::from_str(&date) {
                        resp.headers_mut()
                            .insert(HeaderName::from_static("sunset"), val);
                    }
                }
            }
            Ok(resp)
        })
    }
}

#[cfg(test)]
#[path = "versioning_tests.rs"]
mod tests;
