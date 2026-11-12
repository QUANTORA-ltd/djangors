use std::fmt;

use hyper::StatusCode;

use crate::request::Request;
use crate::response::Response;

/// A project-supplied override for how [`DjangorsError`] values are rendered
/// into HTTP responses.
pub trait ErrorRenderer: Send + Sync {
    /// Render an error for the given request.
    fn render(&self, err: &DjangorsError, req: &Request) -> Response;
}

/// A JSON error renderer suitable for JSON APIs.
pub struct JsonErrorRenderer;

impl ErrorRenderer for JsonErrorRenderer {
    fn render(&self, err: &DjangorsError, _req: &Request) -> Response {
        err.into_json_response()
    }
}

/// An [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) Problem Details
/// (`application/problem+json`) error renderer.
///
/// Renders any [`DjangorsError`] (across REST ViewSets, form validation, and
/// authentication failures) as a single RFC 9457 Problem Details JSON object
/// using Djangors's direct JSON byte writer (`write_json_str` / `write_json_i64`),
/// including the request path as `instance` and any field/validation map under
/// `errors` (plus `details` for compatibility).
#[derive(Debug, Clone, Default)]
pub struct ProblemDetailsRenderer {
    type_prefix: String,
}

impl ProblemDetailsRenderer {
    /// Creates a new `ProblemDetailsRenderer` with the default `"urn:djangors:error:"`
    /// type URI prefix.
    pub fn new() -> Self {
        Self {
            type_prefix: "urn:djangors:error:".to_string(),
        }
    }

    /// Creates a `ProblemDetailsRenderer` using a custom `type` URI prefix
    /// (for example `"https://api.example.com/problems/"`).
    pub fn with_type_prefix(prefix: impl Into<String>) -> Self {
        Self {
            type_prefix: prefix.into(),
        }
    }
}

impl ErrorRenderer for ProblemDetailsRenderer {
    fn render(&self, err: &DjangorsError, req: &Request) -> Response {
        // Renderers do not know the debug flag, so internal error text is never exposed here.
        let redacted;
        let shown = if err.is_internal() {
            redacted = DjangorsError::Internal("Internal Server Error".to_string());
            &redacted
        } else {
            err
        };
        let mut buf = Vec::with_capacity(256);
        shown.write_problem_json_with_prefix(&self.type_prefix, Some(req.path()), &mut buf);
        Response::problem_json_bytes(shown.status_code(), buf)
    }
}

/// An application-defined error carrying an explicit status code, a stable
/// machine-readable code, and optional structured details.
///
/// This is the escape hatch for domain errors that the built-in
/// [`DjangorsError`] variants cannot express — any status code, any code
/// string, and an arbitrary JSON payload that clients can branch on.
///
/// # Examples
///
/// ```
/// use djangors_core::error::DjangorsError;
/// use hyper::StatusCode;
/// use serde_json::json;
///
/// let err = DjangorsError::api(StatusCode::CONFLICT, "seat_taken", "That seat is already booked")
///     .with_details(json!({ "seat": "12A", "flight": "DL404" }));
///
/// assert_eq!(err.status_code(), StatusCode::CONFLICT);
/// assert_eq!(err.code(), "seat_taken");
/// assert!(err.details().is_some());
/// ```
#[derive(Debug, Clone)]
pub struct ApiError {
    /// The HTTP status code to respond with.
    pub status: StatusCode,
    /// A stable, machine-readable domain code (e.g. `"insufficient_funds"`).
    pub code: String,
    /// A human-readable message safe to show to the caller.
    pub message: String,
    /// Optional structured payload, serialized under `error.details`.
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    /// Create a new API error.
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Attach a structured details payload.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Errors that can occur during request handling.
#[derive(Debug)]
pub enum DjangorsError {
    /// The requested resource was not found (404).
    NotFound,
    /// The request was malformed (400).
    BadRequest(String),
    /// An internal server error occurred (500).
    Internal(String),
    /// The handler panicked (500).
    Panicked(String),
    /// The request is unauthorized (401).
    Unauthorized(String),
    /// The request is forbidden (403).
    Forbidden(String),
    /// The request exceeded a configured rate limit (429).
    TooManyRequests(String),
    /// An application-defined error with an explicit status, domain code, and
    /// optional structured details. See [`ApiError`].
    Api(ApiError),
}

impl fmt::Display for DjangorsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DjangorsError::NotFound => write!(f, "Not Found"),
            DjangorsError::BadRequest(msg) => write!(f, "Bad Request: {msg}"),
            DjangorsError::Internal(msg) => write!(f, "Internal Error: {msg}"),
            DjangorsError::Panicked(msg) => write!(f, "Handler panicked: {msg}"),
            DjangorsError::Unauthorized(msg) => write!(f, "Unauthorized: {msg}"),
            DjangorsError::Forbidden(msg) => write!(f, "Forbidden: {msg}"),
            DjangorsError::TooManyRequests(msg) => write!(f, "Too Many Requests: {msg}"),
            DjangorsError::Api(api) => write!(f, "{}: {}", api.code, api.message),
        }
    }
}

impl std::error::Error for DjangorsError {}

impl From<ApiError> for DjangorsError {
    fn from(err: ApiError) -> Self {
        DjangorsError::Api(err)
    }
}

impl DjangorsError {
    /// Construct an application-defined error with an explicit status code and
    /// stable domain code.
    ///
    /// Chain [`DjangorsError::with_details`] to attach a structured payload.
    pub fn api(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        DjangorsError::Api(ApiError::new(status, code, message))
    }

    /// Attach a structured details payload to this error.
    ///
    /// Built-in variants are promoted to [`DjangorsError::Api`], preserving
    /// their status code, domain code, and message.
    pub fn with_details(self, details: serde_json::Value) -> Self {
        let status = self.status_code();
        match self {
            DjangorsError::Api(api) => DjangorsError::Api(api.with_details(details)),
            other => {
                let code = other.code().to_string();
                let message = other.message();
                DjangorsError::Api(ApiError {
                    status,
                    code,
                    message,
                    details: Some(details),
                })
            }
        }
    }

    /// The stable, machine-readable code for this error.
    ///
    /// Built-in variants return a fixed snake_case code; [`DjangorsError::Api`]
    /// returns the application-supplied one.
    pub fn code(&self) -> &str {
        match self {
            Self::NotFound => "not_found",
            Self::BadRequest(_) => "bad_request",
            Self::Internal(_) => "internal",
            Self::Panicked(_) => "panicked",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::TooManyRequests(_) => "too_many_requests",
            Self::Api(api) => &api.code,
        }
    }

    /// The human-readable message for this error.
    pub fn message(&self) -> String {
        match self {
            Self::NotFound => "Not Found".to_string(),
            Self::BadRequest(msg)
            | Self::Internal(msg)
            | Self::Panicked(msg)
            | Self::Unauthorized(msg)
            | Self::Forbidden(msg) => msg.clone(),
            Self::TooManyRequests(msg) => msg.clone(),
            Self::Api(api) => api.message.clone(),
        }
    }

    /// The structured details payload, if one was attached.
    pub fn details(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Api(api) => api.details.as_ref(),
            _ => None,
        }
    }

    /// Render this error with a project-registered renderer, if present.
    pub fn try_custom_render(&self, req: &Request) -> Option<Response> {
        req.state::<std::sync::Arc<dyn ErrorRenderer>>()
            .map(|renderer| renderer.render(self, req))
    }

    /// Whether this error should be rendered as JSON for the given request.
    ///
    /// True when the caller explicitly asked for JSON via `Accept`, or when the
    /// error is an [`ApiError`] — an application-defined code and details
    /// payload is only meaningful to a machine reader, so an HTML page would
    /// throw the information away.
    fn prefers_json(&self, req: &Request) -> bool {
        if matches!(self, DjangorsError::Api(_)) {
            return true;
        }
        req.header("accept")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|accept| {
                accept.contains("application/json") || accept.contains("application/problem+json")
            })
    }

    fn prefers_problem_json(&self, req: &Request) -> bool {
        req.header("accept")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|accept| accept.contains("application/problem+json"))
    }

    /// Render this error into the response the caller should receive.
    ///
    /// Resolution order:
    /// 1. A project-registered [`ErrorRenderer`], if one is in state.
    /// 2. RFC 9457 `application/problem+json` when requested via `Accept`.
    /// 3. The JSON envelope, when the caller asked for JSON or this is an
    ///    [`ApiError`].
    /// 4. The rich debug page (when `debug`) or the minimal production page.
    ///
    /// Handlers do not call this — the router does, on every error path.
    pub fn render(&self, req: &Request, debug: bool) -> Response {
        if let Some(resp) = self.try_custom_render(req) {
            return resp;
        }
        // Internal error text can carry SQL, file paths or upstream responses; outside
        // debug mode JSON clients get the same generic message the HTML page shows.
        let redacted;
        let shown = if !debug && self.is_internal() {
            redacted = DjangorsError::Internal("Internal Server Error".to_string());
            &redacted
        } else {
            self
        };
        if shown.prefers_problem_json(req) {
            return shown.into_problem_response_for(Some(req.path()));
        }
        if shown.prefers_json(req) {
            return shown.into_json_response();
        }
        if debug {
            crate::debug_page::render_debug_page(self, req)
        } else {
            crate::debug_page::render_production_error_page(self.status_code())
        }
    }

    fn is_internal(&self) -> bool {
        matches!(
            self,
            DjangorsError::Internal(_) | DjangorsError::Panicked(_)
        )
    }

    /// Return the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            DjangorsError::NotFound => StatusCode::NOT_FOUND,
            DjangorsError::BadRequest(_) => StatusCode::BAD_REQUEST,
            DjangorsError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DjangorsError::Panicked(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DjangorsError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            DjangorsError::Forbidden(_) => StatusCode::FORBIDDEN,
            DjangorsError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            DjangorsError::Api(api) => api.status,
        }
    }

    /// Render this error as the standard JSON error envelope using the direct
    /// byte writer.
    ///
    /// The shape is `{"error": {"status", "code", "message", "details"?}}`.
    /// `details` is omitted entirely when absent.
    pub fn into_json_response(&self) -> Response {
        let mut buf = Vec::with_capacity(192);
        self.write_error_envelope_json(&mut buf);
        Response::json_bytes(self.status_code(), buf)
    }

    /// Write the standard `{"error": {"status", "code", "message", "details"?}}`
    /// JSON envelope directly into `buf` without intermediate structs.
    pub fn write_error_envelope_json(&self, buf: &mut Vec<u8>) {
        use djangors_orm::json::{write_json_i64, write_json_str};

        let status = self.status_code();
        let message = self.message();
        buf.extend_from_slice(b"{\"error\":{\"status\":");
        write_json_i64(buf, status.as_u16() as i64);
        buf.extend_from_slice(b",\"code\":");
        write_json_str(buf, self.code());
        buf.extend_from_slice(b",\"message\":");
        write_json_str(buf, &message);
        if let Some(details) = self.details() {
            buf.extend_from_slice(b",\"details\":");
            let _ = serde_json::to_writer(&mut *buf, details);
        }
        buf.extend_from_slice(b"}}");
    }

    /// Write an [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) Problem Details
    /// JSON body directly into `buf` using Djangors's direct JSON primitives.
    ///
    /// Emits `type`, `title`, `status`, `code`, `detail`, optional `instance`,
    /// and optional `errors` / `details` extension members.
    pub fn write_problem_json(&self, instance: Option<&str>, buf: &mut Vec<u8>) {
        self.write_problem_json_with_prefix("urn:djangors:error:", instance, buf);
    }

    pub(crate) fn write_problem_json_with_prefix(
        &self,
        type_prefix: &str,
        instance: Option<&str>,
        buf: &mut Vec<u8>,
    ) {
        use djangors_orm::json::{write_json_i64, write_json_str};

        let status = self.status_code();
        let title = status.canonical_reason().unwrap_or("Error");
        let code = self.code();
        let detail = self.message();
        let type_uri = format!("{type_prefix}{code}");

        buf.extend_from_slice(b"{\"type\":");
        write_json_str(buf, &type_uri);
        buf.extend_from_slice(b",\"title\":");
        write_json_str(buf, title);
        buf.extend_from_slice(b",\"status\":");
        write_json_i64(buf, status.as_u16() as i64);
        buf.extend_from_slice(b",\"code\":");
        write_json_str(buf, code);
        buf.extend_from_slice(b",\"detail\":");
        write_json_str(buf, &detail);
        if let Some(inst) = instance {
            buf.extend_from_slice(b",\"instance\":");
            write_json_str(buf, inst);
        }
        if let Some(details) = self.details() {
            buf.extend_from_slice(b",\"errors\":");
            let _ = serde_json::to_writer(&mut *buf, details);
            buf.extend_from_slice(b",\"details\":");
            let _ = serde_json::to_writer(&mut *buf, details);
        }
        buf.push(b'}');
    }

    /// Render this error as an RFC 9457 `application/problem+json` response.
    pub fn into_problem_response(&self) -> Response {
        self.into_problem_response_for(None)
    }

    /// Render this error as an RFC 9457 `application/problem+json` response
    /// with an optional request `instance` URI.
    pub fn into_problem_response_for(&self, instance: Option<&str>) -> Response {
        let mut buf = Vec::with_capacity(256);
        self.write_problem_json(instance, &mut buf);
        Response::problem_json_bytes(self.status_code(), buf)
    }

    /// Render this error as a plain-text response.
    ///
    /// [`DjangorsError::Api`] renders as the JSON envelope instead, since an
    /// application-defined code and details payload only carry meaning to a
    /// machine reader.
    fn text_response(&self) -> Response {
        let status = self.status_code();
        match self {
            DjangorsError::NotFound => Response::text(status, "404 Not Found"),
            DjangorsError::BadRequest(msg) => {
                Response::text(status, &format!("400 Bad Request: {msg}"))
            }
            DjangorsError::Internal(msg) => {
                Response::text(status, &format!("500 Internal Server Error: {msg}"))
            }
            DjangorsError::Panicked(msg) => Response::text(
                status,
                &format!("500 Internal Server Error: Handler panicked: {msg}"),
            ),
            DjangorsError::Unauthorized(msg) => {
                Response::text(status, &format!("401 Unauthorized: {msg}"))
            }
            DjangorsError::Forbidden(msg) => {
                Response::text(status, &format!("403 Forbidden: {msg}"))
            }
            DjangorsError::TooManyRequests(msg) => {
                Response::text(status, &format!("429 Too Many Requests: {msg}"))
            }
            DjangorsError::Api(_) => self.into_json_response(),
        }
    }

    /// Convert this error into an HTTP response.
    ///
    /// See [`DjangorsError::text_response`] for the rendering rules.
    pub fn into_response(self) -> Response {
        self.text_response()
    }

    /// Render this error for a router that has no debug/production error pages,
    /// honouring a project renderer and JSON content negotiation before falling
    /// back to plain text.
    pub fn render_basic(&self, req: &Request) -> Response {
        if let Some(resp) = self.try_custom_render(req) {
            return resp;
        }
        if self.prefers_problem_json(req) {
            return self.into_problem_response_for(Some(req.path()));
        }
        if self.prefers_json(req) {
            return self.into_json_response();
        }
        self.text_response()
    }
}

impl From<djangors_orm::djangors_forms::FormErrors> for DjangorsError {
    fn from(errors: djangors_orm::djangors_forms::FormErrors) -> Self {
        let details = errors.to_json();
        DjangorsError::api(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            "Form validation failed",
        )
        .with_details(details)
    }
}

/// Maps arbitrary error types into [`DjangorsError::Api`] with an explicit
/// status and domain code.
///
/// This keeps handler bodies free of `map_err` closures while still forcing a
/// deliberate choice of status code and stable code for each failure mode.
///
/// # Examples
///
/// ```no_run
/// use djangors_core::error::{DjangorsError, ApiResultExt};
/// use hyper::StatusCode;
///
/// fn parse_quantity(raw: &str) -> Result<u32, DjangorsError> {
///     raw.parse::<u32>()
///         .api_err(StatusCode::UNPROCESSABLE_ENTITY, "invalid_quantity")
/// }
/// ```
pub trait ApiResultExt<T> {
    /// Convert the error case into [`DjangorsError::Api`], using the source
    /// error's `Display` output as the message.
    fn api_err(self, status: StatusCode, code: impl Into<String>) -> Result<T, DjangorsError>;

    /// Convert the error case into [`DjangorsError::Api`] with a fixed message,
    /// discarding the source error's own text.
    ///
    /// Use this when the underlying error may leak internal detail.
    fn api_err_msg(
        self,
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<T, DjangorsError>;
}

impl<T, E: fmt::Display> ApiResultExt<T> for Result<T, E> {
    fn api_err(self, status: StatusCode, code: impl Into<String>) -> Result<T, DjangorsError> {
        self.map_err(|e| DjangorsError::api(status, code, e.to_string()))
    }

    fn api_err_msg(
        self,
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<T, DjangorsError> {
        self.map_err(|_| DjangorsError::api(status, code, message))
    }
}
