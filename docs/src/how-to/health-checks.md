# How to Add Health and Readiness Checks

Load balancers and orchestrators like Kubernetes distinguish between **liveness** (`/healthz` — is the process alive?) and **readiness** (`/readyz` — can it reach its dependencies?).

## Registering `/healthz` and `/readyz`

Call `.health(checks)` on `Router`:

```rust,compile
# use std::sync::Arc;
# use std::time::Duration;
# use djangors_core::{DatabaseCheck, HealthChecks, Router};
# fn example(db: djangors_db::Database) -> Router {
let checks = HealthChecks::new()
    .timeout(Duration::from_secs(3))
    .with(DatabaseCheck::new(db));

Router::new().health(checks)
# }
```

- `GET /healthz` returns `200 OK` with `{"status":"ok"}` without running any checks.
- `GET /readyz` runs all registered checks concurrently, each with the configured timeout (3 seconds by default).
  - If all pass: `200 OK` with `{"status":"ok","checks":{"database":"ok"}}`.
  - If any fail or time out: `503 Service Unavailable` with `"status":"error"` and the failing check's message (or `"timed out"`).
- Both endpoints set `Cache-Control: no-store`.

## Checking a cache backend

With the `cache` feature enabled (on by default), `CacheCheck` verifies a `djangors_cache::Cache` by writing, reading, and deleting a random key with a 5-second TTL:

```rust,compile
# use std::sync::Arc;
# use djangors_core::{CacheCheck, HealthChecks, Router};
# fn example(cache: Arc<djangors_cache::InMemoryCache>) -> Router {
let checks = HealthChecks::new().with(CacheCheck::new(cache));
Router::new().health(checks)
# }
```

## Writing a custom check

Implement `HealthCheck` for any dependency (external API, message broker, disk space):

```rust,compile
# use djangors_core::HealthCheck;
struct PaymentGatewayCheck;

#[async_trait::async_trait]
impl HealthCheck for PaymentGatewayCheck {
    fn name(&self) -> &str {
        "payment_gateway"
    }

    async fn check(&self) -> Result<(), String> {
        Ok(())
    }
}
```

Keep error strings short and avoid including connection strings or credentials.
