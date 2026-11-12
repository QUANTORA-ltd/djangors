# How to Make Payments and Other POSTs Safe to Retry

Wrap a Tower service with `IdempotencyLayer` and have clients send a unique
`Idempotency-Key` header for each logical operation. A completed response is cached and
replayed for retries; reusing a key with a different request body returns 422.

```rust,illustrative
use std::{sync::Arc, time::Duration};
use djangors_cache::{IdempotencyLayer, InMemoryCache};
use tower::ServiceBuilder;

let cache = Arc::new(InMemoryCache::default());
let service = ServiceBuilder::new()
    .layer(IdempotencyLayer::new(cache, Duration::from_secs(86_400)))
    .service(router_service);
// Serve `service` in place of `router_service`.
```

Use a shared cache such as `DatabaseCache` or `RedisCache` when requests can reach
multiple application instances. Cache failures fail closed with 503 so a payment handler
does not run when duplicate protection cannot be established.
