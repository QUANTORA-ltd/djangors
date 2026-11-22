//! Context processors that inject shared variables into template contexts.
//!
//! Analogous to Django's `TEMPLATES['OPTIONS']['context_processors']`, each
//! [`ContextProcessor`] receives an optional [`djangors_core::Request`] and
//! returns a map of template context variables merged before rendering.

use std::collections::BTreeMap;

use djangors_core::Request;
use minijinja::Value;

/// Trait for template context processors.
///
/// Implementors produce a deterministic map of variable names to MiniJinja
/// values that are merged into the rendering context by
/// [`crate::TemplateEngine::render_with_request`].
pub trait ContextProcessor: Send + Sync {
    /// Compute context variables for an optional incoming request.
    fn process(&self, req: Option<&Request>) -> BTreeMap<String, Value>;
}

/// Injects `STATIC_URL` into the template context.
#[derive(Debug, Clone)]
pub struct StaticUrlProcessor {
    static_url: String,
}

impl StaticUrlProcessor {
    /// Create a `StaticUrlProcessor` with a custom static URL prefix.
    pub fn new(static_url: impl Into<String>) -> Self {
        Self {
            static_url: static_url.into(),
        }
    }
}

impl Default for StaticUrlProcessor {
    fn default() -> Self {
        Self::new("/static/")
    }
}

impl ContextProcessor for StaticUrlProcessor {
    fn process(&self, _req: Option<&Request>) -> BTreeMap<String, Value> {
        let mut map = BTreeMap::new();
        map.insert(
            "STATIC_URL".to_string(),
            Value::from(self.static_url.clone()),
        );
        map
    }
}

/// Injects `_csrf_token` (and `csrf_token_val`) from the incoming request when present.
///
/// Uses `_csrf_token` rather than `csrf_token` so it does not shadow the
/// built-in `{{ csrf_token() }}` template function. Checks the `X-CSRFToken`
/// request header first, then falls back to the `csrftoken` cookie in the
/// `Cookie` header.
#[derive(Debug, Clone, Default)]
pub struct CsrfContextProcessor {
    cookie_name: String,
}

impl CsrfContextProcessor {
    /// Create a new `CsrfContextProcessor` using the default `csrftoken` cookie name.
    pub fn new() -> Self {
        Self {
            cookie_name: "csrftoken".to_string(),
        }
    }

    /// Create a `CsrfContextProcessor` with a custom cookie name.
    pub fn with_cookie_name(cookie_name: impl Into<String>) -> Self {
        Self {
            cookie_name: cookie_name.into(),
        }
    }

    fn extract_token(&self, req: &Request) -> Option<String> {
        if let Some(hdr) = req.header("x-csrftoken").and_then(|v| v.to_str().ok()) {
            let trimmed = hdr.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(cookie_hdr) = req.header("cookie").and_then(|v| v.to_str().ok()) {
            for part in cookie_hdr.split(';') {
                if let Some((name, val)) = part.trim().split_once('=') {
                    if name.trim() == self.cookie_name {
                        let v = val.trim();
                        if !v.is_empty() {
                            return Some(v.to_string());
                        }
                    }
                }
            }
        }
        None
    }
}

impl ContextProcessor for CsrfContextProcessor {
    fn process(&self, req: Option<&Request>) -> BTreeMap<String, Value> {
        let mut map = BTreeMap::new();
        if let Some(token) = req.and_then(|r| self.extract_token(r)) {
            map.insert("_csrf_token".to_string(), Value::from(token.clone()));
            map.insert("csrf_token_val".to_string(), Value::from(token));
        }
        map
    }
}

/// Injects request metadata under the `request` template variable.
///
/// Exposes `request.path`, `request.method`, `request.raw_query`,
/// `request.is_htmx`, and `request.is_htmx_boosted`.
#[derive(Debug, Clone, Default)]
pub struct RequestContextProcessor;

impl RequestContextProcessor {
    /// Create a new `RequestContextProcessor`.
    pub fn new() -> Self {
        Self
    }
}

impl ContextProcessor for RequestContextProcessor {
    fn process(&self, req: Option<&Request>) -> BTreeMap<String, Value> {
        let mut map = BTreeMap::new();
        if let Some(req) = req {
            let mut req_map = BTreeMap::new();
            req_map.insert("path".to_string(), Value::from(req.path().to_string()));
            req_map.insert(
                "method".to_string(),
                Value::from(req.method().as_str().to_string()),
            );
            req_map.insert(
                "raw_query".to_string(),
                Value::from(req.raw_query().unwrap_or("").to_string()),
            );
            req_map.insert("is_htmx".to_string(), Value::from(req.is_htmx()));
            req_map.insert(
                "is_htmx_boosted".to_string(),
                Value::from(req.is_htmx_boosted()),
            );
            map.insert("request".to_string(), Value::from(req_map));
        }
        map
    }
}

/// Injects static key-value pairs into every template context.
#[derive(Debug, Clone, Default)]
pub struct ExtraContextProcessor {
    values: BTreeMap<String, Value>,
}

impl ExtraContextProcessor {
    /// Create a new `ExtraContextProcessor` with the provided map of values.
    pub fn new(values: BTreeMap<String, Value>) -> Self {
        Self { values }
    }

    /// Add a key-value pair to this processor (builder pattern).
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }
}

impl ContextProcessor for ExtraContextProcessor {
    fn process(&self, _req: Option<&Request>) -> BTreeMap<String, Value> {
        self.values.clone()
    }
}
