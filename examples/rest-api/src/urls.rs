use std::sync::Arc;

use djangors::cache::InMemoryCache;
use djangors::core::health::{CacheCheck, DatabaseCheck, HealthChecks};
use djangors::core::resilience::CircuitBreaker;
use djangors::core::versioning::{VersionSource, Versioning};
use djangors::core::Router;
use djangors::db::Database;

use crate::views;

/// Returns the API [`Versioning`] policy: `/v1/*` is deprecated with a `Sunset`
/// header, `/v2/*` is current.
pub fn api_versioning() -> Versioning {
    Versioning::new(VersionSource::UrlPrefix)
        .default_version("2")
        .supported(["1", "2"])
        .deprecate("1", Some("Wed, 01 Jul 2027 00:00:00 GMT"))
}

/// Builds the application [`Router`] with health/readiness probes and versioned routes.
pub fn urls(db: Database, cache: Arc<InMemoryCache>) -> Router {
    let checks = HealthChecks::new()
        .with(DatabaseCheck::new(db.clone()))
        .with(CacheCheck::new(cache));

    Router::new()
        .health(checks)
        .get("/openapi.json", views::openapi_spec)
        .get("/v1/orders/", views::list_orders)
        .get("/v2/orders/", views::list_orders)
        .post("/v2/orders/", views::create_order)
        .get("/v2/quote/", views::upstream_quote)
        .with_state(db)
        .with_state(api_versioning())
        .with_state(CircuitBreaker::new())
}
