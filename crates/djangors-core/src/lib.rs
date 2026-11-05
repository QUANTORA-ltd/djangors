#![deny(missing_docs)]
//! HTTP kernel for the Djangors web framework.
//!
//! Provides the core [`Request`], [`Response`], [`Router`], and [`Handler`]
//! types that form the foundation of Djangors's HTTP layer.
//!
//! # CSRF Protection (v1 Scope)
//!
//! **CRITICAL SECURITY NOTE:**
//! CSRF middleware in v1 only validates unsafe requests (e.g. POST, PUT, PATCH, DELETE)
//! via the `X-CSRFToken` header. It does **not** validate CSRF tokens passed inside a
//! form body field (like Django's `csrfmiddlewaretoken`). Consequently:
//! - **Protected:** JSON/AJAX-style requests that set the `X-CSRFToken` header.
//! - **NOT Protected:** Classic `<form method="post">` HTML form submissions without JavaScript.
//!
//! **BREACH Defense (Future Work):**
//! This version uses a double-submit cookie scheme. Django's BREACH-hardened masked-secret
//! scheme is not yet implemented and is planned as future work.

extern crate self as djangors_core;

/// Application builder and launcher logic.
pub mod app;
/// Django-style HTML debug and error page rendering.
pub mod debug_page;
/// Framework core error types.
pub mod error;
/// Request parameter extractor utilities.
pub mod extract;
/// Pub/sub broadcast group channel registry.
pub mod groups;
/// Handler trait definitions for standard and streaming HTTP routes.
pub mod handler;
/// HTTP request/response logging middleware.
pub mod logging;
/// Optional RED metrics middleware and Prometheus exposition endpoint.
#[cfg(feature = "metrics")]
pub mod metrics;
/// Standard HTTP middleware layers (CSRF, Request ID, etc.).
pub mod middleware;
/// Pagination utility engines.
pub mod pagination;
/// Path parameter map container.
pub mod path_params;
/// Named, opt-in endpoint rate limiting.
pub mod ratelimit;
/// Request structure wrapper.
pub mod request;
/// Response structure wrapper.
pub mod response;
/// URL routing engine and pattern matching.
pub mod router;
/// Tower service implementations for Djangors applications.
pub mod service;
/// Framework-wide settings container.
pub mod settings;
/// Framework lifecycle event signals.
pub mod signals;
/// Server-Sent Events (SSE) streaming response types.
pub mod sse;
/// Type-map state storage container.
pub mod state;

pub use app::Djangors;
pub use error::{ApiError, ApiResultExt, DjangorsError, ErrorRenderer, JsonErrorRenderer};
pub use groups::{GroupHandle, Groups};
pub use handler::{Handler, StreamingHandler};
pub use pagination::{decode_cursor, encode_cursor, CursorError, CursorPage, Paginator};
pub use path_params::PathParams;
pub use ratelimit::{rate_limited, ByIp, RateLimitKey, RateLimiter};
pub use request::Request;
pub use response::Response;
pub use router::{MatchedRoute, Router};
pub use settings::DjangorsSettings;
pub use sse::StreamingResponse;
pub use state::AppState;

/// Re-export of [`hyper::StatusCode`] for convenience.
pub use hyper::StatusCode;

/// Escapes HTML special characters in string input.
// FREEZE-REVIEW: This cannot be made `pub(crate)` in this pass because the out-of-scope
// `djangors-admin` crate still calls it directly in production code; hiding it without first
// migrating that consumer would break the required workspace build. Revisit in Freeze Review Pass 2.
pub fn html_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            '/' => escaped.push_str("&#x2F;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Re-export of `inventory` for use by `#[management_command]` macro generated code.
pub use inventory;

/// A registration record for custom management commands registered via `#[management_command]`.
#[allow(clippy::type_complexity)]
pub struct ManagementCommandRegistration {
    /// The name of the command (e.g. `"seed"` or `"loaddata"`).
    pub name: &'static str,
    /// The async handler function that receives the raw CLI args after the command name.
    pub handler: fn(Vec<String>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
}

inventory::collect!(ManagementCommandRegistration);

/// If enabled, look up the named custom management command in the inventory, run it, and exit.
///
/// Mirrors [`introspect_models_if_requested`] — checked via `DJANGORS_RUN_COMMAND` env var.
/// Called (and awaited) from the user project's `main()` right alongside
/// `introspect_models_if_requested`. This is `async` rather than spinning its own
/// `tokio::runtime::Runtime` because `main()` is already running inside a `#[tokio::main]`
/// runtime by the time this is called — starting a second nested runtime via `block_on`
/// panics ("Cannot start a runtime from within a runtime").
pub async fn run_management_command_if_requested() {
    let command_name = match std::env::var("DJANGORS_RUN_COMMAND") {
        Ok(name) => name,
        Err(_) => return,
    };

    let handler = inventory::iter::<ManagementCommandRegistration>()
        .find(|reg| reg.name == command_name)
        .map(|reg| reg.handler);

    match handler {
        Some(handler) => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            handler(args).await;
            std::process::exit(0);
        }
        None => {
            eprintln!("[dj] error: unknown management command '{command_name}'");
            std::process::exit(1);
        }
    }
}

/// If enabled, print this binary's compiled model registry and terminate.
pub fn introspect_models_if_requested() {
    if std::env::var("DJANGORS_INTROSPECT_MODELS").ok().as_deref() == Some("1") {
        println!(
            "{}",
            serde_json::to_string(&djangors_orm::meta::registered_models_snapshot())
                .expect("model metadata is serializable")
        );
        std::process::exit(0);
    }
}

#[cfg(test)]
#[path = "integration_tests.rs"]
mod integration_tests;
