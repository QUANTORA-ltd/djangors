use chrono::{DateTime, Utc};
use djangors::db::{Database, DbError};
use djangors_macros::Model;

use crate::djangors_orm;

#[derive(Model, Debug, Clone, serde::Serialize)]
#[djangors(app = "orders", table_name = "orders_order", soft_delete)]
pub struct Order {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 64, unique)]
    pub reference: String,

    pub amount_cents: i64,

    #[djangors(max_length = 32)]
    pub status: String,

    pub created_at: DateTime<Utc>,

    pub deleted_at: Option<DateTime<Utc>>,
}

/// Creates the `orders_order` table if it does not already exist.
pub async fn create_orders_table(db: &Database) -> Result<(), DbError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk_type = dialect.auto_pk_type();
    let ts_type = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS orders_order (
            id {pk_type},
            reference VARCHAR(64) NOT NULL UNIQUE,
            amount_cents BIGINT NOT NULL,
            status VARCHAR(32) NOT NULL,
            created_at {ts_type} NOT NULL,
            deleted_at {ts_type}
        );"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(DbError::QueryFailed)?;
    Ok(())
}
