use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use hyper::http::Method;

use crate::error::DjangorsError;
use crate::handler::{Handler, StreamingHandler};
use crate::path_params::PathParams;
use crate::request::Request;
use crate::response::Response;
use crate::sse::StreamingResponse;
use crate::state::AppState;

#[derive(Debug, Clone)]
enum CaptureType {
    String,
    I64,
    Slug,
}

#[derive(Debug, Clone)]
enum Segment {
    Literal(String),
    Capture(CaptureType),
}

/// Encapsulates standard or streaming handler implementations.
#[derive(Clone)]
pub enum HandlerKind {
    /// A standard buffered handler.
    Standard(Arc<dyn Handler>),
    /// A streaming HTTP response handler.
    Streaming(Arc<dyn StreamingHandler>),
}

/// Discriminates standard buffered responses from streaming responses.
pub enum ResponseKind {
    /// A standard buffered HTTP response.
    Standard(Response),
    /// A streaming HTTP response.
    Streaming(StreamingResponse),
}

#[derive(Clone)]
struct Route {
    pattern: String,
    method: Method,
    segments: Vec<Segment>,
    param_names: Vec<String>,
    handler: HandlerKind,
    name: Option<String>,
}

/// Error type returned when route reversing fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReverseError {
    /// The specified route name was not found in the router.
    #[error("Unknown route name: {0}")]
    UnknownName(String),
    /// A required parameter for the route pattern was not provided.
    #[error("Missing parameter '{param}' for route '{route}'")]
    MissingParam {
        /// The route name or pattern.
        route: String,
        /// The name of the missing parameter.
        param: String,
    },
    /// An unexpected parameter was provided that is not part of the route pattern.
    #[error("Unexpected parameter '{param}' for route '{route}'")]
    UnexpectedParam {
        /// The route name or pattern.
        route: String,
        /// The name of the unexpected parameter.
        param: String,
    },
}

/// A URL router that matches incoming requests to registered handlers.
///
/// Routes are defined with Django-style path syntax:
/// - Literal segments match exactly (e.g. `hello/world`)
/// - `{name}` captures any single path segment as a `String`
/// - `{name:i64}` captures a segment that must parse as `i64`
/// - `{name:slug}` captures a segment matching `[a-zA-Z0-9_-]+`
/// - `:name` is accepted as a shorthand alias for `{name}` (an untyped
///   `String` capture), for developers coming from Express/Django URLconf
///   muscle memory. Prefer `{name}`/`{name:i64}`/`{name:slug}` in new code —
///   the typed forms give you request-time validation the alias can't.
///
/// Routers can be nested with [`mount`](Self::mount).
///
/// `Router` is cheaply [`Clone`]-able (the route table is `Arc`-wrapped) so it
/// can be cloned per-request by `tower::Service` without deep-copying every
/// registered route.
#[derive(Clone)]
pub struct Router {
    routes: Arc<Vec<Route>>,
    state: AppState,
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Router {
    /// Create an empty router.
    pub fn new() -> Self {
        Router {
            routes: Arc::new(Vec::new()),
            state: AppState::new(),
        }
    }

    /// Attach a piece of shared state (e.g. a database connection pool) that
    /// handlers can retrieve via [`Request::state`]. Can be called
    /// multiple times with different types to attach several independent
    /// pieces of state.
    ///
    /// Sub-routers mounted with [`mount`](Self::mount) contribute any state the
    /// parent has not already set for that type; the parent's value wins on
    /// conflict.
    pub fn with_state<T: Send + Sync + 'static>(mut self, value: T) -> Self {
        self.state = self.state.insert(value);
        self
    }

    /// Register a project-wide [`crate::error::ErrorRenderer`] used to turn [`DjangorsError`]
    /// values into responses.
    ///
    /// This is the ergonomic equivalent of
    /// `with_state(Arc::new(renderer) as Arc<dyn ErrorRenderer>)` — the trait-object
    /// cast is the part that is easy to get wrong, because
    /// `with_state(Arc::new(renderer))` compiles perfectly well while storing the
    /// value under its concrete type, where the renderer lookup never finds it and
    /// the custom renderer is silently ignored.
    ///
    /// ```ignore
    /// let router = Router::new().set_error_renderer(MyRenderer);
    /// ```
    pub fn set_error_renderer<R: crate::error::ErrorRenderer + 'static>(self, renderer: R) -> Self {
        self.with_state(
            std::sync::Arc::new(renderer) as std::sync::Arc<dyn crate::error::ErrorRenderer>
        )
    }

    /// Register a handler for the given path and HTTP method.
    ///
    /// # Panics
    /// Panics if the path pattern contains invalid capture syntax.
    pub fn route(mut self, path: &str, method: Method, handler: impl Handler + 'static) -> Self {
        let pattern = path
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();
        let (segments, param_names) = Self::parse_pattern(&pattern);
        Arc::make_mut(&mut self.routes).push(Route {
            pattern,
            method,
            segments,
            param_names,
            handler: HandlerKind::Standard(Arc::new(handler)),
            name: None,
        });
        self
    }

    /// Register a streaming handler for the given path and HTTP method.
    pub fn route_streaming(
        mut self,
        path: &str,
        method: Method,
        handler: impl StreamingHandler + 'static,
    ) -> Self {
        let pattern = path
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();
        let (segments, param_names) = Self::parse_pattern(&pattern);
        Arc::make_mut(&mut self.routes).push(Route {
            pattern,
            method,
            segments,
            param_names,
            handler: HandlerKind::Streaming(Arc::new(handler)),
            name: None,
        });
        self
    }

    /// Assign a name to the most recently registered route.
    ///
    /// # Panics
    /// Panics if called on an empty router or if a route with the given name already exists.
    pub fn name(mut self, name: &str) -> Self {
        if self.routes.is_empty() {
            panic!("Router::name called on an empty router");
        }
        if self.routes.iter().any(|r| r.name.as_deref() == Some(name)) {
            panic!("Duplicate route name '{name}'");
        }
        Arc::make_mut(&mut self.routes).last_mut().unwrap().name = Some(name.to_string());
        self
    }

    /// Reconstruct a path URL for a named route by substituting parameters.
    ///
    /// Values are percent-encoded for safety. Note that typed path converters (e.g. `{id:i64}`)
    /// are not type-validated during reversing in v1, but will not crash on non-numeric strings.
    ///
    /// # Errors
    /// Returns [`ReverseError::UnknownName`] if the name is not registered,
    /// [`ReverseError::MissingParam`] if a required parameter is missing, or
    /// [`ReverseError::UnexpectedParam`] if an unrecognised parameter is supplied.
    pub fn reverse(&self, name: &str, params: &[(&str, &str)]) -> Result<String, ReverseError> {
        let route = self
            .routes
            .iter()
            .find(|r| r.name.as_deref() == Some(name))
            .ok_or_else(|| ReverseError::UnknownName(name.to_string()))?;

        for (k, _) in params {
            if !route.param_names.iter().any(|p| p.as_str() == *k) {
                return Err(ReverseError::UnexpectedParam {
                    route: name.to_string(),
                    param: k.to_string(),
                });
            }
        }

        for p in &route.param_names {
            if !params.iter().any(|(k, _)| k == p) {
                return Err(ReverseError::MissingParam {
                    route: name.to_string(),
                    param: p.clone(),
                });
            }
        }

        let mut path_parts = Vec::new();
        let mut param_idx = 0;

        for segment in &route.segments {
            match segment {
                Segment::Literal(lit) => {
                    path_parts.push(lit.clone());
                }
                Segment::Capture(_) => {
                    let p_name = &route.param_names[param_idx];
                    param_idx += 1;
                    let (_, val) = params.iter().find(|(k, _)| k == p_name).unwrap();

                    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
                    const PATH_SEGMENT_SET: &AsciiSet = &CONTROLS
                        .add(b' ')
                        .add(b'"')
                        .add(b'#')
                        .add(b'<')
                        .add(b'>')
                        .add(b'?')
                        .add(b'`')
                        .add(b'{')
                        .add(b'}')
                        .add(b'/');

                    let encoded = utf8_percent_encode(val, PATH_SEGMENT_SET).to_string();
                    path_parts.push(encoded);
                }
            }
        }

        Ok(format!("/{}", path_parts.join("/")))
    }

    /// Register an SSE streaming GET handler.
    pub fn sse(self, path: &str, handler: impl StreamingHandler + 'static) -> Self {
        self.route_streaming(path, Method::GET, handler)
    }

    /// Register an SSE streaming GET handler (alias for [`sse`](Self::sse)).
    pub fn get_sse(self, path: &str, handler: impl StreamingHandler + 'static) -> Self {
        self.sse(path, handler)
    }

    /// Register a GET handler.
    pub fn get(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::GET, handler)
    }

    /// Register a POST handler.
    pub fn post(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::POST, handler)
    }

    /// Register a PUT handler.
    pub fn put(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::PUT, handler)
    }

    /// Register a DELETE handler.
    pub fn delete(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::DELETE, handler)
    }

    /// Mount a sub-router at the given prefix.
    ///
    /// All routes from `sub_router` are registered with `prefix` prepended.
    /// For example, mounting a router with route `users/{id}` at `/api`
    /// creates a route matching `/api/users/{id}`.
    pub fn mount(mut self, prefix: &str, sub_router: Router) -> Self {
        let prefix = prefix.trim_start_matches('/').trim_end_matches('/');
        for route in sub_router.routes.iter() {
            let full_pattern = if route.pattern.is_empty() {
                prefix.to_string()
            } else {
                format!("{}/{}", prefix, route.pattern)
            };
            if let Some(ref name) = route.name {
                if self.routes.iter().any(|r| r.name.as_deref() == Some(name)) {
                    panic!("Duplicate route name '{name}'");
                }
            }
            let (segments, param_names) = Self::parse_pattern(&full_pattern);
            Arc::make_mut(&mut self.routes).push(Route {
                pattern: full_pattern,
                method: route.method.clone(),
                segments,
                param_names,
                handler: route.handler.clone(),
                name: route.name.clone(),
            });
        }
        // Inherit any state the sub-router configured that the parent has not.
        // Previously a sub-router's state was dropped on the floor, so a
        // handler that worked standalone would fail with "state absent" once
        // mounted — the parent's values still take precedence.
        self.state = self.state.merge(&sub_router.state);
        self
    }

    /// Parse a path pattern into segments and parameter names.
    fn parse_pattern(pattern: &str) -> (Vec<Segment>, Vec<String>) {
        let mut segments = Vec::new();
        let mut param_names = Vec::new();

        if pattern.is_empty() {
            return (segments, param_names);
        }

        for part in pattern.split('/') {
            if part.is_empty() {
                continue;
            }
            if part.starts_with('{') && part.ends_with('}') {
                let inner = &part[1..part.len() - 1];
                let (name, capture_type) = if let Some((name, ty)) = inner.split_once(':') {
                    let ct = match ty {
                        "i64" => CaptureType::I64,
                        "slug" => CaptureType::Slug,
                        _ => CaptureType::String,
                    };
                    (name, ct)
                } else {
                    (inner, CaptureType::String)
                };
                param_names.push(name.to_string());
                segments.push(Segment::Capture(capture_type));
            } else if let Some(name) = part.strip_prefix(':') {
                // Express/Django-style `:name` segments never matched anything real —
                // a real request path segment is never the literal text `:name` — so
                // every route written this way was silently unreachable. Treat it as
                // an alias for the untyped `{name}` capture instead of a 404 trap.
                param_names.push(name.to_string());
                segments.push(Segment::Capture(CaptureType::String));
            } else {
                segments.push(Segment::Literal(part.to_string()));
            }
        }

        (segments, param_names)
    }

    /// Try to match a path and method against registered routes.
    fn match_path(&self, path: &str, method: &Method) -> Option<(usize, PathParams)> {
        let normalized = path.trim_start_matches('/').trim_end_matches('/');
        let parts: Vec<&str> = if normalized.is_empty() {
            Vec::new()
        } else {
            normalized.split('/').collect()
        };

        let mut best_match: Option<(usize, PathParams, (usize, usize))> = None;
        for (idx, route) in self.routes.iter().enumerate() {
            if route.method != *method {
                continue;
            }
            if route.segments.len() != parts.len() {
                continue;
            }

            let mut params = PathParams::new();
            let mut matched = true;
            let mut param_idx = 0;

            for (i, segment) in route.segments.iter().enumerate() {
                let matched_segment = match segment {
                    Segment::Literal(expected) => parts[i] == expected.as_str(),
                    Segment::Capture(ct) => {
                        let valid = match ct {
                            CaptureType::I64 => parts[i].parse::<i64>().is_ok(),
                            CaptureType::Slug => parts[i]
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                            CaptureType::String => true,
                        };
                        if valid {
                            if param_idx < route.param_names.len() {
                                params.insert(&route.param_names[param_idx], parts[i]);
                            }
                            param_idx += 1;
                        }
                        valid
                    }
                };

                if !matched_segment {
                    matched = false;
                    break;
                }
            }

            if matched {
                let static_segments = route
                    .segments
                    .iter()
                    .filter(|segment| matches!(segment, Segment::Literal(_)))
                    .count();
                let typed_captures = route
                    .segments
                    .iter()
                    .filter(|segment| {
                        matches!(
                            segment,
                            Segment::Capture(CaptureType::I64 | CaptureType::Slug)
                        )
                    })
                    .count();
                let specificity = (static_segments, typed_captures);
                if best_match
                    .as_ref()
                    .is_none_or(|(_, _, current)| specificity > *current)
                {
                    best_match = Some((idx, params, specificity));
                }
            }
        }

        best_match.map(|(idx, params, _)| (idx, params))
    }

    /// Dispatch a fully constructed [`Request`] to the matching handler.
    ///
    /// Matches the request's path and method against registered routes, calls
    /// the matching handler, and returns the response. Returns a 404 error if
    /// no route matches.
    ///
    /// # Panic Isolation
    ///
    /// Handler execution is wrapped in a task spawned via `tokio::spawn` to catch
    /// and isolate any panics. A panicking handler must not take down other
    /// in-flight requests or crash the whole server process.
    /// Dispatch a request to the matching handler (standard or streaming).
    pub async fn handle_dispatch(&self, req: Request) -> Result<ResponseKind, DjangorsError> {
        let path = req.path().to_string();
        let method = req.method().clone();

        crate::signals::REQUEST_STARTED
            .send(crate::signals::RequestStarted {
                method: method.to_string(),
                path: path.clone(),
            })
            .await;

        let res = match self.match_path(&path, &method) {
            Some((idx, params)) => match &self.routes[idx].handler {
                HandlerKind::Standard(handler) => {
                    let handler = handler.clone();
                    let join_handle = tokio::spawn(async move { handler.call(req, params).await });
                    match join_handle.await {
                        Ok(Ok(resp)) => Ok(ResponseKind::Standard(resp)),
                        Ok(Err(e)) => Err(e),
                        Err(join_err) => {
                            if join_err.is_panic() {
                                let payload = join_err.into_panic();
                                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                                    s.to_string()
                                } else if let Some(s) = payload.downcast_ref::<String>() {
                                    s.clone()
                                } else {
                                    "unknown panic message".to_string()
                                };
                                Err(DjangorsError::Panicked(msg))
                            } else {
                                Err(DjangorsError::Internal(format!(
                                    "handler task execution failed: {join_err}"
                                )))
                            }
                        }
                    }
                }
                HandlerKind::Streaming(handler) => {
                    let handler = handler.clone();
                    let join_handle = tokio::spawn(async move { handler.call(req, params).await });
                    match join_handle.await {
                        Ok(Ok(resp)) => Ok(ResponseKind::Streaming(resp)),
                        Ok(Err(e)) => Err(e),
                        Err(join_err) => {
                            if join_err.is_panic() {
                                let payload = join_err.into_panic();
                                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                                    s.to_string()
                                } else if let Some(s) = payload.downcast_ref::<String>() {
                                    s.clone()
                                } else {
                                    "unknown panic message".to_string()
                                };
                                Err(DjangorsError::Panicked(msg))
                            } else {
                                Err(DjangorsError::Internal(format!(
                                    "handler task execution failed: {join_err}"
                                )))
                            }
                        }
                    }
                }
            },
            None => Err(DjangorsError::NotFound),
        };

        let status = match &res {
            Ok(ResponseKind::Standard(resp)) => resp.status().as_u16(),
            Ok(ResponseKind::Streaming(resp)) => resp.status().as_u16(),
            Err(err) => err.status_code().as_u16(),
        };

        crate::signals::REQUEST_FINISHED
            .send(crate::signals::RequestFinished {
                method: method.to_string(),
                path: path.clone(),
                status,
            })
            .await;

        res
    }

    /// Dispatch a fully constructed [`Request`] to the matching handler.
    ///
    /// Matches the request's path and method against registered routes, calls
    /// the matching handler, and returns the response. Returns a 404 error if
    /// no route matches.
    ///
    /// # Panic Isolation
    ///
    /// Handler execution is wrapped in a task spawned via `tokio::spawn` to catch
    /// and isolate any panics. A panicking handler must not take down other
    /// in-flight requests or crash the whole server process.
    pub async fn handle(&self, req: Request) -> Result<Response, DjangorsError> {
        match self.handle_dispatch(req).await? {
            ResponseKind::Standard(resp) => Ok(resp),
            ResponseKind::Streaming(_) => Err(DjangorsError::Internal(
                "Streaming response cannot be handled via buffered handle() call".into(),
            )),
        }
    }

    /// Dispatch an incoming hyper request, consuming the body, and return a
    /// hyper response.
    ///
    /// This is the top-level entry point intended for use by server bindings.
    /// Body read errors are converted to 500 responses.
    pub async fn dispatch<B>(
        &self,
        hyper_req: hyper::Request<B>,
    ) -> hyper::Response<http_body_util::Full<Bytes>>
    where
        B: hyper::body::Body<Data = Bytes> + Send,
        B::Error: fmt::Display,
    {
        let (parts, body) = hyper_req.into_parts();

        let body_bytes = match http_body_util::BodyExt::collect(body).await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                let err = DjangorsError::Internal(format!("failed to read request body: {e}"));
                let req = Request::new(parts.method, parts.uri, parts.headers, Bytes::new())
                    .with_state(self.state.clone());
                return err.render_basic(&req).into_hyper();
            }
        };

        let req = Request::new(parts.method, parts.uri, parts.headers, body_bytes)
            .with_state(self.state.clone())
            .with_extensions(parts.extensions);

        if let Some(pending) = req.ext::<crate::middleware::CsrfPendingFormCheck>() {
            let is_form_content_type = req
                .header("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.starts_with("application/x-www-form-urlencoded"));

            let form_token = if is_form_content_type {
                serde_urlencoded::from_bytes::<std::collections::HashMap<String, String>>(
                    req.body_bytes().await,
                )
                .ok()
                .and_then(|form| form.get("csrfmiddlewaretoken").cloned())
            } else {
                None
            };

            let valid = form_token
                .map(|t| crate::middleware::constant_time_eq(t.as_bytes(), pending.0.as_bytes()))
                .unwrap_or(false);

            if !valid {
                let err = DjangorsError::Forbidden("CSRF token missing or invalid".to_string());
                return err.render_basic(&req).into_hyper();
            }
        }

        let req_clone = req.clone();
        match self.handle(req).await {
            Ok(resp) => resp.into_hyper(),
            Err(e) => e.render_basic(&req_clone).into_hyper(),
        }
    }

    /// Dispatch an incoming hyper request, and render a Django-style debug page
    /// or a production-safe generic page depending on the `debug` setting.
    pub async fn dispatch_debug<B>(
        &self,
        hyper_req: hyper::Request<B>,
        debug: bool,
    ) -> hyper::Response<http_body_util::Full<Bytes>>
    where
        B: hyper::body::Body<Data = Bytes> + Send,
        B::Error: fmt::Display,
    {
        let (parts, body) = hyper_req.into_parts();

        let body_bytes = match http_body_util::BodyExt::collect(body).await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                let err = DjangorsError::Internal(format!("failed to read request body: {e}"));
                let dummy_req = Request::new(parts.method, parts.uri, parts.headers, Bytes::new())
                    .with_state(self.state.clone());
                let resp = err.render(&dummy_req, debug);
                return resp.into_hyper();
            }
        };

        let req = Request::new(parts.method, parts.uri, parts.headers, body_bytes)
            .with_state(self.state.clone())
            .with_extensions(parts.extensions);

        if let Some(pending) = req.ext::<crate::middleware::CsrfPendingFormCheck>() {
            let is_form_content_type = req
                .header("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.starts_with("application/x-www-form-urlencoded"));

            let form_token = if is_form_content_type {
                serde_urlencoded::from_bytes::<std::collections::HashMap<String, String>>(
                    req.body_bytes().await,
                )
                .ok()
                .and_then(|form| form.get("csrfmiddlewaretoken").cloned())
            } else {
                None
            };

            let valid = form_token
                .map(|t| crate::middleware::constant_time_eq(t.as_bytes(), pending.0.as_bytes()))
                .unwrap_or(false);

            if !valid {
                let err = DjangorsError::Forbidden("CSRF token missing or invalid".to_string());
                let resp = err.render(&req, debug);
                return resp.into_hyper();
            }
        }

        let req_clone = req.clone();

        match self.handle(req).await {
            Ok(resp) => resp.into_hyper(),
            Err(e) => {
                let resp = e.render(&req_clone, debug);
                resp.into_hyper()
            }
        }
    }

    /// Dispatch an incoming hyper request and return a boxed hyper response.
    /// Supports both standard and streaming responses.
    pub async fn dispatch_boxed<B>(
        &self,
        hyper_req: hyper::Request<B>,
        debug: bool,
    ) -> hyper::Response<http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>>
    where
        B: hyper::body::Body<Data = Bytes> + Send,
        B::Error: fmt::Display,
    {
        let (parts, body) = hyper_req.into_parts();

        let body_bytes = match http_body_util::BodyExt::collect(body).await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                let err = DjangorsError::Internal(format!("failed to read request body: {e}"));
                let dummy_req = Request::new(parts.method, parts.uri, parts.headers, Bytes::new())
                    .with_state(self.state.clone());
                let resp = err.render(&dummy_req, debug);
                return resp.into_hyper_boxed();
            }
        };

        let req = Request::new(parts.method, parts.uri, parts.headers, body_bytes)
            .with_state(self.state.clone())
            .with_extensions(parts.extensions);

        if let Some(pending) = req.ext::<crate::middleware::CsrfPendingFormCheck>() {
            let is_form_content_type = req
                .header("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.starts_with("application/x-www-form-urlencoded"));

            let form_token = if is_form_content_type {
                serde_urlencoded::from_bytes::<std::collections::HashMap<String, String>>(
                    req.body_bytes().await,
                )
                .ok()
                .and_then(|form| form.get("csrfmiddlewaretoken").cloned())
            } else {
                None
            };

            let valid = form_token
                .map(|t| crate::middleware::constant_time_eq(t.as_bytes(), pending.0.as_bytes()))
                .unwrap_or(false);

            if !valid {
                let err = DjangorsError::Forbidden("CSRF token missing or invalid".to_string());
                let resp = err.render(&req, debug);
                return resp.into_hyper_boxed();
            }
        }

        let req_clone = req.clone();

        match self.handle_dispatch(req).await {
            Ok(ResponseKind::Standard(resp)) => resp.into_hyper_boxed(),
            Ok(ResponseKind::Streaming(resp)) => resp.into_hyper(),
            Err(e) => {
                let resp = e.render(&req_clone, debug);
                resp.into_hyper_boxed()
            }
        }
    }
}

/// A wrapper around [`Router`] that implements [`tower::Service`], carrying a
/// `debug` flag through to [`Router::dispatch_debug`] so a layered serving
/// path (see [`crate::app::Djangors::serve_service`]) still gets the
/// dev debug page / production error page distinction.
///
/// `Router` itself also implements `tower::Service` directly (see
/// `crate::service`), but that impl always calls the plain
/// [`Router::dispatch`] (no debug page) since it predates the `debug` flag
/// having anywhere to come from in that context — this wrapper is not
/// redundant with it, it exists specifically to carry that flag.
#[derive(Clone)]
pub struct RouterService {
    router: Router,
    debug: bool,
}

impl RouterService {
    /// Create a new `RouterService` wrapping the given router.
    pub fn new(router: Router, debug: bool) -> Self {
        Self { router, debug }
    }
}

impl tower::Service<hyper::Request<hyper::body::Incoming>> for RouterService {
    type Response = hyper::Response<http_body_util::Full<Bytes>>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<hyper::body::Incoming>) -> Self::Future {
        let router = self.router.clone();
        let debug = self.debug;
        Box::pin(async move {
            let resp = router.dispatch_debug(req, debug).await;
            Ok(resp)
        })
    }
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
