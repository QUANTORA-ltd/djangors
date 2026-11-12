#![deny(missing_docs)]
//! HTTP kernel for the Djangors web framework.
//!
//! Provides the core [`Request`], [`Response`], [`Router`], and [`Handler`]
//! types that form the foundation of Djangors's HTTP layer.
//!
//! # CSRF Protection
//!
//! [`middleware::csrf_layer`] validates unsafe requests (`POST`, `PUT`, `PATCH`, `DELETE`)
//! using a header-first, form-body-fallback scheme:
//! 1. **Cookie**: A `csrftoken` cookie holds the client's token.
//! 2. **Header Check (`X-CSRFToken`)**: Checked first on all unsafe requests (ideal for JSON/AJAX/HTMX requests).
//! 3. **Form Body Fallback (`csrfmiddlewaretoken`)**: If the `X-CSRFToken` header is absent on an
//!    `application/x-www-form-urlencoded` `POST` request, the `Form<T>` extractor and form handlers
//!    verify the `csrfmiddlewaretoken` hidden form field against the `csrftoken` cookie.
//!
//! **BREACH Defense (Future Work):**
//! This version uses a constant-time double-submit cookie comparison. Django's BREACH-hardened
//! per-request masked-secret scheme is planned as future work.

extern crate self as djangors_core;

/// Application builder and launcher logic.
pub mod app;
/// `VerifiedCaptcha` extractor for JSON APIs (feature `captcha`).
#[cfg(feature = "captcha")]
pub mod captcha;
/// Django-style HTML debug and error page rendering.
pub mod debug_page;
/// Dev-only request and SQL query inspection panel (`debug-panel` feature).
#[cfg(feature = "debug-panel")]
pub mod debug_panel;
/// Framework core error types.
pub mod error;
/// Request parameter extractor utilities.
pub mod extract;
/// Pub/sub broadcast group channel registry.
pub mod groups;
/// Cross-server broadcast groups over PostgreSQL `LISTEN`/`NOTIFY` (`postgres` feature).
#[cfg(feature = "postgres")]
pub mod groups_pg;
/// Handler trait definitions for standard and streaming HTTP routes.
pub mod handler;
/// Liveness (`/healthz`) and readiness (`/readyz`) health-check endpoints.
pub mod health;
/// HTTP request/response logging middleware.
pub mod logging;
/// Standard HTTP middleware layers (CSRF, Request ID, etc.).
pub mod middleware;
/// Pagination utility engines.
pub mod pagination;
/// Path parameter map container.
pub mod path_params;
/// Named, opt-in endpoint rate limiting.
#[cfg(feature = "cache")]
pub mod ratelimit;
/// Request structure wrapper.
pub mod request;
/// Resilience primitives: retries, timeouts, and circuit breakers.
pub mod resilience;
/// Response structure wrapper.
pub mod response;
/// URL routing engine and pattern matching.
pub mod router;
/// Secrets providers (`EnvProvider`, `DotenvProvider`, Vault, GCP, AWS) and interpolation.
#[cfg(feature = "secrets")]
pub mod secrets;
/// Tower service implementations for Djangors applications.
pub mod service;
/// Framework-wide settings container.
pub mod settings;
/// Framework lifecycle event signals.
pub mod signals;
/// Django-style signing (`Signer`, `TimestampSigner`, `dumps`/`loads`) and signed URLs.
pub mod signing;
/// Server-Sent Events (SSE) streaming response types.
pub mod sse;
/// Type-map state storage container.
pub mod state;
/// API versioning extractors and deprecation response middleware.
pub mod versioning;
/// RFC 6455 WebSocket upgrade and frame protocol (`ws` feature).
#[cfg(feature = "ws")]
pub mod ws;

pub use app::Djangors;
#[cfg(feature = "debug-panel")]
pub use debug_panel::{debug_panel_layer, DebugPanelLayer, DebugPanelService};
pub use error::{
    ApiError, ApiResultExt, DjangorsError, ErrorRenderer, JsonErrorRenderer, ProblemDetailsRenderer,
};
pub use groups::{GroupHandle, Groups};
#[cfg(feature = "postgres")]
pub use groups_pg::PgGroups;
pub use handler::{Handler, StreamingHandler};
#[cfg(feature = "cache")]
pub use health::CacheCheck;
pub use health::{DatabaseCheck, HealthCheck, HealthChecks};
pub use pagination::{decode_cursor, encode_cursor, CursorError, CursorPage, Paginator};
pub use path_params::PathParams;
#[cfg(feature = "cache")]
pub use ratelimit::{rate_limited, ByIp, RateLimitKey, RateLimiter};
pub use request::Request;
#[cfg(feature = "resilience")]
pub use resilience::{retry, RetryPolicy};
pub use resilience::{with_timeout, BreakerError, CircuitBreaker, CircuitState, TimedOut};
pub use response::Response;
pub use router::Router;
pub use settings::DjangorsSettings;
pub use sse::StreamingResponse;
pub use state::AppState;
pub use versioning::{ApiVersion, DeprecationLayer, DeprecationService, VersionSource, Versioning};
#[cfg(feature = "ws")]
pub use ws::{
    upgrade_websocket, upgrade_websocket_with_policy, OriginPolicy, WebSocket, WsHandler, WsMessage,
};

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

    if command_name == "rotate-encryption-keys" {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Err(err) = run_rotate_encryption_keys(&args).await {
            eprintln!("[dj rotate-encryption-keys] error: {err}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

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

async fn run_rotate_encryption_keys(args: &[String]) -> Result<(), String> {
    let mut model: Option<String> = None;
    let mut batch: usize = 500;
    let mut database_url: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(val) = arg.strip_prefix("--model=") {
            model = Some(val.to_string());
        } else if arg == "--model" {
            i += 1;
            model = args.get(i).cloned();
        } else if let Some(val) = arg.strip_prefix("--batch=") {
            batch = val
                .parse::<usize>()
                .map_err(|e| format!("invalid --batch value '{val}': {e}"))?;
        } else if arg == "--batch" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| "missing value after --batch".to_string())?;
            batch = val
                .parse::<usize>()
                .map_err(|e| format!("invalid --batch value '{val}': {e}"))?;
        } else if let Some(val) = arg.strip_prefix("--database-url=") {
            database_url = Some(val.to_string());
        } else if arg == "--database-url" {
            i += 1;
            database_url = args.get(i).cloned();
        }
        i += 1;
    }

    let model = model.ok_or_else(|| "missing required --model <app.Model> argument".to_string())?;
    let url = match database_url {
        Some(u) if !u.trim().is_empty() => u,
        _ => std::env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL environment variable is not set".to_string())?,
    };
    let config = djangors_db::DatabaseConfig::new(url);
    let db = djangors_db::Database::connect(&config)
        .await
        .map_err(|e| format!("failed to connect to database: {e}"))?;
    let report = djangors_orm::encryption::rotate_registered_model_keys(&db, &model, batch)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "[dj rotate-encryption-keys] table={}: scanned={}, updated={}, batches={}",
        report.table_name, report.rows_scanned, report.rows_updated, report.batches
    );
    Ok(())
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
