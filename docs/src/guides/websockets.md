# WebSockets and Live Reload

Djangors has built-in RFC 6455 WebSockets: no separate ASGI server, and nothing like
Django Channels to install. A WebSocket endpoint is a route like any other, and it
runs in the same binary as the rest of your app.

The `ws` Cargo feature turns this on. It's enabled by default in both `djangors` and
`djangors-core`; builds with `default-features = false` need `features = ["ws"]`.

## A WebSocket endpoint

Register a handler with `Router::ws`. It runs once the handshake succeeds and receives
the upgraded `WebSocket` along with the original request and path parameters:

```rust,compile
use djangors_core::{PathParams, Request, Router, WebSocket, WsMessage};

async fn echo(mut ws: WebSocket, _req: Request, _params: PathParams) {
    // `recv` returns Ok(None) when the client closes the connection.
    while let Ok(Some(message)) = ws.recv().await {
        let sent = match message {
            WsMessage::Text(text) => ws.send_text(text).await,
            WsMessage::Binary(bytes) => ws.send_binary(&bytes).await,
            // Pings are answered automatically; nothing else to do.
            _ => Ok(()),
        };
        if sent.is_err() {
            break;
        }
    }
}

pub fn routes() -> Router {
    Router::new().ws("/ws/echo", echo)
}
```

From the browser:

```js
const socket = new WebSocket(`ws://${location.host}/ws/echo`);
socket.onmessage = (event) => console.log(event.data);
socket.onopen = () => socket.send("hello");
```

### Messages

| `WsMessage` variant | Meaning |
| :--- | :--- |
| `Text(String)` | A UTF-8 text message |
| `Binary(Vec<u8>)` | A binary message |
| `Ping(Vec<u8>)` / `Pong(Vec<u8>)` | Control frames. `recv` already answers each ping with a pong |
| `Close` | Send this to close the connection |

`WebSocket` has `recv`, `send`, `send_text`, `send_binary`, and `close` methods.
Messages split across several frames come back from `recv` as one message, up to
16 MiB. A client that breaks the protocol (sends unmasked frames, malformed control
frames, or an unknown opcode) is disconnected with close code 1002, and `recv` returns
an error.

## Broadcasting with groups

To push one message to many connected clients, such as notifications or a live
dashboard, combine a WebSocket handler with the in-process broadcast
[`Groups`](../how-to/stream-sse.md#in-process-broadcast-groups) that SSE uses:

```rust,compile
use djangors_core::{Groups, PathParams, Request, Router, WebSocket};
use tokio_stream::StreamExt;

async fn notifications(mut ws: WebSocket, req: Request, _params: PathParams) {
    let Some(groups) = req.state::<Groups>() else { return };
    let mut feed = groups.group("notifications").subscribe();
    while let Some(message) = feed.next().await {
        // A failed send means the client has gone away.
        if ws.send_text(message).await.is_err() {
            break;
        }
    }
}

pub fn routes(groups: Groups) -> Router {
    // Anywhere else in the app: groups.group("notifications").send("new order");
    Router::new()
        .with_state(groups)
        .ws("/ws/notifications", notifications)
}
```

`Groups` lives in one process. If you run several instances behind a load balancer,
each one only reaches its own clients unless you connect them with `PgGroups`. It sends
every message through Postgres `NOTIFY` and each instance delivers what it hears to its
local `Groups`:

```rust,illustrative
use djangors_core::{DjangorsError, Groups, PgGroups};
use djangors_orm::djangors_db::Database;

async fn start_groups(db: &Database) -> Result<PgGroups, DjangorsError> {
    // Every instance listens on the same channel with its own local `Groups`.
    let groups = Groups::new();
    PgGroups::start(db, groups, "app_events").await
}

// Any instance, including the one that sends:
// pg_groups.send("notifications", "new order").await?;
```

The sending instance also receives its own message through `LISTEN`, so call sites use only
`send`. `PgGroups` needs a PostgreSQL database; it returns an error on SQLite. Each started
`PgGroups` holds one connection from the pool for its `LISTEN`. Payloads must be under 8000
bytes, and larger messages return an error instead of being truncated.

## Cross-origin protection

Browsers attach the user's cookies to a WebSocket handshake even when another website
opens it, and CSRF protection doesn't run on the `GET` that starts a WebSocket. A
WebSocket that accepted any origin would let a malicious site act as the logged-in
user. This attack is called cross-site WebSocket hijacking.

So `Router::ws` only accepts browser handshakes whose `Origin` matches the request's
`Host`, and answers anything else with `403 Forbidden`. Clients that send no `Origin`
header, such as mobile apps, CLI tools, and server-to-server connections, are not
browsers and are allowed.

If your frontend lives on another domain, list it explicitly:

```rust,compile
# use djangors_core::{PathParams, Request, Router, WebSocket};
# async fn chat(_ws: WebSocket, _req: Request, _params: PathParams) {}
use djangors_core::OriginPolicy;

pub fn routes() -> Router {
    Router::new().ws_with_origin_policy(
        "/ws/chat",
        OriginPolicy::Allow(vec!["https://app.example.com".to_string()]),
        chat,
    )
}
```

`OriginPolicy::Any` accepts every origin. Use it only for sockets that send nothing
private and act on nothing a user is allowed to do.

To upgrade from inside your own handler, for example after checking a permission, call
`djangors_core::upgrade_websocket(&req, params, handler)` or
`upgrade_websocket_with_policy` and return the `Response` it gives you.

## Live reload in `dj run`

`dj run` rebuilds and restarts your app whenever a file under `src/` or `templates/`
changes, and now also refreshes the open browser tabs:

1. `dj run` starts a small live-reload WebSocket server on the next port up (8001 when
   your app is on 8000) and passes its port to your app in the
   `DJANGORS_LIVERELOAD_PORT` environment variable.
2. While that variable is set, every full HTML page built with `Response::html` gets a
   short script added before `</body>` that connects to the live-reload server. HTML
   fragments, such as htmx partials, are left alone.
3. After each successful rebuild, `dj run` tells connected browsers to reload.

There's nothing to configure. In production nothing sets `DJANGORS_LIVERELOAD_PORT`,
so no script is added.
