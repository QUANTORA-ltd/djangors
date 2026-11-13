# How to Make Payments and Other POSTs Safe to Retry

## Problem

A client sends "charge this card" and the connection drops before the response arrives.
The client can't tell whether the charge happened. If it retries, the customer may be
charged twice; if it doesn't, the order may never be paid.

## Solution

Have clients send an `Idempotency-Key` header, a unique value per operation such as a
UUID, and wrap your app in `IdempotencyLayer`. This is the scheme Stripe's API uses:

- The first request with a key runs normally, and its response is stored.
- A retry with the same key and the same body gets the stored response back, with the
  header `Idempotent-Replayed: true`. The handler does not run again.
- A retry that arrives while the first request is still running gets `409 Conflict`.
- The same key with a different body gets `422`: that's a client bug, not a retry.
- A `5xx` response is not stored, so the client can retry an operation that failed.

Only `POST`, `PUT`, `PATCH`, and `DELETE` requests that carry the header are affected;
everything else passes straight through.

```rust,compile
use std::sync::Arc;
use std::time::Duration;
use djangors_cache::{DatabaseCache, IdempotencyLayer};
use djangors_core::router::RouterService;
use djangors_core::{Djangors, DjangorsError, DjangorsSettings, Router};
use djangors_db::Database;
use tower::Layer;

async fn serve(settings: DjangorsSettings, router: Router, db: Database) -> Result<(), DjangorsError> {
    let service = RouterService::new(router.clone(), settings.debug);
    // Keep records for 24 hours, in the database so every app instance shares them.
    let idempotent = IdempotencyLayer::new(Arc::new(DatabaseCache::new(db)), Duration::from_secs(86_400))
        .layer(service);

    let app = Djangors::new(settings, router);
    let listener = app.bind().await?;
    app.serve_service(listener, idempotent).await
}
```

From the client:

```bash
curl -X POST https://example.com/api/payments \
  -H "Idempotency-Key: 3f1c9a52-6b8e-4c11-9d0f-2a7e5b4c8d10" \
  -d '{"amount": 5000}'
```

## Choosing a cache

The cache must be shared by every instance of your app, or a retry that reaches a
different instance runs again. Use `DatabaseCache` or `RedisCache` in production;
`InMemoryCache` is only safe with a single process. All three implement the atomic
"store if absent" operation (`Cache::add`) the layer needs. If the cache can't be reached,
the layer answers `503` without running the handler, so duplicate protection is never
silently skipped.

## Whose key is it?

Keys are scoped to the method, the path, and the caller, so two users who happen to send
the same key never see each other's responses. By default the caller is identified by
the `Authorization` and `Cookie` headers. If other cookies change between retries (an
analytics cookie, say), a retry would look like a different caller and run again. In that
case scope by just the session cookie, which the server issues and signs:

```rust,illustrative
let layer = IdempotencyLayer::new(cache, Duration::from_secs(86_400)).with_scope(|parts| {
    let cookies = parts.headers.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or("");
    cookies
        .split(';')
        .map(str::trim)
        .find(|c| c.starts_with("djangors_sessionid="))
        .unwrap_or("")
        .as_bytes()
        .to_vec()
});
```

Never scope by a header the client sets freely, such as a user ID sent by the client:
anyone could send someone else's ID and receive their stored responses.

Requests without the scoped credential share one scope. An anonymous visitor with no
session cookie has an empty session value, so all anonymous callers using the same key
and path see each other's stored responses. Require login for endpoints that use
idempotency keys, or scope by something every caller has, so that a missing credential
never becomes a shared scope.

## What is replayed

The stored response keeps its status code, body, `Content-Type`, and `Location`. Other
headers, including `Set-Cookie`, are not replayed.

## Body size limit

The layer reads the whole request body to compare retries, so it applies its own limit:
10 MiB by default. A larger body gets `413 Payload Too Large` before your handler runs,
whether or not it carries an `Idempotency-Key`. A `Content-Length` over the limit is
refused without reading the body. Change the limit with `.max_body_bytes(...)`:

```rust,illustrative
let idempotent = IdempotencyLayer::new(cache, Duration::from_secs(86_400))
    .max_body_bytes(1024 * 1024);
```
