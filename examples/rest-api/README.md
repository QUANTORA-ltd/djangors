# `examples/rest-api` — Modern Djangors REST API Example

This example demonstrates Djangors 0.8–0.12 features in a self-contained SQLite-backed service using the `djangors` facade crate:

- **Health & readiness probes**: `Router::health(HealthChecks::new().with(DatabaseCheck::new(..)).with(CacheCheck::new(..)))` serving `/healthz` and `/readyz`.
- **CORS**: `cors_layer(&CorsConfig::from_settings(&settings))` wired in the Tower `ServiceBuilder` stack.
- **API versioning & deprecation**: `Versioning` (`VersionSource::UrlPrefix`), `ApiVersion` extractor, and `DeprecationLayer` emitting `Deprecation` and `Sunset` headers on `/v1/*`.
- **API key authentication & scopes**: `create_api_key_table`, `create_api_key`, `ApiKeyAuth` extractor, and scope enforcement (`orders:write`).
- **OpenAPI 3.1**: `OpenApiBuilder` with `.version("2.0.0")`, `.api_key_security()`, and `Order` schema at `/openapi.json`.
- **Idempotency**: `IdempotencyLayer` backed by `InMemoryCache`.
- **Distributed locks + transactional outbox**: `djangors::db::locks::with_lock` combined with `djangors::tasks::enqueue_in` inside the same database transaction.
- **Resilience**: `RetryPolicy`, `retry`, `with_timeout`, and `CircuitBreaker` on `/v2/quote/`.
- **Soft deletes**: `#[djangors(soft_delete)]` on `Order`.

## Running

```bash
cargo run -p rest-api
```

## Testing

```bash
cargo test -p rest-api
```
