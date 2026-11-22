use djangors_core::error::DjangorsError;
use djangors_core::request::Request;
use djangors_core::response::Response;
use djangors_core::StatusCode;
use serde::Serialize;

use crate::engine::TemplateEngine;

/// Shortcut to render a template name using the provided context and engine,
/// returning a `Response` or a `DjangorsError::Internal`.
pub fn render(
    engine: &TemplateEngine,
    name: &str,
    ctx: impl Serialize,
) -> Result<Response, DjangorsError> {
    let html = engine
        .render(name, ctx)
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok(Response::html(StatusCode::OK, html))
}

/// Shortcut to render a template with registered context processors populated
/// from `req`, merged with `extra_ctx`, returning an HTML `Response`.
pub fn render_with_request(
    engine: &TemplateEngine,
    req: &Request,
    name: &str,
    extra_ctx: impl Serialize,
) -> Result<Response, DjangorsError> {
    let html = engine
        .render_with_request(req, name, extra_ctx)
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok(Response::html(StatusCode::OK, html))
}
