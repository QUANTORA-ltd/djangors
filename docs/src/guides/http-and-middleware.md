# HTTP Core and Middleware

Djangors's HTTP kernel (`djangors-core`) supplies the `Request`/`Response` types, the `Router`,
Tower middleware layers, error handling, pagination primitives, and logging setup that every
handler, admin site, and REST viewset builds on. The highest-level use is covered in the
tutorial; this guide documents the core surface directly against the source.

## Request

The [`Request`](https://docs.rs/djangors_core/latest/djangors_core/request/index.html) wraps a fully
buffered hyper request. Everything (including the body) is read eagerly at construction time, so
handlers take `Request` by value and are free to move it into spawned tasks.

Key methods (see `crates/djangors-core/src/request.rs`):

| Method | Signature | Notes |
| --- | --- | --- |
| `new` | `new(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Self` | Parses + URL-decodes the query string at construction |
| `method` | `fn(&self) -> &Method` | HTTP verb |
| `path` | `fn(&self) -> &str` | URI path, e.g. `"/hello/world"` |
| `header` | `fn(&self, name: &str) -> Option<&HeaderValue>` | Single header lookup |
| `headers` | `fn(&self) -> &HeaderMap` | All headers |
| `query` | `fn(&self, name: &str) -> Option<&str>` | Decoded query parameter |
| `raw_query` | `fn(&self) -> Option<&str>` | Undecoded query string, e.g. `"q=rust&page=2"` |
| `body_bytes` | `async fn(&self) -> &[u8]` | Buffered body (trivially async) |
| `state` | `fn<T: Send + Sync + 'static>(&self) -> Option<&T>` | Top-level app state (see below) |
| `require_state` | `fn<T>(&self) -> Result<&T, DjangorsError>` | Like `state`, but `DjangorsError::Internal` when missing |
| `ext` | `fn<T: Send + Sync + 'static>(&self) -> Option<&T>` | Per-request extension map |
| `with_state` | `fn(self, state: AppState) -> Self` | Attach app state |
| `with_extensions` | `fn(self, extensions: Extensions) -> Self` | Replace per-request extensions |
| `into_parts` | `fn(self) -> (Method, Uri, HeaderMap, Bytes)` | Consume and split |

### State vs ext: two very different kinds of typed value

Djangors has **two separate type maps** and the distinction matters:

- **`state`** is app-wide, configured **once** at startup via `Router::with_state(...)`, and stored in
  an [`AppState`](#appstate) on the request. Use it for long-lived singletons every handler needs —
  a `Database` connection pool, a `Groups` broadcast registry, a storage backend.
- **`ext`** is a **per-request** map of values inserted by middleware into the incoming hyper
  request's `Extensions` (which `Router::dispatch` propagates via `with_extensions`). Use it for
  request-scoped facts middleware computes — the resolved `Session`, `CurrentTenant`,
  `CsrfToken`, `ResolvedLocale`.

`state::<T>()` is persistent and shared; `ext::<T>()` is per-request scratch memory. A session
handle belongs in `ext`; the database pool it was loaded from belongs in `state`.

```rust,compile
# fn main() {
use djangors_core::{Request, PathParams, Response, Router, DjangorsError, StatusCode};

#[derive(Clone, Debug)]
struct Database;

// require_state() errors with a descriptive DjangorsError if the type
// was never attached — a wiring mistake surfaces fast at first request.
async fn index(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = req.require_state::<Database>()?;
    let _ = db;
    Ok(Response::text(StatusCode::OK, "ok"))
}

let router = Router::new()
    .with_state(Database)
    .get("/", index);
# let _ = router;
# }
```

```rust,illustrative
use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode};
use djangors_core::middleware::CsrfToken;

async fn show_form(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    // Middleware injected this per-request (via csrf_layer).
    let token = req.ext::<CsrfToken>().expect("csrf layer must be installed");
    Ok(Response::html(
        StatusCode::OK,
        format!(r#"<input name="csrfmiddlewaretoken" value="{}">"#, token.0),
    ))
}
```

For an end-to-end `state` + `ext` example see the router tests (`test_router_app_state` and
`test_dispatch_propagates_hyper_extensions_to_handler` in `crates/djangors-core/src/router.rs`).

## Response

[`Response`](https://docs.rs/djangors_core/latest/djangors_core/response/index.html) is a fully buffered
response built with helper constructors and converted to hyper with `into_hyper`.

| Constructor | Signature | Notes |
| --- | --- | --- |
| `text` | `text(status: StatusCode, body: &str) -> Self` | `text/plain; charset=utf-8` |
| `html` | `html(status: StatusCode, body: impl Into<String>) -> Self` | `text/html; charset=utf-8` |
| `bytes` | `bytes(status: StatusCode, content_type: &str, body: Vec<u8>) -> Self` | Raw bytes, explicit content type |
| `json` | `json<T: Serialize>(status: StatusCode, value: &T) -> Result<Self, DjangorsError>` | `application/json; charset=utf-8`; `DjangorsError::Internal` on serialize failure |
| `redirect` | `redirect(location: &str) -> Self` | 302 Found + `Location` |
| `sse` | `sse<S>(stream: S) -> StreamingResponse` where `S: Stream<Item = String>` | Builds an SSE stream (see [How to Stream Server-Sent Events](../how-to/stream-sse.md)) |
| `header` | `header(name: &str, value: &str) -> Self` | Builder, overwrites existing value |
| `into_hyper` | `into_hyper(self) -> hyper::Response<Full<Bytes>>` | Converts for the server/`Service` path |
| `into_hyper_boxed` | `into_hyper_boxed(self) -> hyper::Response<BoxBody<Bytes, Infallible>>` | Boxed-body variant |
| `status` / `headers` / `body` | accessors | Read back the buffered pieces |

```rust,compile
# fn main() {
use djangors_core::{Response, StatusCode};
use serde_json::json;

let json_resp = Response::json(StatusCode::OK, &json!({ "count": 3 })).unwrap();

let bytes_resp = Response::bytes(
    StatusCode::OK,
    "image/png",
    vec![1, 2, 3, 4],
);

let html_resp = Response::html(StatusCode::NOT_FOUND, "<h1>nope</h1>");
let redirect = Response::redirect("/login");
let customized = Response::text(StatusCode::OK, "hello").header("X-Trace", "abc");
# let _ = (json_resp, bytes_resp, html_resp, redirect, customized);
# }
```

```rust,compile
# fn main() {
use djangors_core::{Response, StatusCode};

// json() returns Result — ? works from any handler returning DjangorsError.
fn build_status_response() -> Result<Response, djangors_core::DjangorsError> {
    Ok(Response::json(
        StatusCode::OK,
        &serde_json::json!({ "healthy": true }),
    )?)
}
# let _ = build_status_response();
# }
```

## AppState

[`AppState`](https://docs.rs/djangors_core/latest/djangors_core/state/index.html) is a type-erased map of
`Send + Sync + 'static` values indexed by `TypeId`. It is clone-cheap (wraps the map in an `Arc`),
which is why it can be copied onto every request. `Router::with_state` stores one; `req.state::<T>()`
reads from it.

| Method | Signature | Notes |
| --- | --- | --- |
| `new` | `new() -> Self` | Empty state |
| `insert` | `insert<T>(self, value: T) -> Self` | Builder-style; returns a new `AppState` |
| `get` | `get<T>(&self) -> Option<&T>` | Lookup by type |
| `contains` | `contains<T>(&self) -> bool` | Type check |
| `len` | `fn(&self) -> usize` | Number of distinct types |
| `is_empty` | `fn(&self) -> bool` | No state attached |
| `merge` | `merge(self, other: &AppState) -> Self` | Union; `self` wins on type conflicts (used by `mount`) |

`insert` is used internally by `Router::with_state` and by `mount` when a sub-router wants to
inherit the parent's state (parent values win).

```rust,compile
# fn main() {
use djangors_core::AppState;

#[derive(Clone)]
struct Database;
#[derive(Clone)]
struct Cache;

let state = AppState::new()
    .insert(Database)
    .insert(Cache);

assert!(state.contains::<Database>());
assert_eq!(state.get::<Cache>().is_some(), true);
assert_eq!(state.len(), 2);

let merged = state.merge(&AppState::new().insert(Database));
assert!(!merged.is_empty());
# }
```

## Router extras

Beyond `get`/`post`/`put`/`delete`, the [`Router`](https://docs.rs/djangors_core/latest/djangors_core/router/index.html)
exposes a lower-level route API, named-route reversing, and mounting. Path syntax supports literal
segments, `{name}` (String capture), `{name:i64}`, and `{name:slug}`. `:name` is also accepted as a
shorthand alias for `{name}`, for muscle memory carried over from Express/Django URLconfs — prefer
the typed `{name}`/`{name:i64}`/`{name:slug}` forms in new code, since they validate the segment at
match time instead of leaving that to the handler.

| Method | Signature | Notes |
| --- | --- | --- |
| `route` | `route(path: &str, method: Method, handler: impl Handler)` | Register with an explicit method |
| `route_streaming` | `route_streaming(path: &str, method: Method, handler: impl StreamingHandler)` | Streaming handlers for non-GET too |
| `get_sse` / `sse` | `get_sse(path: &str, handler: impl StreamingHandler)` | SSE GET helper (alias: `sse`) |
| `name` | `name(name: &str) -> Self` | Name the most recently registered route (panics on duplicates) |
| `reverse` | `reverse(name: &str, params: &[(&str, &str)]) -> Result<String, ReverseError>` | Path reconstruction; values are percent-encoded |
| `mount` | `mount(prefix: &str, sub_router: Router) -> Self` | Prefixes every sub-router route; inherits sub-router state the parent lacks |

```rust,compile
# fn main() {
use djangors_core::{Request, PathParams, Response, Router, DjangorsError, StatusCode};
use hyper::Method;

async fn poll(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "poll"))
}

let router = Router::new()
    .route("/polls/{id:i64}", Method::GET, poll)
    .name("poll-detail")
    .get_sse("/polls/{id:i64}/live", |_req: Request, _params: PathParams| {
        async { Err(djangors_core::DjangorsError::NotFound) }
    })
    .mount("/api", Router::new().get("/users/{id}", poll).name("user-detail"));

assert_eq!(router.reverse("poll-detail", &[("id", "42")]).unwrap(), "/polls/42");
assert_eq!(router.reverse("user-detail", &[("id", "7")]).unwrap(), "/api/users/7");
# }
```

> [!NOTE]
> `get_sse` was used here to keep the snippet self-contained; a real SSE handler returns a
> `StreamingResponse`. See [How to Stream Server-Sent Events](../how-to/stream-sse.md).

**`ReverseError`** (in `djangors_core::router::ReverseError`) has three variants:
- `UnknownName(String)` — no route registered under that name.
- `MissingParam { route, param }` — the pattern needs a parameter the caller didn't supply.
- `UnexpectedParam { route, param }` — the caller supplied a parameter the pattern doesn't use.

Typed converters are not type-validated during reversing (`{id:i64}` won't reject `"abc"`), but
captured values are always percent-encoded before substitution.

## Middleware layers

All middleware lives in `djangors_core::middleware` and composes with
[`tower::ServiceBuilder`](https://docs.rs/tower/latest/tower/struct.ServiceBuilder.html)
around `RouterService::new(router, debug)` — the same stack shape as
[layer middleware in the Django world](tutorial/06-static-files-and-middleware.md).

| Layer | Builder | Behavior |
| --- | --- | --- |
| `logging_layer()` | `fn() -> TraceLayer` | `tower_http::trace`, logs method/path/status/latency via `tracing` |
| `security_headers_layer()` | `fn() -> SecurityHeadersLayer` | `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: same-origin` |
| `hsts_layer(max_age)` | `fn(max_age_secs: u64) -> HstsLayer` | `Strict-Transport-Security`; `.with_include_subdomains(bool)` (default off) |
| `normalize_path_layer()` | `fn() -> NormalizePathLayer` | Trims trailing slashes (Django `CommonMiddleware`-ish) |
| `compression_layer()` | `fn() -> CompressionLayer` | gzip + brotli response compression |
| `request_id_layer()` | `fn() -> RequestIdLayer` | Sets/propagates `X-Request-ID` (UUID) |
| `csrf_layer()` | `fn() -> CsrfLayer` | Double-submit cookie CSRF; `.with_cookie_name/.with_header_name/.with_secure` |

Also available: `HostValidationLayer::new(allowed_hosts)` and `CspBuilder` — both documented in
depth in the [Security guide](security.md).

The `CsrfLayer` injects a `CsrfToken(String)` extension on every request (plus a pending-check
marker for form bodies). Sessions and tenancy similarly communicate through `ext`; see
[Sessions and CSRF Protection](sessions.md) and [Multi-tenancy](multi-tenancy.md).

```rust,compile
# fn main() {
use djangors_core::middleware::{
    logging_layer, security_headers_layer, hsts_layer, normalize_path_layer,
    compression_layer, request_id_layer, csrf_layer,
};
use djangors_core::router::RouterService;
use tower::ServiceBuilder;

# let debug = false;
# let router = djangors_core::Router::new();
let router_service = RouterService::new(router, debug);

let service = ServiceBuilder::new()
    .layer(compression_layer()) // outermost; changes the body type
    .layer(logging_layer())
    .layer(request_id_layer())
    .layer(normalize_path_layer())
    .layer(security_headers_layer())
    .layer(hsts_layer(31536000).with_include_subdomains(true))
    .layer(csrf_layer().with_secure(!debug))
    .service(router_service);
# let _ = service;
# }
```

> [!IMPORTANT]
> **Layer order matters.** `compression_layer()` rewraps the response body, so anything that
> requires a plain `Full<Bytes>` body (e.g. `security_headers_layer`) must sit *inside* it —
> put compression first in the `ServiceBuilder` chain as shown above.
> `csrf_layer().with_secure(true)` (and `SignedCookieStore::with_secure`) must be enabled off HTTP
> in production — see [Security](security.md#csrf-protection-csrflayer).
> Serve the stack with `Djangors::serve_service(listener, service)`; the plain `serve()`/`run()`
> loops do not apply middleware layers.

## Request size limit

The router reads each request body into memory before the handler runs, so it caps how
large a body it will read. The default limit is 10 MiB.

A request whose body is larger than the limit gets `413 Payload Too Large`, and its handler
is not called. When the client sends a `Content-Length` header that is already larger than
the limit, the router refuses the request without reading the body. Without that header
(for example a chunked upload), the router reads until the body passes the limit and then
refuses it. WebSocket upgrade requests carry no body, so they are not affected.

The limit comes from `DJANGORS_MAX_BODY_BYTES`, or from
`DjangorsSettings::with_max_body_bytes`, when the application is built with
`Djangors::new`. A limit set on the router with `Router::max_body_bytes` takes precedence
over the settings value.

```rust,compile
use djangors_core::{Djangors, DjangorsSettings, Router};

fn main() {
    // Application-wide limit: 1 MiB for every route.
    let settings = DjangorsSettings::new().with_max_body_bytes(1024 * 1024);

    // This router keeps its own, smaller limit: 64 KiB.
    let router = Router::new().max_body_bytes(64 * 1024);

    let _app = Djangors::new(settings, router);
}
```

> [!IMPORTANT]
> The limit applies only to the body the router reads. Layers that buffer the body before
> the router sees it, such as `djangors_cache::IdempotencyLayer`, read the whole body first
> and are not covered. Set a body size limit at your reverse proxy as well, which protects
> those layers too.

## DjangorsError

[`DjangorsError`](https://docs.rs/djangors_core/latest/djangors_core/error/index.html) is the single error
type handlers return. Built-in variants map 1:1 to HTTP statuses, plus the application-defined
`Api` carrier:

| Variant | Status | Domain code |
| --- | --- | --- |
| `NotFound` | 404 | `not_found` |
| `BadRequest(String)` | 400 | `bad_request` |
| `Internal(String)` | 500 | `internal` |
| `Panicked(String)` | 500 | `panicked` |
| `Unauthorized(String)` | 401 | `unauthorized` |
| `Forbidden(String)` | 403 | `forbidden` |
| `TooManyRequests(String)` | 429 | `too_many_requests` |
| `Api(ApiError)` | `api.status` | `api.code` |

Every variant exposes `status_code()`, `code()`, `message()`, and `details()`.

### ApiError and constructors

`ApiError` is a struct with public fields `status: StatusCode`, `code: String`, `message: String`,
and `details: Option<serde_json::Value>` — the escape hatch for domain errors that need a custom
status/code/JSON payload.

- `DjangorsError::api(status, code, message)` builds the `Api` variant.
- `.with_details(json!(...))` attaches a structured payload; when chained onto a *built-in* variant
  it promotes it to `Api`, preserving its status, code, and message.

### ApiResultExt

`ApiResultExt<T>` is implemented for **any** `Result<T, E: Display>` and removes hand-written
`map_err` closures:

- `.api_err(status, code)` — uses the source error's `Display` text as the message.
- `.api_err_msg(status, code, message)` — fixed message, discards the source error's text (use when
  the underlying error leaks internals).

```rust,compile
# fn main() {
use djangors_core::DjangorsError;
use djangors_core::error::ApiResultExt;
use djangors_core::StatusCode;
use serde_json::json;

fn parse_quantity(raw: &str) -> Result<u32, DjangorsError> {
    raw.parse::<u32>().api_err(StatusCode::BAD_REQUEST, "invalid_quantity")
}

let err = DjangorsError::api(StatusCode::CONFLICT, "seat_taken", "That seat is already booked")
    .with_details(json!({ "seat": "12A", "flight": "DL404" }));

assert_eq!(err.status_code(), StatusCode::CONFLICT);
assert_eq!(err.code(), "seat_taken");
assert!(err.details().is_some());
# let _ = parse_quantity("x");
# }
```

### Custom rendering

By default the router renders errors as the JSON envelope `{"error": {status, code, message, details?}}`
when the caller accepts JSON or the error is an `Api`, otherwise as a debug page (when `debug`) or a
minimal production page. To take full control, implement the `ErrorRenderer` trait and register it with
`Router::set_error_renderer(renderer)`; there is a ready-made `JsonErrorRenderer`:

```rust,illustrative
use djangors_core::{DjangorsError, Request};
use djangors_core::error::{ErrorRenderer, JsonErrorRenderer};
use djangors_core::Response;

#[derive(Clone)]
struct MyErrorRenderer; // render every error as JSON, e.g. for a pure API

impl ErrorRenderer for MyErrorRenderer {
    fn render(&self, err: &DjangorsError, req: &Request) -> Response {
        JsonErrorRenderer.render(err, req)
    }
}

fn configure(router: djangors_core::Router) -> djangors_core::Router {
    router.set_error_renderer(MyErrorRenderer)
}
```

`set_error_renderer` is the ergonomic equivalent of the manual cast form:

```rust,illustrative
Router::new().with_state(Arc::new(MyRenderer) as Arc<dyn ErrorRenderer>)
```

The helper exists because the `as Arc<dyn ErrorRenderer>` cast is easy to omit, and
`with_state(Arc::new(MyRenderer))` compiles perfectly well while storing the value under its
concrete type — where the renderer lookup never finds it. The custom renderer is then silently
ignored, with no error at compile time or run time. That silent-failure mode is exactly what
`set_error_renderer` removes: it always performs the trait-object cast for you.

### Standard problem details (RFC 9457)

Djangors can also send errors in the standard format from
[RFC 9457](https://www.rfc-editor.org/rfc/rfc9457), with the `application/problem+json`
content type. Many API clients and gateways already understand this format, and the
[generated TypeScript client](code-generation.md#dj-openapi-and-the-typescript-client)
turns it into a typed `ProblemDetailsError`.

You don't have to configure anything for clients that ask for it: when a request's
`Accept` header includes `application/problem+json`, the default renderer answers in that
format. To use it for every error response, register `ProblemDetailsRenderer`:

```rust,compile
use djangors_core::{ProblemDetailsRenderer, Router};

fn configure(router: Router) -> Router {
    router.set_error_renderer(ProblemDetailsRenderer::new())
    // Or with your own documentation URLs for `type`:
    // router.set_error_renderer(ProblemDetailsRenderer::with_type_prefix("https://api.example.com/problems/"))
}
```

The `seat_taken` error from the example above, raised while handling `/flights/DL404/seats`,
renders as:

```json
{
  "type": "urn:djangors:error:seat_taken",
  "title": "Conflict",
  "status": 409,
  "code": "seat_taken",
  "detail": "That seat is already booked",
  "instance": "/flights/DL404/seats",
  "errors": { "seat": "12A", "flight": "DL404" },
  "details": { "seat": "12A", "flight": "DL404" }
}
```

`errors` and `details` carry the same data: `errors` is what problem-details clients look
for, and `details` matches the default Djangors JSON envelope.

Validation and login failures convert into the same shape with `?`:

| Error | Status | `code` |
| --- | --- | --- |
| `djangors_forms::FormErrors` | 422 | `validation_error`, with the field errors in `errors` |
| `AuthError::RateLimited` | 429 | `rate_limited` |
| `AuthError::AccountLocked` | 429 | `account_locked`, with `retry_after_secs` in `errors` |
| `AuthError::InvalidToken` | 401 | `invalid_token` |

To build one by hand, call `err.into_problem_response()`, or `err.write_problem_json(...)`
to write the body into your own buffer.

**Internal errors stay private.** `DjangorsError::Internal` and panics are always rendered
as `"Internal Server Error"` by `ProblemDetailsRenderer`, and by the JSON and problem+json
output of the default renderer whenever `debug` is off, so the real message, which might
contain SQL or file paths, never reaches the client. The router logs the original message
once at `error` level via `tracing` before rendering the response.

## Paginator and cursor primitives

[`Paginator`](https://docs.rs/djangors_core/latest/djangors_core/pagination/index.html) does the offset
math. `total_items` is clamped to ≥ 0 and `page_size` must be > 0; per the admin convention, 0 items
still yields 1 page.

| Method | Signature | Notes |
| --- | --- | --- |
| `new` | `new(total_items: i64, page_size: i64) -> Self` | Panics if `page_size <= 0` |
| `total_pages` | `fn(&self) -> i64` | Ceiling division; 0 items ⇒ 1 page |
| `offset` | `fn(&self, page: i64) -> i64` | 0-indexed SQL `OFFSET`, clamped into `[1, total_pages()]` |
| `has_previous` | `fn(&self, page: i64) -> bool` | `page > 1` |
| `has_next` | `fn(&self, page: i64) -> bool` | `page * page_size < total_items` |

Cursor helpers are the building blocks behind `djangors-rest` cursor pagination:

- `encode_cursor(pk: i64, order_value: Option<&str>) -> String` — base64 of `pk|order-value`
  (order value may itself contain `|`).
- `decode_cursor(cursor: &str) -> Result<(i64, Option<String>), CursorError>`.
- `CursorError` variants: `InvalidEncoding`, `InvalidFormat`, `InvalidPrimaryKey`.
- `CursorPage<T> { items: Vec<T>, next_cursor: Option<String>, previous_cursor: Option<String> }`.

```rust,compile
# fn main() {
use djangors_core::{
    Paginator, CursorPage, encode_cursor, decode_cursor,
};

let p = Paginator::new(250, 100);
assert_eq!(p.total_pages(), 3);
assert_eq!(p.offset(2), 100);
assert!(p.has_next(1));
assert!(p.has_previous(2));
assert!(!p.has_next(3));

let cursor = encode_cursor(42, Some("title|with|pipes"));
assert_eq!(
    decode_cursor(&cursor).unwrap(),
    (42, Some("title|with|pipes".to_string()))
);

let page: CursorPage<String> = CursorPage {
    items: vec!["a".into(), "b".into()],
    next_cursor: Some(cursor),
    previous_cursor: None,
};
assert_eq!(page.items.len(), 2);
# }
```

## Logging setup

Call exactly one of these **once**, early in `main()` (before starting the server). A second call
is a harmless no-op (only one global `tracing` subscriber can be installed).

| Function | Output | Notes |
| --- | --- | --- |
| `init_dev_logging()` | compact colored console output | Default level `"info,djangors_core=debug"` |
| `init_production_logging()` | structured JSON lines | Default level `"info,djangors_core=info"`, ideal for log aggregators |
| `init_production_logging_with_sentry(dsn)` *(`sentry` feature)* | JSON + Sentry capture | `ERROR` events → Sentry, `WARN..TRACE` → breadcrumbs, panics captured |

All three respect the standard `RUST_LOG` env var (e.g. `RUST_LOG=djangors_core=trace`).
`init_production_logging_with_sentry` returns a `ClientInitGuard` that **must be held for the
process lifetime**: `let _guard = init_production_logging_with_sentry(&dsn);`. An empty/invalid DSN
produces a disabled client rather than an error, so it is safe to call unconditionally.

```rust,compile
# fn main() {
use djangors_core::logging::{init_dev_logging, init_production_logging};

// Pick one per environment:
init_dev_logging();
// init_production_logging();
# }
```

## Management commands

`#[management_command(name = "...")]` (from `djangors_macros`, applied to an `async fn` taking
`Vec<String>` and returning nothing) registers a custom command through an `inventory` registry.
Two helper functions driven by env vars replace the CLI plumbing in `main()`:

- `run_management_command_if_requested()` — exits (with code 0) and runs the command named by
  `DJANGORS_RUN_COMMAND`; unknown names print an error and exit 1. **Async** — call it from
  `#[tokio::main]` (it relies on the runtime already installed).
- `introspect_models_if_requested()` — dumps the compiled model registry as JSON and exits, when
  `DJANGORS_INTROSPECT_MODELS=1`.

```rust,illustrative
use djangors_macros::management_command;

#[management_command(name = "seed")]
async fn seed(_args: Vec<String>) {
    println!("seeding database...");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    djangors_core::logging::init_dev_logging();

    // dj run ... --env DJANGORS_RUN_COMMAND=seed, or DJANGORS_INTROSPECT_MODELS=1
    djangors_core::run_management_command_if_requested().await;
    djangors_core::introspect_models_if_requested();

    // ... build settings + router, app.run().await
    Ok(())
}
```

```bash
DJANGORS_RUN_COMMAND=seed cargo run
DJANGORS_INTROSPECT_MODELS=1 cargo run   # prints JSON model registry
```

## html_escape

`djangors_core::html_escape(input: &str) -> String` escapes HTML-significant characters
(`<` `>` `&` `"` `'` and `/`) to their entities.

```rust,compile
# fn main() {
use djangors_core::html_escape;

let escaped = html_escape("<script>alert('x&y')</script>");
assert!(escaped.contains("&lt;script&gt;"));
assert!(escaped.contains("&amp;"));
assert!(escaped.contains("&#x27;"));
# }
```

## Where to go next

- [Requests and Responses tutorial](../tutorial/01-requests-and-responses.md) — the fast path to a first endpoint.
- [Security](security.md) — CSRF, HSTS, host validation, CSP, and the production checklist.
- [Sessions and CSRF Protection](sessions.md) — session middleware and `SignedCookieStore`.
- [How to Stream Server-Sent Events](../how-to/stream-sse.md) — `Response::sse` / `Router::get_sse` in practice.
- [REST Framework](rest.md) — where `Paginator`/cursor primitives surface as cursor pagination.
