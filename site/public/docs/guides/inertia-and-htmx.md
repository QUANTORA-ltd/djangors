# Inertia.js, htmx & AI SSE Streaming

## Inertia.js Protocol Adapter

`djangors_core::Inertia` lets you build React, Vue, or Svelte frontends backed by standard Djangors routes, sessions, and CSRF protection without maintaining a separate REST API:

```rust,compile
use djangors_core::{DjangorsError, Inertia, InertiaConfig, PathParams, Request, Response};

pub async fn dashboard(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let inertia = Inertia::with_config(
        InertiaConfig::new("v1.1").with_shared_prop("appName", "Djangors")?,
    );
    inertia.render(&req, "Dashboard/Index", serde_json::json!({ "users": 42 }))
}
```

- Browser requests receive `200 OK` HTML with `<div id="app" data-page="..."></div>`.
- Inertia XHR requests (`X-Inertia: true`) receive JSON with `X-Inertia: true` and `Vary: X-Inertia`.
- Partial reloads (`X-Inertia-Partial-Component` + `X-Inertia-Partial-Data`) filter props automatically.
- Stale `X-Inertia-Version` on `GET` returns `409 Conflict` with `X-Inertia-Location`.

## htmx Request & Response Helpers

`Request` and `Response` include ergonomic htmx helpers:

- `Request`: `is_htmx()`, `is_htmx_boosted()`, `htmx_target()`, `htmx_trigger()`, `htmx_trigger_name()`, `htmx_prompt()`, `htmx_current_url()`.
- `Response`: `hx_trigger()`, `hx_trigger_after_swap()`, `hx_trigger_after_settle()`, `hx_redirect()`, `hx_location()`, `hx_refresh()`, `hx_push_url()`, `hx_replace_url()`, `hx_retarget()`, `hx_reswap()`.

## AI / LLM Token Streaming (SSE)

```rust,illustrative
use djangors_core::StreamingResponse;

pub fn stream_tokens() -> StreamingResponse {
    let tokens = futures_util::stream::iter(vec!["Hello".to_string(), " world".to_string()]);
    StreamingResponse::llm_stream(tokens)
}
```

`StreamingResponse::llm_stream` sets `X-Accel-Buffering: no`, emits `data: {"token":...}\n\n` frames, and finishes with `data: [DONE]\n\n`. For custom events, use `SseEvent` with `StreamingResponse::sse_events` or `StreamingResponse::sse_json`.
