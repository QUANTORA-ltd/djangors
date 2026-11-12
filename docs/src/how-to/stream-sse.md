# How to Stream Server-Sent Events (SSE)

## Problem
You want to stream real-time updates or events to web browsers over Server-Sent Events (SSE).

## Solution
Create a streaming response using `StreamingResponse::sse(stream)` or register an SSE endpoint on a `Router` using `.sse(path, handler)`.

## Code Example

```rust,compile
use async_stream::stream;
use tokio_stream::StreamExt;
use djangors_core::{Request, PathParams, DjangorsError, Router, DjangorsSettings, Djangors};
use djangors_core::sse::StreamingResponse;

// 1. Define SSE streaming handler
async fn live_events_handler(
    _req: Request,
    _params: PathParams,
) -> Result<StreamingResponse, DjangorsError> {
    // Generate an async stream of String events
    let event_stream = stream! {
        for i in 1..=5 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            yield format!("Event #{}", i);
        }
    };

    Ok(StreamingResponse::sse(event_stream))
}

// 2. Register endpoint on Router
fn build_sse_router() -> Router {
    Router::new().sse("/events", live_events_handler)
}

// 3. Start server
#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    let settings = DjangorsSettings::default();
    let app = Djangors::new(settings, build_sse_router());
    
    // Note: Must use standard serve()/run() loop
    app.run().await
}
```

### ⚠️ Real v1 Limitations & Constraints
- **Stream Format**: `StreamingResponse::sse` automatically formats each yielded string as an SSE data frame (`data: {item}\n\n`) and applies headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `Connection: keep-alive`.
- **Server Execution Path**: Streaming responses produce dynamic `BoxBody<Bytes, Infallible>` chunked streams. In v1, SSE streaming endpoints only work when served via the standard `serve()` / `run()` server loop, not custom `run_service()` middleware stacks (which expect fixed `Full<Bytes>` response bodies).

---

## In-Process Broadcast Groups

For fanning the same message out to many connected SSE clients (live dashboards, chat rooms,
notification streams), `djangors-core` ships an in-process pub/sub registry,
`djangors_core::Groups` (`crates/djangors-core/src/groups.rs`). Unlike a per-connection
stream, a `Groups` channel is a *broadcast* channel: one `send` reaches every subscriber.

| Method | Signature | Notes |
| --- | --- | --- |
| `Groups::new()` | `fn() -> Groups` | Default per-channel buffer capacity (1024) |
| `Groups::with_capacity(cap)` | `fn(capacity: usize) -> Groups` | Custom per-channel buffer capacity |
| `groups.group(name)` | `fn(&self, name: &str) -> GroupHandle` | Lazily creates the channel on first access |
| `handle.send(msg)` | `fn(&self, msg: impl Into<String>)` | Broadcast to every current subscriber; dropped silently if there are none |
| `handle.subscribe()` | `fn(&self) -> GroupStream` | An async `Stream<Item = String>` of messages |
| `handle.receiver_count()` | `fn(&self) -> usize` | Number of active subscribers |

```rust,compile
# use djangors_core::Groups;
# use tokio_stream::StreamExt;
# #[tokio::main]
# async fn main() {
let groups = Groups::with_capacity(64);   // per-channel buffer of 64
let feed = groups.group("live-events");   // lazily creates the channel

// A subscriber (e.g. an open SSE connection) gets a stream of messages.
let mut stream = feed.subscribe();
feed.send("event: kickoff".to_string());  // broadcast to all subscribers

assert_eq!(feed.receiver_count(), 1);     // one active receiver
assert_eq!(stream.next().await.as_deref(), Some("event: kickoff"));
# }
```

### Wiring SSE + POST around a group

The typical shape: a shared `GroupHandle` (frequently a `LazyLock` static, or attached to app state
via `Router::with_state` and read with `req.require_state::<Groups>()`), an SSE handler that
subscribes to it, and a POST handler that broadcasts to it.

```rust,illustrative
use djangors_core::{DjangorsError, Groups, PathParams, Request, Response, StatusCode};
use djangors_core::groups::GroupHandle;
use djangors_core::sse::StreamingResponse;
use std::sync::LazyLock;

// One registry shared app-wide; each named group is a broadcast channel.
static EVENTS: LazyLock<GroupHandle> = LazyLock::new(|| {
    Groups::new().group("live-events")
});

// GET /events/live — subscribe once; every broadcast fans out as a `data:` frame.
async fn events_live(_req: Request, _params: PathParams) -> Result<StreamingResponse, DjangorsError> {
    Ok(StreamingResponse::sse(EVENTS.subscribe()))
}

// POST /events — broadcast the request body to every connected SSE client.
async fn publish_event(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let body = String::from_utf8_lossy(req.body_bytes().await).into_owned();
    EVENTS.send(body);
    Ok(Response::text(StatusCode::OK, "published"))
}

let router = djangors_core::Router::new()
    .sse("/events/live", events_live)
    .post("/events", publish_event);
```

### Across several servers (PostgreSQL)

`Groups` only reaches subscribers in the same process. When you run several instances,
use `PgGroups` (the `postgres` feature) so that a `send` on one instance reaches the SSE
clients connected to every instance. Each instance starts a `PgGroups` on the same channel
name; `send` publishes through Postgres `LISTEN`/`NOTIFY`:

```rust,illustrative
use djangors_core::{DjangorsError, Groups, PgGroups};

async fn publish_event(pg_groups: &PgGroups, body: String) -> Result<(), DjangorsError> {
    // Reaches `live-events` subscribers on every instance, this one included.
    pg_groups.send("live-events", &body).await
}
```

The message is delivered back to the sender through `LISTEN`, so do not also call
`Groups::send` for the same message. Messages larger than 8000 bytes are rejected.

### Semantics to keep in mind
- **Fixed buffer**: each channel buffers up to its capacity. A slow subscriber that falls behind
  skips the missed messages (the `GroupStream` swallows `Lagged` errors) rather than blocking the
  sender.
- **Zero subscribers**: a `send` with no subscribers returns immediately and the message is
  dropped — no error.
- **Lower-level registration**: `Router::route_streaming(path, Method, handler)` registers a
  streaming handler for any HTTP method, and `Router::get_sse(path, handler)` is an alias for
  `.sse(path, handler)` (i.e. a GET-only `route_streaming`).
