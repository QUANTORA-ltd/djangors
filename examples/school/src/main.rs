use djangors_core::{Djangors, DjangorsError, DjangorsSettings, Router};
use school::urls;

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    djangors_core::introspect_models_if_requested();
    djangors_core::run_management_command_if_requested().await;
    djangors_core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {
        eprintln!("settings warning: {w}");
    }

    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };

    let mut config = djangors_db::config::DatabaseConfig::new(db_url);
    if let Ok(raw) = std::env::var("DJANGORS_MAX_CONNECTIONS") {
        let max = raw.parse().map_err(|_| {
            DjangorsError::Internal("DJANGORS_MAX_CONNECTIONS must be an integer".to_string())
        })?;
        config = config.max_connections(max);
    }
    let db = djangors_db::Database::connect(&config)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let router = urls::urls().with_state(db);
    let router_service = djangors_core::router::RouterService::new(router, settings.debug);

    let secret_key = if settings.secret_key.is_empty() {
        "dev-only-secret-key-at-least-32-bytes-long-for-signing-cookies".to_string()
    } else {
        settings.secret_key.clone()
    };

    let cors = djangors_core::middleware::cors_layer(
        &djangors_core::middleware::CorsConfig::from_settings(&settings),
    )?;

    let service = tower::ServiceBuilder::new()
        .layer(cors)
        .layer(djangors_core::middleware::security_headers_layer())
        .layer(djangors_sessions::SessionLayer::new(
            djangors_sessions::SignedCookieStore::new(secret_key.as_bytes())
                .with_secure(!settings.debug),
        ))
        .layer(djangors_core::middleware::csrf_layer().with_secure(!settings.debug))
        .service(router_service);

    Djangors::new(settings, Router::new())
        .run_service(service)
        .await
}
