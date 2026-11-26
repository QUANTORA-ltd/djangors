//! Inertia.js server-side protocol adapter.
//!
//! Enables React, Vue, and Svelte frontends to work with standard Djangors
//! routes, sessions, and CSRF protection without maintaining a separate REST API.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::DjangorsError;
use crate::html_escape;
use crate::request::Request;
use crate::response::Response;
use crate::StatusCode;

const DEFAULT_ROOT_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Djangors App</title>
</head>
<body>
  <div id="app" data-page="{{page}}"></div>
</body>
</html>"#;

/// Configuration for [`Inertia`] rendering.
#[derive(Debug, Clone)]
pub struct InertiaConfig {
    /// Current asset bundle version compared against `X-Inertia-Version`.
    pub version: String,
    /// Root HTML document template containing `{{page}}` where the escaped JSON page object is injected.
    pub root_template: String,
    /// Shared props merged into every rendered page (page props override shared props on key collision).
    pub shared_props: Map<String, Value>,
}

impl Default for InertiaConfig {
    fn default() -> Self {
        Self {
            version: "1".to_string(),
            root_template: DEFAULT_ROOT_TEMPLATE.to_string(),
            shared_props: Map::new(),
        }
    }
}

impl InertiaConfig {
    /// Create a new [`InertiaConfig`] with the given asset version.
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            ..Self::default()
        }
    }

    /// Set a custom root HTML template containing a `{{page}}` placeholder.
    pub fn with_root_template(mut self, template: impl Into<String>) -> Self {
        self.root_template = template.into();
        self
    }

    /// Attach a shared prop included on every Inertia render.
    pub fn with_shared_prop(
        mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<Self, DjangorsError> {
        let val = serde_json::to_value(value)
            .map_err(|e| DjangorsError::Internal(format!("inertia shared prop: {e}")))?;
        self.shared_props.insert(key.into(), val);
        Ok(self)
    }
}

/// Wire-format Inertia page object returned in JSON responses or `data-page` HTML attributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InertiaPage {
    /// Frontend page component name (e.g. `"Dashboard/Index"`).
    pub component: String,
    /// Page props JSON value.
    pub props: Value,
    /// Current page URL (path + optional query string).
    pub url: String,
    /// Asset version string.
    pub version: String,
}

/// Server-side Inertia.js renderer.
#[derive(Debug, Clone, Default)]
pub struct Inertia {
    config: InertiaConfig,
}

impl Inertia {
    /// Create an [`Inertia`] renderer with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an [`Inertia`] renderer from an explicit [`InertiaConfig`].
    pub fn with_config(config: InertiaConfig) -> Self {
        Self { config }
    }

    /// Return the configured asset version string.
    pub fn version(&self) -> &str {
        &self.config.version
    }

    /// Render an Inertia component for the incoming [`Request`].
    ///
    /// Handles:
    /// - `409 Conflict` with `X-Inertia-Location` when a `GET` Inertia request sends a stale `X-Inertia-Version`.
    /// - Partial reloads when `X-Inertia-Partial-Component` matches `component` and `X-Inertia-Partial-Data` is set.
    /// - `200 OK` JSON response with `X-Inertia: true` and `Vary: X-Inertia` for Inertia XHR requests.
    /// - `200 OK` HTML shell with `<div id="app" data-page="...">` for initial browser page loads.
    pub fn render<P: Serialize>(
        &self,
        req: &Request,
        component: &str,
        props: P,
    ) -> Result<Response, DjangorsError> {
        let url = match req.raw_query() {
            Some(q) if !q.is_empty() => format!("{}?{}", req.path(), q),
            _ => req.path().to_string(),
        };

        if req.is_inertia() && req.method() == hyper::Method::GET {
            if let Some(client_version) = req.inertia_version() {
                if client_version != self.config.version {
                    return Ok(Self::location(&url));
                }
            }
        }

        let mut merged_props = self.config.shared_props.clone();
        let raw_props = serde_json::to_value(props)
            .map_err(|e| DjangorsError::Internal(format!("inertia props serialize: {e}")))?;

        let final_props = match raw_props {
            Value::Object(map) => {
                for (k, v) in map {
                    merged_props.insert(k, v);
                }
                if req.is_inertia() && req.inertia_partial_component() == Some(component) {
                    if let Some(requested_keys) = req.inertia_partial_data() {
                        let filtered: Map<String, Value> = merged_props
                            .into_iter()
                            .filter(|(k, _)| requested_keys.contains(&k.as_str()))
                            .collect();
                        Value::Object(filtered)
                    } else {
                        Value::Object(merged_props)
                    }
                } else {
                    Value::Object(merged_props)
                }
            }
            other => other,
        };

        let page = InertiaPage {
            component: component.to_string(),
            props: final_props,
            url,
            version: self.config.version.clone(),
        };

        if req.is_inertia() {
            let resp = Response::json(StatusCode::OK, &page)?
                .header("X-Inertia", "true")
                .vary_on_headers(&["X-Inertia"]);
            Ok(resp)
        } else {
            let json = serde_json::to_string(&page)
                .map_err(|e| DjangorsError::Internal(format!("inertia page serialize: {e}")))?;
            let escaped = html_escape(&json);
            let html = if self.config.root_template.contains("{{page}}") {
                self.config.root_template.replace("{{page}}", &escaped)
            } else {
                format!(
                    "{}\n<div id=\"app\" data-page=\"{}\"></div>",
                    self.config.root_template, escaped
                )
            };
            Ok(Response::html(StatusCode::OK, html).vary_on_headers(&["X-Inertia"]))
        }
    }

    /// Force a full client-side redirect (`409 Conflict` + `X-Inertia-Location`).
    pub fn location(url: &str) -> Response {
        Response::new(StatusCode::CONFLICT).header("X-Inertia-Location", url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use hyper::http::{HeaderMap, Method, Uri};

    #[test]
    fn initial_browser_request_returns_html_shell() {
        let inertia = Inertia::with_config(
            InertiaConfig::new("v1.1")
                .with_shared_prop("appName", "Djangors")
                .unwrap(),
        );
        let req = Request::new(
            Method::GET,
            "/dashboard?tab=overview".parse::<Uri>().unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        );
        let resp = inertia
            .render(
                &req,
                "Dashboard/Index",
                serde_json::json!({ "count": 42 }),
            )
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "text/html; charset=utf-8"
        );
        let body = std::str::from_utf8(resp.body()).unwrap();
        assert!(body.contains("data-page=\""));
        assert!(body.contains("Dashboard&#x2F;Index"));
        assert!(body.contains("Djangors"));
    }

    #[test]
    fn inertia_xhr_returns_json_and_headers() {
        let inertia = Inertia::with_config(InertiaConfig::new("v1.1"));
        let mut headers = HeaderMap::new();
        headers.insert("x-inertia", "true".parse().unwrap());
        headers.insert("x-inertia-version", "v1.1".parse().unwrap());
        let req = Request::new(
            Method::GET,
            "/posts".parse::<Uri>().unwrap(),
            headers,
            Bytes::new(),
        );
        let resp = inertia
            .render(&req, "Posts/List", serde_json::json!({ "total": 5 }))
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-inertia").unwrap(), "true");
        assert!(resp
            .headers()
            .get("vary")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("X-Inertia"));
        let page: InertiaPage = serde_json::from_slice(resp.body()).unwrap();
        assert_eq!(page.component, "Posts/List");
        assert_eq!(page.url, "/posts");
        assert_eq!(page.version, "v1.1");
        assert_eq!(page.props["total"], 5);
    }

    #[test]
    fn inertia_partial_reload_filters_props() {
        let inertia = Inertia::with_config(
            InertiaConfig::new("v1")
                .with_shared_prop("auth", serde_json::json!({ "user": "ada" }))
                .unwrap(),
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-inertia", "true".parse().unwrap());
        headers.insert("x-inertia-version", "v1".parse().unwrap());
        headers.insert("x-inertia-partial-component", "Posts/List".parse().unwrap());
        headers.insert("x-inertia-partial-data", "posts, filter".parse().unwrap());
        let req = Request::new(
            Method::GET,
            "/posts".parse::<Uri>().unwrap(),
            headers,
            Bytes::new(),
        );
        let resp = inertia
            .render(
                &req,
                "Posts/List",
                serde_json::json!({ "posts": [1, 2], "filter": "hot", "heavy": "ignored" }),
            )
            .unwrap();
        let page: InertiaPage = serde_json::from_slice(resp.body()).unwrap();
        let obj = page.props.as_object().unwrap();
        assert!(obj.contains_key("posts"));
        assert!(obj.contains_key("filter"));
        assert!(!obj.contains_key("heavy"));
        assert!(!obj.contains_key("auth"));
    }

    #[test]
    fn inertia_version_mismatch_returns_409_location() {
        let inertia = Inertia::with_config(InertiaConfig::new("v2"));
        let mut headers = HeaderMap::new();
        headers.insert("x-inertia", "true".parse().unwrap());
        headers.insert("x-inertia-version", "v1".parse().unwrap());
        let req = Request::new(
            Method::GET,
            "/settings?sec=billing".parse::<Uri>().unwrap(),
            headers,
            Bytes::new(),
        );
        let resp = inertia
            .render(&req, "Settings", serde_json::json!({}))
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        assert_eq!(
            resp.headers().get("x-inertia-location").unwrap(),
            "/settings?sec=billing"
        );
    }
}
