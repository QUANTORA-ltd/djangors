//! Dev-only request & SQL query inspection panel (Django Debug Toolbar / Laravel Telescope / Rails Bullet).
//!
//! Available when the `debug-panel` feature is enabled. Wraps each HTTP request in a
//! task-local [`with_query_log`](djangors_orm::djangors_db::with_query_log) scope,
//! measures wall-clock request time and per-query SQL timings, flags N+1 duplicate
//! queries executed in a loop, sets diagnostic `X-Djangors-*` headers, and optionally
//! injects a collapsible HTML toolbar before `</body>` on `text/html` responses.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Bytes;
use djangors_orm::djangors_db::{with_query_log, QueryStats};
use http_body_util::{BodyExt, Full};
use hyper::http::{header::CONTENT_TYPE, HeaderName, HeaderValue};
use tower::{Layer, Service};

use crate::html_escape;

/// Tower [`Layer`] that records SQL queries per request, detects N+1 duplicate query loops,
/// attaches `X-Djangors-Query-Count`, `X-Djangors-Query-Time-Us`, `X-Djangors-Request-Time-Us`,
/// and `X-Djangors-Duplicate-Queries` response headers, and injects an interactive HTML
/// debug panel before `</body>` on HTML responses.
#[derive(Clone, Debug)]
pub struct DebugPanelLayer {
    inject_html: bool,
    enabled: bool,
    /// True when the caller did not say whether this is a debug build (`new()`), so
    /// `layer()` falls back to checking `DJANGORS_DEBUG` before warning.
    debug_unknown: bool,
}

impl Default for DebugPanelLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugPanelLayer {
    /// Creates a new [`DebugPanelLayer`] with HTML toolbar injection enabled.
    pub fn new() -> Self {
        Self {
            inject_html: true,
            enabled: true,
            debug_unknown: true,
        }
    }

    /// Creates a panel layer whose enabled state follows the application debug setting.
    pub fn for_settings(settings: &crate::DjangorsSettings) -> Self {
        Self {
            enabled: settings.debug,
            debug_unknown: false,
            ..Self::new()
        }
    }

    /// Configures whether the collapsible HTML debug toolbar is injected into `text/html` responses.
    pub fn with_html_injection(mut self, enabled: bool) -> Self {
        self.inject_html = enabled;
        self
    }
}

/// Returns a [`DebugPanelLayer`] with HTML toolbar injection and diagnostic headers enabled.
pub fn debug_panel_layer() -> DebugPanelLayer {
    DebugPanelLayer::new()
}

impl<S> Layer<S> for DebugPanelLayer {
    type Service = DebugPanelService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        let debug_env = std::env::var("DJANGORS_DEBUG").is_ok_and(|v| v == "true" || v == "1");
        if self.enabled && self.debug_unknown && !debug_env {
            tracing::warn!(
                "debug panel enabled without DebugPanelLayer::for_settings and DJANGORS_DEBUG \
                 is not set; it exposes SQL to every visitor, so never run it in production"
            );
        }
        DebugPanelService {
            inner,
            inject_html: self.inject_html,
            enabled: self.enabled,
        }
    }
}

/// Tower [`Service`] produced by [`DebugPanelLayer`].
#[derive(Clone, Debug)]
pub struct DebugPanelService<S> {
    inner: S,
    inject_html: bool,
    enabled: bool,
}

impl<S, ReqBody> Service<hyper::Request<ReqBody>> for DebugPanelService<S>
where
    S: Service<
            hyper::Request<ReqBody>,
            Response = hyper::Response<Full<Bytes>>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = hyper::Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: hyper::Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        let inject_html = self.inject_html;
        let enabled = self.enabled;
        let method = req.method().as_str().to_string();
        let path = req.uri().path().to_string();

        Box::pin(async move {
            if !enabled {
                return inner.call(req).await;
            }
            let req_start = Instant::now();
            let (res, stats) = with_query_log(inner.call(req)).await;
            let req_micros = req_start.elapsed().as_micros() as u64;

            let mut resp = res.expect("infallible service");
            attach_debug_headers(resp.headers_mut(), &stats, req_micros);

            if inject_html && is_html_response(resp.headers()) {
                let (mut parts, body) = resp.into_parts();
                let bytes = match body.collect().await {
                    Ok(collected) => collected.to_bytes(),
                    Err(_) => Bytes::new(),
                };
                if let Ok(html_str) = std::str::from_utf8(&bytes) {
                    let panel_html = render_debug_panel_html(&method, &path, req_micros, &stats);
                    // Only full pages get the toolbar; fragments (htmx partials) stay as-is.
                    if let Some(idx) = html_str.rfind("</body>") {
                        let mut buf = String::with_capacity(html_str.len() + panel_html.len());
                        buf.push_str(&html_str[..idx]);
                        buf.push_str(&panel_html);
                        buf.push_str(&html_str[idx..]);
                        parts.headers.remove(hyper::http::header::CONTENT_LENGTH);
                        resp = hyper::Response::from_parts(parts, Full::new(Bytes::from(buf)));
                    } else {
                        resp = hyper::Response::from_parts(parts, Full::new(bytes));
                    }
                } else {
                    resp = hyper::Response::from_parts(parts, Full::new(bytes));
                }
            }

            Ok(resp)
        })
    }
}

fn is_html_response(headers: &hyper::HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("text/html"))
}

fn attach_debug_headers(headers: &mut hyper::HeaderMap, stats: &QueryStats, req_micros: u64) {
    if let Ok(val) = HeaderValue::from_str(&stats.total_queries().to_string()) {
        headers.insert(HeaderName::from_static("x-djangors-query-count"), val);
    }
    if let Ok(val) = HeaderValue::from_str(&stats.total_duration_micros.to_string()) {
        headers.insert(HeaderName::from_static("x-djangors-query-time-us"), val);
    }
    if let Ok(val) = HeaderValue::from_str(&req_micros.to_string()) {
        headers.insert(HeaderName::from_static("x-djangors-request-time-us"), val);
    }
    if let Ok(val) = HeaderValue::from_str(&stats.duplicates.len().to_string()) {
        headers.insert(HeaderName::from_static("x-djangors-duplicate-queries"), val);
    }
}

/// Renders the self-contained HTML snippet for the Djangors Debug Panel.
pub fn render_debug_panel_html(
    method: &str,
    path: &str,
    request_micros: u64,
    stats: &QueryStats,
) -> String {
    let req_ms = request_micros as f64 / 1000.0;
    let sql_ms = stats.total_duration_micros as f64 / 1000.0;
    let query_count = stats.total_queries();
    let dup_count = stats.duplicates.len();

    let badge_color = if dup_count > 0 { "#d97706" } else { "#059669" };
    let warning_banner = if dup_count > 0 {
        let mut items = String::new();
        for dup in &stats.duplicates {
            let dup_ms = dup.total_duration_micros as f64 / 1000.0;
            items.push_str(&format!(
                "<li><strong>{}x</strong> ({dup_ms:.2} ms): <code>{}</code></li>",
                dup.count,
                html_escape(&dup.sql)
            ));
        }
        format!(
            "<div class=\"dj-debug-n1-warning\" style=\"background:#fef3c7;color:#92400e;padding:8px 12px;border-left:4px solid #d97706;margin-bottom:10px;\">\
             <strong>Potential N+1 Query Warning:</strong> {dup_count} duplicate SQL statement(s) detected in a loop:\
             <ul style=\"margin:4px 0 0 18px;padding:0;\">{items}</ul>\
             </div>"
        )
    } else {
        String::new()
    };

    let mut rows = String::new();
    for (idx, q) in stats.queries.iter().enumerate() {
        let q_ms = q.duration_micros as f64 / 1000.0;
        rows.push_str(&format!(
            "<tr><td style=\"padding:4px 8px;border-bottom:1px solid #374151;\">{}</td>\
             <td style=\"padding:4px 8px;border-bottom:1px solid #374151;font-family:monospace;\">{}</td>\
             <td style=\"padding:4px 8px;border-bottom:1px solid #374151;text-align:right;\">{}</td>\
             <td style=\"padding:4px 8px;border-bottom:1px solid #374151;text-align:right;\">{q_ms:.3} ms</td></tr>",
            idx + 1,
            html_escape(&q.sql),
            q.param_count
        ));
    }

    format!(
        "<aside id=\"djangors-debug-panel\" style=\"position:fixed;bottom:0;right:0;left:0;background:#111827;color:#f9fafb;font-family:system-ui,-apple-system,sans-serif;font-size:13px;z-index:99999;border-top:2px solid {badge_color};\">\
         <details>\
         <summary style=\"cursor:pointer;padding:8px 16px;display:flex;gap:16px;align-items:center;\">\
         <strong>Djangors Debug Panel</strong>\
         <span><code>{} {}</code></span>\
         <span>Request: <strong>{req_ms:.2} ms</strong></span>\
         <span>Queries: <strong>{query_count}</strong> ({sql_ms:.2} ms)</span>\
         <span style=\"color:{badge_color};font-weight:600;\">Duplicates: {dup_count}</span>\
         </summary>\
         <div style=\"max-height:280px;overflow:auto;padding:12px 16px;background:#1f2937;\">\
         {warning_banner}\
         <table style=\"width:100%;border-collapse:collapse;text-align:left;\">\
         <thead><tr>\
         <th style=\"padding:4px 8px;border-bottom:1px solid #4b5563;\">#</th>\
         <th style=\"padding:4px 8px;border-bottom:1px solid #4b5563;\">SQL</th>\
         <th style=\"padding:4px 8px;border-bottom:1px solid #4b5563;text-align:right;\">Params</th>\
         <th style=\"padding:4px 8px;border-bottom:1px solid #4b5563;text-align:right;\">Time</th>\
         </tr></thead>\
         <tbody>{rows}</tbody>\
         </table>\
         </div>\
         </details>\
         </aside>",
        html_escape(method),
        html_escape(path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DjangorsSettings;
    use crate::{PathParams, Request, Response, Router, StatusCode};
    use djangors_orm::djangors_db::{Database, DatabaseConfig};
    use tower::ServiceExt;

    #[tokio::test]
    async fn debug_panel_counts_queries_flags_n_plus_one_and_injects_toolbar() {
        let db = Database::connect(&DatabaseConfig::new("sqlite::memory:"))
            .await
            .expect("sqlite memory");
        {
            let db_init = db.clone();
            db_init
                .conn()
                .execute(
                    "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
                    &[],
                )
                .await
                .unwrap();
        }

        let db_for_handler = db.clone();
        let router = Router::new().get("/items", move |_req: Request, _params: PathParams| {
            let db = db_for_handler.clone();
            async move {
                // Intentional N+1 loop: run the exact same query 3 times
                for _ in 0..3 {
                    let _ = db
                        .conn()
                        .fetch_all("SELECT id, name FROM items WHERE id = 1", &[])
                        .await;
                }
                let _ = db.conn().fetch_all("SELECT COUNT(*) FROM items", &[]).await;
                Ok(Response::html(
                    StatusCode::OK,
                    "<html><body><h1>Items</h1></body></html>".to_string(),
                ))
            }
        });

        let svc = debug_panel_layer().layer(router);
        let req = hyper::Request::builder()
            .method("GET")
            .uri("/items")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("x-djangors-query-count")
                .and_then(|v| v.to_str().ok()),
            Some("4")
        );
        assert_eq!(
            resp.headers()
                .get("x-djangors-duplicate-queries")
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );

        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let html = std::str::from_utf8(&body_bytes).unwrap();
        assert!(html.contains("id=\"djangors-debug-panel\""));
        assert!(html.contains("Potential N+1 Query Warning:"));
        assert!(html.contains("3x"));
        assert!(html.contains("SELECT id, name FROM items WHERE id = 1"));
    }

    #[tokio::test]
    async fn disabled_debug_panel_leaves_response_unchanged() {
        let router = Router::new().get("/", |_req: Request, _params: PathParams| async {
            Ok(Response::html(
                StatusCode::OK,
                "<html><body>plain</body></html>",
            ))
        });
        let mut layer = DebugPanelLayer::new();
        layer.enabled = false;
        let svc = layer.layer(router);
        let req = hyper::Request::builder()
            .uri("/")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();
        assert!(resp.headers().get("x-djangors-query-count").is_none());
        assert_eq!(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .as_ref(),
            b"<html><body>plain</body></html>"
        );
    }

    #[test]
    fn for_settings_follows_debug_setting() {
        assert!(!DebugPanelLayer::for_settings(&DjangorsSettings::new().with_debug(false)).enabled);
        assert!(DebugPanelLayer::for_settings(&DjangorsSettings::new().with_debug(true)).enabled);
    }
}
