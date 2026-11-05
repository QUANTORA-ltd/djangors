use std::convert::Infallible;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use tower::layer::Layer;
use tower::Service;

/// Logs each request's method, path, status, and latency via `tracing`.
/// Roughly equivalent to Django's request logging.
pub fn logging_layer() -> tower_http::trace::TraceLayer<tower_http::trace::HttpMakeClassifier> {
    tower_http::trace::TraceLayer::new_for_http()
}

/// Sets security-related response headers mimicking Django's `SecurityMiddleware`:
/// `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
/// and `Referrer-Policy: same-origin`.
#[derive(Clone)]
pub struct SecurityHeadersLayer;

impl<S> Layer<S> for SecurityHeadersLayer
where
    S: Service<
            hyper::Request<Incoming>,
            Response = hyper::Response<Full<Bytes>>,
            Error = Infallible,
        > + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Service = tower_http::set_header::SetResponseHeader<
        tower_http::set_header::SetResponseHeader<
            tower_http::set_header::SetResponseHeader<S, hyper::http::HeaderValue>,
            hyper::http::HeaderValue,
        >,
        hyper::http::HeaderValue,
    >;

    fn layer(&self, inner: S) -> Self::Service {
        tower_http::set_header::SetResponseHeaderLayer::overriding(
            hyper::http::HeaderName::from_static("x-content-type-options"),
            hyper::http::HeaderValue::from_static("nosniff"),
        )
        .layer(
            tower_http::set_header::SetResponseHeaderLayer::overriding(
                hyper::http::HeaderName::from_static("x-frame-options"),
                hyper::http::HeaderValue::from_static("DENY"),
            )
            .layer(
                tower_http::set_header::SetResponseHeaderLayer::overriding(
                    hyper::http::HeaderName::from_static("referrer-policy"),
                    hyper::http::HeaderValue::from_static("same-origin"),
                )
                .layer(inner),
            ),
        )
    }
}

/// Returns a [`SecurityHeadersLayer`].
pub fn security_headers_layer() -> SecurityHeadersLayer {
    SecurityHeadersLayer
}

/// Strict-Transport-Security (HSTS) header middleware.
#[derive(Clone)]
pub struct HstsLayer {
    max_age_seconds: u64,
    include_subdomains: bool,
}

impl HstsLayer {
    /// Creates a new `HstsLayer` with the given `max_age_seconds`.
    /// `include_subdomains` is disabled by default.
    pub fn new(max_age_seconds: u64) -> Self {
        Self {
            max_age_seconds,
            include_subdomains: false,
        }
    }

    /// Configures whether to include the `includeSubDomains` directive in the HSTS header.
    pub fn with_include_subdomains(mut self, yes: bool) -> Self {
        self.include_subdomains = yes;
        self
    }
}

impl<S> Layer<S> for HstsLayer {
    type Service = tower_http::set_header::SetResponseHeader<S, hyper::http::HeaderValue>;

    fn layer(&self, inner: S) -> Self::Service {
        let value_str = if self.include_subdomains {
            format!("max-age={}; includeSubDomains", self.max_age_seconds)
        } else {
            format!("max-age={}", self.max_age_seconds)
        };
        let header_val = hyper::http::HeaderValue::from_str(&value_str)
            .unwrap_or_else(|_| hyper::http::HeaderValue::from_static("max-age=0"));

        tower_http::set_header::SetResponseHeaderLayer::overriding(
            hyper::http::HeaderName::from_static("strict-transport-security"),
            header_val,
        )
        .layer(inner)
    }
}

/// Returns a [`HstsLayer`] with the given `max_age_seconds`.
pub fn hsts_layer(max_age_seconds: u64) -> HstsLayer {
    HstsLayer::new(max_age_seconds)
}

/// Builds a `Content-Security-Policy` (or `Content-Security-Policy-Report-Only`) header,
/// mirroring `django-csp`.
///
/// Directive values are passed through verbatim, so CSP keyword sources must be quoted
/// exactly as the spec requires (e.g. `"'self'"`, `"'unsafe-inline'"`, `"'nonce-abc123'"`) —
/// this builder only handles directive assembly and header formatting, not source-keyword
/// validation, matching `HstsLayer`'s minimal scope.
///
/// ```text
/// let csp = CspBuilder::new()
///     .default_src(&["'self'"])
///     .script_src(&["'self'", "https://cdn.example.com"])
///     .img_src(&["'self'", "data:"])
///     .build();
/// ```
#[derive(Clone, Default)]
pub struct CspBuilder {
    directives: Vec<(&'static str, Vec<String>)>,
    upgrade_insecure_requests: bool,
    report_only: bool,
}

impl CspBuilder {
    /// Creates an empty builder with no directives set.
    pub fn new() -> Self {
        Self::default()
    }

    fn directive(mut self, name: &'static str, sources: &[&str]) -> Self {
        self.directives
            .push((name, sources.iter().map(|s| s.to_string()).collect()));
        self
    }

    /// Sets the `default-src` directive (the fallback for any directive not set explicitly).
    pub fn default_src(self, sources: &[&str]) -> Self {
        self.directive("default-src", sources)
    }

    /// Sets the `script-src` directive.
    pub fn script_src(self, sources: &[&str]) -> Self {
        self.directive("script-src", sources)
    }

    /// Sets the `style-src` directive.
    pub fn style_src(self, sources: &[&str]) -> Self {
        self.directive("style-src", sources)
    }

    /// Sets the `img-src` directive.
    pub fn img_src(self, sources: &[&str]) -> Self {
        self.directive("img-src", sources)
    }

    /// Sets the `connect-src` directive.
    pub fn connect_src(self, sources: &[&str]) -> Self {
        self.directive("connect-src", sources)
    }

    /// Sets the `font-src` directive.
    pub fn font_src(self, sources: &[&str]) -> Self {
        self.directive("font-src", sources)
    }

    /// Sets the `frame-ancestors` directive (controls who may embed this site in a frame).
    pub fn frame_ancestors(self, sources: &[&str]) -> Self {
        self.directive("frame-ancestors", sources)
    }

    /// Sets the `form-action` directive (restricts where `<form>` submissions may target).
    pub fn form_action(self, sources: &[&str]) -> Self {
        self.directive("form-action", sources)
    }

    /// Sets the `object-src` directive (controls `<object>`/`<embed>`/`<applet>`).
    pub fn object_src(self, sources: &[&str]) -> Self {
        self.directive("object-src", sources)
    }

    /// Adds the `upgrade-insecure-requests` directive (no source list — it's a bare flag).
    pub fn upgrade_insecure_requests(mut self) -> Self {
        self.upgrade_insecure_requests = true;
        self
    }

    /// Sends the policy as `Content-Security-Policy-Report-Only` instead of enforcing it —
    /// useful for testing a policy's real-world impact before switching it on.
    pub fn report_only(mut self, yes: bool) -> Self {
        self.report_only = yes;
        self
    }

    fn header_value(&self) -> String {
        let mut parts: Vec<String> = self
            .directives
            .iter()
            .map(|(name, sources)| format!("{} {}", name, sources.join(" ")))
            .collect();
        if self.upgrade_insecure_requests {
            parts.push("upgrade-insecure-requests".to_string());
        }
        parts.join("; ")
    }

    /// Finalizes the builder into a [`CspLayer`].
    pub fn build(self) -> CspLayer {
        let header_name = if self.report_only {
            "content-security-policy-report-only"
        } else {
            "content-security-policy"
        };
        CspLayer {
            header_name,
            header_value: self.header_value(),
        }
    }
}

/// `Content-Security-Policy` header middleware. Built via [`CspBuilder`].
#[derive(Clone)]
pub struct CspLayer {
    header_name: &'static str,
    header_value: String,
}

impl<S> Layer<S> for CspLayer {
    type Service = tower_http::set_header::SetResponseHeader<S, hyper::http::HeaderValue>;

    fn layer(&self, inner: S) -> Self::Service {
        let header_val = hyper::http::HeaderValue::from_str(&self.header_value)
            .unwrap_or_else(|_| hyper::http::HeaderValue::from_static("default-src 'self'"));

        tower_http::set_header::SetResponseHeaderLayer::overriding(
            hyper::http::HeaderName::from_static(self.header_name),
            header_val,
        )
        .layer(inner)
    }
}

/// Normalizes request paths by trimming trailing slashes.
/// Roughly equivalent to Django's `CommonMiddleware` trailing-slash behavior.
pub fn normalize_path_layer() -> tower_http::normalize_path::NormalizePathLayer {
    tower_http::normalize_path::NormalizePathLayer::trim_trailing_slash()
}

/// Compresses response bodies using gzip and brotli encoding.
pub fn compression_layer() -> tower_http::compression::CompressionLayer {
    tower_http::compression::CompressionLayer::new()
}

/// Sets and propagates the `X-Request-ID` header on every request/response using a
/// UUID generator. Roughly equivalent to how Django's `CommonMiddleware` or system
/// request-id middleware works.
#[derive(Clone)]
pub struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer
where
    S: Service<
            hyper::Request<Incoming>,
            Response = hyper::Response<Full<Bytes>>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Service = tower_http::request_id::SetRequestId<
        tower_http::request_id::PropagateRequestId<S>,
        tower_http::request_id::MakeRequestUuid,
    >;

    fn layer(&self, inner: S) -> Self::Service {
        let header = hyper::http::HeaderName::from_static("x-request-id");
        tower_http::request_id::SetRequestIdLayer::new(
            header.clone(),
            tower_http::request_id::MakeRequestUuid,
        )
        .layer(tower_http::request_id::PropagateRequestIdLayer::new(header).layer(inner))
    }
}

/// Returns a [`RequestIdLayer`].
pub fn request_id_layer() -> RequestIdLayer {
    RequestIdLayer
}

/// Extension containing the validated or generated CSRF token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsrfToken(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CsrfPendingFormCheck(pub String);

/// CSRF protection middleware implementing a double-submit cookie scheme.
///
/// On an unsafe-method request (POST/PUT/PATCH/DELETE), this layer first checks the custom HTTP
/// header (default: `X-CSRFToken`) against the `csrftoken` cookie using a constant-time
/// comparison. If the header is missing or doesn't match, the request is **not** rejected
/// immediately — instead a pending-check marker is attached and the request falls through to the
/// framework's own request dispatch, which (for `application/x-www-form-urlencoded` bodies) checks
/// a `csrfmiddlewaretoken` form field against the same pending token before allowing the request
/// through. This means both JavaScript-set-header submissions and classic HTML `<form>`
/// submissions (with a hidden `csrfmiddlewaretoken` input) are protected.
#[derive(Clone)]
pub struct CsrfLayer {
    cookie_name: String,
    header_name: String,
    secure: bool,
}

impl CsrfLayer {
    /// Creates a new `CsrfLayer` with default settings:
    /// - Cookie name: `csrftoken`
    /// - Header name: `x-csrftoken`
    pub fn new() -> Self {
        Self {
            cookie_name: "csrftoken".to_string(),
            header_name: "x-csrftoken".to_string(),
            secure: false,
        }
    }

    /// Sets a custom cookie name.
    pub fn with_cookie_name(mut self, name: String) -> Self {
        self.cookie_name = name;
        self
    }

    /// Sets a custom header name.
    pub fn with_header_name(mut self, name: String) -> Self {
        self.header_name = name;
        self
    }

    /// Sets the Secure attribute on the cookie.
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }
}

impl Default for CsrfLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns a default [`CsrfLayer`].
pub fn csrf_layer() -> CsrfLayer {
    CsrfLayer::new()
}

impl<S> Layer<S> for CsrfLayer {
    type Service = CsrfService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CsrfService {
            inner,
            cookie_name: self.cookie_name.clone(),
            header_name: self.header_name.clone(),
            secure: self.secure,
        }
    }
}

/// Service middleware handling CSRF token validation and cookie setting.
#[derive(Clone)]
pub struct CsrfService<S> {
    inner: S,
    cookie_name: String,
    header_name: String,
    secure: bool,
}

fn extract_cookie(headers: &hyper::HeaderMap, cookie_name: &str) -> Option<String> {
    for cookie_header in headers.get_all(hyper::header::COOKIE) {
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

fn generate_csrf_token() -> String {
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

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

impl<S, B> Service<hyper::Request<B>> for CsrfService<S>
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
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let cookie_name = self.cookie_name.clone();
        let header_name = self.header_name.clone();
        let secure = self.secure;

        let cookie_val = extract_cookie(req.headers(), &cookie_name);
        let mut new_token_generated = false;
        let token_value = match cookie_val {
            Some(val) if !val.is_empty() => val,
            _ => {
                new_token_generated = true;
                generate_csrf_token()
            }
        };

        let method = req.method();
        let is_unsafe = method == hyper::Method::POST
            || method == hyper::Method::PUT
            || method == hyper::Method::PATCH
            || method == hyper::Method::DELETE;

        if is_unsafe {
            let mut is_valid = false;
            if let Some(header_val) = req.headers().get(&header_name) {
                if constant_time_eq(header_val.as_bytes(), token_value.as_bytes()) {
                    is_valid = true;
                }
            }
            if !is_valid {
                req.extensions_mut()
                    .insert(CsrfPendingFormCheck(token_value.clone()));
            }
        }

        req.extensions_mut().insert(CsrfToken(token_value.clone()));

        Box::pin(async move {
            let mut resp = inner.call(req).await?;

            if new_token_generated {
                // Deliberately not setting HttpOnly (unlike the session cookie):
                // the token must be readable by client-side JS to populate
                // the X-CSRFToken header on AJAX requests, matching Django's
                // own default behavior.
                let mut set_cookie_val = format!(
                    "{}={}; Path=/; SameSite=Lax; Max-Age=31536000",
                    cookie_name, token_value
                );
                if secure {
                    set_cookie_val.push_str("; Secure");
                }
                if let Ok(hdr_val) = hyper::header::HeaderValue::from_str(&set_cookie_val) {
                    resp.headers_mut()
                        .append(hyper::header::SET_COOKIE, hdr_val);
                }
            }

            Ok(resp)
        })
    }
}

/// Host header validation middleware.
/// Checks the incoming request's `Host` header (or URI authority if absent) against an allowed list of hosts.
#[derive(Clone)]
pub struct HostValidationLayer {
    allowed_hosts: Vec<String>,
}

impl HostValidationLayer {
    /// Creates a new `HostValidationLayer` with the given allowed hosts.
    /// Hosts are lowercased at construction time.
    pub fn new(allowed_hosts: Vec<String>) -> Self {
        let allowed_hosts = allowed_hosts
            .into_iter()
            .map(|h| h.to_lowercase())
            .collect();
        Self { allowed_hosts }
    }
}

impl<S> Layer<S> for HostValidationLayer {
    type Service = HostValidationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        HostValidationService {
            inner,
            allowed_hosts: self.allowed_hosts.clone(),
        }
    }
}

/// Service for `HostValidationLayer`.
#[derive(Clone)]
pub struct HostValidationService<S> {
    inner: S,
    allowed_hosts: Vec<String>,
}

impl<S, B> Service<hyper::Request<B>> for HostValidationService<S>
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
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: hyper::Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let allowed_hosts = self.allowed_hosts.clone();

        let host_str = match req.headers().get(hyper::header::HOST) {
            Some(hdr) => hdr.to_str().ok(),
            None => req.uri().authority().map(|a| a.as_str()),
        };

        // If allowed_hosts is empty, unrestricted (always valid)
        let is_valid = if allowed_hosts.is_empty() {
            true
        } else if let Some(h) = host_str {
            // Strip trailing port if present
            let host_without_port = if h.starts_with('[') {
                if let Some(close_bracket_idx) = h.find(']') {
                    &h[..=close_bracket_idx]
                } else {
                    h
                }
            } else {
                h.split(':').next().unwrap_or(h)
            };
            let host_lower = host_without_port.to_lowercase();
            allowed_hosts.iter().any(|allowed| allowed == &host_lower)
        } else {
            false
        };

        if !is_valid {
            let response = hyper::Response::builder()
                .status(400)
                .body(Full::new(Bytes::from("400 Bad Request: Disallowed Host")))
                .unwrap();
            return Box::pin(async move { Ok(response) });
        }

        Box::pin(async move {
            let resp = inner.call(req).await?;
            Ok(resp)
        })
    }
}

#[cfg(test)]
#[path = "middleware_tests.rs"]
mod tests;
