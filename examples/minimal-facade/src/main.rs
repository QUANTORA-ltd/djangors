#![deny(missing_docs)]
//! Minimal Postgres-only Djangors application built against the `djangors` facade
//! with `default-features = false, features = ["postgres"]`.

use djangors::core::health::{DatabaseCheck, HealthChecks};
use djangors::core::{
    Djangors, DjangorsError, DjangorsSettings, PathParams, Request, Response, Router, StatusCode,
};
use djangors::db::{config::DatabaseConfig, Database};
use djangors::orm::{self as djangors_orm, Model};
use djangors_macros::Model;

/// Minimal model exercising `#[derive(Model)]` and direct `Model::write_json` serialization.
#[derive(Model, Debug, Clone)]
#[djangors(app = "minimal", table_name = "minimal_item")]
pub struct Item {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Item label.
    #[djangors(max_length = 120)]
    pub title: String,
}

async fn list_items(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("Database not found in state".into()))?;
    let items = Item::objects()
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let mut buf = Vec::with_capacity(items.len().saturating_mul(64).max(2));
    buf.push(b'[');
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            buf.push(b',');
        }
        item.write_json(&mut buf);
    }
    buf.push(b']');
    Ok(Response::json_bytes(StatusCode::OK, buf))
}

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    let (settings, _) = DjangorsSettings::load()?;
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/djangors".into());
    let db = Database::connect(&DatabaseConfig::new(db_url))
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let router = Router::new()
        .health(HealthChecks::new().with(DatabaseCheck::new(db.clone())))
        .get("/items", list_items)
        .with_state(db);

    Djangors::new(settings, router).run().await
}
