use std::sync::Arc;
use std::time::Duration;

use djangors::cache::{IdempotencyLayer, InMemoryCache};
use djangors::core::middleware::{cors_layer, security_headers_layer, CorsConfig};
use djangors::core::router::RouterService;
use djangors::core::versioning::DeprecationLayer;
use djangors::core::{Djangors, DjangorsError, DjangorsSettings, Router};
use djangors::db::config::DatabaseConfig;
use djangors::db::Database;
use djangors::rest::{create_api_key, create_api_key_table};
use djangors::tasks::create_task_table;
use tower::ServiceBuilder;

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    djangors::core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {
        eprintln!("settings warning: {w}");
    }

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
    let mut config = DatabaseConfig::new(db_url.clone());
    if db_url.contains(":memory:") {
        config = config.max_connections(1).min_connections(1);
    }

    let db = Database::connect(&config)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    rest_api::models::create_orders_table(&db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    create_api_key_table(&db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    create_task_table(&db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let (_key, raw_key) =
        create_api_key(&db, "demo-client", &["orders:read", "orders:write"], None)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    println!("Demo X-Api-Key: {raw_key}");

    let cache = Arc::new(InMemoryCache::new(1024));
    let router = rest_api::urls::urls(db, Arc::clone(&cache));
    let router_service = RouterService::new(router, settings.debug);
    let cors = cors_layer(&CorsConfig::from_settings(&settings))?;

    let service = ServiceBuilder::new()
        .layer(cors)
        .layer(security_headers_layer())
        .layer(DeprecationLayer::new(rest_api::urls::api_versioning()))
        .layer(IdempotencyLayer::new(cache, Duration::from_secs(300)))
        .service(router_service);

    Djangors::new(settings, Router::new())
        .run_service(service)
        .await
}
