#![deny(missing_docs)]
//! Customer-facing outbound webhooks for Djangors.
//!
//! Provides [`WebhookEndpoint`] and [`WebhookDelivery`] models, HMAC-SHA256
//! signing and constant-time replay-window verification ([`sign_payload`],
//! [`verify_signature`]), [`WebhookDispatcher`] for emitting, delivering with
//! exponential backoff retries, and 1-click replaying deliveries, plus
//! [`create_webhooks_tables`] and optional [`register_admin`].

use chrono::{DateTime, Duration, Utc};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_orm::{q, Model as _, OrmError};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

/// HTTP header carrying the HMAC-SHA256 webhook signature (`t=...,v1=...`).
pub const SIGNATURE_HEADER: &str = "X-Djangors-Signature";

/// HTTP header carrying the webhook event name.
pub const EVENT_HEADER: &str = "X-Djangors-Event";

/// Delivery status for queued or retry-scheduled deliveries.
const STATUS_PENDING: &str = "pending";
/// Delivery status for successfully acknowledged deliveries (2xx response).
const STATUS_DELIVERED: &str = "delivered";
/// Delivery status for deliveries that exhausted `max_attempts`.
const STATUS_FAILED: &str = "failed";

type HmacSha256 = Hmac<Sha256>;

/// Errors returned by webhook signing, verification, storage, and delivery operations.
#[derive(Debug, Error)]
pub enum WebhookError {
    /// Database driver or connection error.
    #[error("database error: {0}")]
    Db(#[from] djangors_db::DbError),
    /// ORM query or persistence error.
    #[error("orm error: {0}")]
    Orm(#[from] OrmError),
    /// JSON serialization or deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Outbound HTTP client error.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    /// Signature header is malformed or does not match the payload and secret.
    #[error("invalid webhook signature")]
    InvalidSignature,
    /// Signature timestamp is outside the allowed replay-protection window.
    #[error("webhook signature timestamp expired")]
    ExpiredTimestamp,
    /// Requested webhook endpoint or delivery record was not found.
    #[error("webhook record not found")]
    NotFound,
}

/// Registered customer webhook destination.
#[derive(Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[djangors(app = "djangors_webhooks", table_name = "djangors_webhook_endpoint")]
pub struct WebhookEndpoint {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Application-defined owner identifier (e.g. tenant or user ID).
    #[djangors(max_length = 128, db_index)]
    pub owner_id: String,
    /// Destination HTTPS/HTTP URL to receive `POST` webhook deliveries.
    #[djangors(max_length = 1024)]
    pub url: String,
    /// Shared secret used to compute HMAC-SHA256 signatures.
    #[djangors(max_length = 255)]
    pub secret: String,
    /// JSON-serialized `Vec<String>` of subscribed event names (supports `"*"`).
    pub events_json: String,
    /// Whether this endpoint is active and should receive emitted events.
    pub enabled: bool,
    /// Human-readable description of this endpoint.
    pub description: String,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
}

/// Outbound webhook delivery attempt record and retry state.
#[derive(Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[djangors(app = "djangors_webhooks", table_name = "djangors_webhook_delivery")]
pub struct WebhookDelivery {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Foreign key ID of the target [`WebhookEndpoint`].
    #[djangors(db_index)]
    pub endpoint_id: i64,
    /// Event name (e.g. `"invoice.paid"`).
    #[djangors(max_length = 128)]
    pub event: String,
    /// JSON-serialized event payload body.
    pub payload_json: String,
    /// Current delivery status (`"pending"`, `"delivered"`, or `"failed"`).
    #[djangors(max_length = 32, db_index)]
    pub status: String,
    /// Number of delivery attempts made so far.
    pub attempts: i32,
    /// Maximum number of attempts before marking the delivery `"failed"`.
    pub max_attempts: i32,
    /// HTTP status code from the most recent attempt, if a response was received.
    pub last_status_code: Option<i32>,
    /// Response body (or error message) from the most recent attempt.
    pub last_response_body: Option<String>,
    /// Earliest UTC timestamp when the next retry may run.
    pub next_retry_at: Option<DateTime<Utc>>,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Timestamp when the delivery succeeded (UTC), if delivered.
    pub delivered_at: Option<DateTime<Utc>>,
}

/// Summary counts returned by [`WebhookDispatcher::deliver_due`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryBatchReport {
    /// Total due deliveries attempted in this batch.
    pub attempted: usize,
    /// Deliveries that succeeded with a 2xx response.
    pub delivered: usize,
    /// Deliveries that failed this attempt and were scheduled for retry.
    pub scheduled_retry: usize,
    /// Deliveries that exhausted `max_attempts` and were marked `"failed"`.
    pub failed: usize,
}

fn compute_hmac_bytes(secret: &str, timestamp_unix: i64, payload: &[u8]) -> Vec<u8> {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC-SHA256 accepts any key length");
    mac.update(timestamp_unix.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    mac.finalize().into_bytes().to_vec()
}

/// Computes an HMAC-SHA256 signature header value formatted as `t={ts},v1={hex}`.
pub fn sign_payload(secret: &str, timestamp_unix: i64, payload: &[u8]) -> String {
    let sig = hex::encode(compute_hmac_bytes(secret, timestamp_unix, payload));
    format!("t={timestamp_unix},v1={sig}")
}

/// Verifies an `X-Djangors-Signature` header against `secret`, `payload`, and `now_unix`
/// using constant-time comparison and a `tolerance_secs` replay window.
pub fn verify_signature(
    secret: &str,
    header: &str,
    payload: &[u8],
    now_unix: i64,
    tolerance_secs: i64,
) -> Result<(), WebhookError> {
    let mut timestamp: Option<i64> = None;
    let mut signature_hex: Option<&str> = None;

    for part in header.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else {
            return Err(WebhookError::InvalidSignature);
        };
        match key.trim() {
            "t" => {
                let ts = value
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| WebhookError::InvalidSignature)?;
                timestamp = Some(ts);
            }
            "v1" => {
                signature_hex = Some(value.trim());
            }
            _ => {}
        }
    }

    let ts = timestamp.ok_or(WebhookError::InvalidSignature)?;
    let sig_hex = signature_hex.ok_or(WebhookError::InvalidSignature)?;
    let provided = hex::decode(sig_hex).map_err(|_| WebhookError::InvalidSignature)?;
    let expected = compute_hmac_bytes(secret, ts, payload);

    if provided.len() != expected.len() || !bool::from(expected.ct_eq(&provided)) {
        return Err(WebhookError::InvalidSignature);
    }

    let age = now_unix.saturating_sub(ts).saturating_abs();
    if tolerance_secs < 0 || age > tolerance_secs {
        return Err(WebhookError::ExpiredTimestamp);
    }

    Ok(())
}

/// Outbound webhook emitter, delivery worker, and replay controller.
#[derive(Debug, Clone)]
pub struct WebhookDispatcher {
    db: Database,
    client: reqwest::Client,
    max_attempts: i32,
}

impl WebhookDispatcher {
    /// Creates a new [`WebhookDispatcher`] with default HTTP client and `max_attempts = 5`.
    pub fn new(db: Database) -> Self {
        Self {
            db,
            client: reqwest::Client::new(),
            max_attempts: 5,
        }
    }

    /// Configures the maximum number of delivery attempts before marking a delivery `"failed"`.
    pub fn with_max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    /// Registers and persists a new enabled [`WebhookEndpoint`].
    pub async fn register_endpoint(
        &self,
        owner_id: &str,
        url: &str,
        secret: &str,
        events: &[&str],
        description: &str,
    ) -> Result<WebhookEndpoint, WebhookError> {
        let events_json = serde_json::to_string(&events)?;
        let endpoint = WebhookEndpoint {
            id: 0,
            owner_id: owner_id.to_string(),
            url: url.to_string(),
            secret: secret.to_string(),
            events_json,
            enabled: true,
            description: description.to_string(),
            created_at: Utc::now(),
        };
        Ok(endpoint.save(&self.db).await?)
    }

    /// Enqueues [`WebhookDelivery`] rows for all enabled endpoints of `owner_id`
    /// subscribed to `event` (or `"*"`), returning the created delivery IDs.
    pub async fn emit(
        &self,
        owner_id: &str,
        event: &str,
        payload: &serde_json::Value,
    ) -> Result<Vec<i64>, WebhookError> {
        let endpoints = WebhookEndpoint::objects()
            .filter(q!(owner_id = owner_id, enabled = true))?
            .all(&self.db)
            .await?;

        let payload_json = serde_json::to_string(payload)?;
        let now = Utc::now();
        let mut delivery_ids = Vec::new();

        for endpoint in endpoints {
            let subscribed: Vec<String> = serde_json::from_str(&endpoint.events_json)?;
            if !subscribed.iter().any(|e| e == "*" || e == event) {
                continue;
            }
            let delivery = WebhookDelivery {
                id: 0,
                endpoint_id: endpoint.id,
                event: event.to_string(),
                payload_json: payload_json.clone(),
                status: STATUS_PENDING.to_string(),
                attempts: 0,
                max_attempts: self.max_attempts,
                last_status_code: None,
                last_response_body: None,
                next_retry_at: Some(now),
                created_at: now,
                delivered_at: None,
            };
            let saved = delivery.save(&self.db).await?;
            delivery_ids.push(saved.id);
        }

        Ok(delivery_ids)
    }

    /// Delivers all pending [`WebhookDelivery`] rows whose `next_retry_at` is unset or `<= now`.
    pub async fn deliver_due(&self) -> Result<DeliveryBatchReport, WebhookError> {
        let pending = WebhookDelivery::objects()
            .filter(q!(status = STATUS_PENDING))?
            .all(&self.db)
            .await?;

        let now = Utc::now();
        let mut report = DeliveryBatchReport::default();

        for mut delivery in pending {
            if let Some(next_at) = delivery.next_retry_at {
                if next_at > now {
                    continue;
                }
            }

            let endpoint = match WebhookEndpoint::objects()
                .filter(q!(id = delivery.endpoint_id))?
                .get(&self.db)
                .await
            {
                Ok(ep) => ep,
                Err(OrmError::NotFound { .. }) => {
                    report.attempted += 1;
                    delivery.attempts += 1;
                    delivery.status = STATUS_FAILED.to_string();
                    delivery.last_response_body = Some("webhook endpoint not found".to_string());
                    delivery.next_retry_at = None;
                    delivery.update(&self.db).await?;
                    report.failed += 1;
                    continue;
                }
                Err(err) => return Err(err.into()),
            };

            if !endpoint.enabled {
                continue;
            }

            report.attempted += 1;
            let attempt_now = Utc::now();
            let signature = sign_payload(
                &endpoint.secret,
                attempt_now.timestamp(),
                delivery.payload_json.as_bytes(),
            );

            let send_result = self
                .client
                .post(&endpoint.url)
                .header("Content-Type", "application/json")
                .header(SIGNATURE_HEADER, signature)
                .header(EVENT_HEADER, &delivery.event)
                .body(delivery.payload_json.clone())
                .send()
                .await;

            match send_result {
                Ok(response) => {
                    let status = response.status();
                    let code = i32::from(status.as_u16());
                    let body = response.text().await.ok();
                    delivery.last_status_code = Some(code);
                    delivery.last_response_body = body;
                    delivery.attempts += 1;

                    if status.is_success() {
                        delivery.status = STATUS_DELIVERED.to_string();
                        delivery.delivered_at = Some(attempt_now);
                        delivery.next_retry_at = None;
                        report.delivered += 1;
                    } else {
                        Self::record_failure(&mut delivery, attempt_now, &mut report);
                    }
                }
                Err(err) => {
                    delivery.attempts += 1;
                    delivery.last_status_code = None;
                    delivery.last_response_body = Some(err.to_string());
                    Self::record_failure(&mut delivery, attempt_now, &mut report);
                }
            }

            delivery.update(&self.db).await?;
        }

        Ok(report)
    }

    fn record_failure(
        delivery: &mut WebhookDelivery,
        now: DateTime<Utc>,
        report: &mut DeliveryBatchReport,
    ) {
        if delivery.attempts >= delivery.max_attempts {
            delivery.status = STATUS_FAILED.to_string();
            delivery.next_retry_at = None;
            report.failed += 1;
        } else {
            delivery.status = STATUS_PENDING.to_string();
            let shift = u32::try_from(delivery.attempts.saturating_sub(1)).unwrap_or(30);
            let multiplier = 1_i64.checked_shl(shift).unwrap_or(i64::MAX);
            let delay_secs = 30_i64.saturating_mul(multiplier);
            delivery.next_retry_at = Some(now + Duration::seconds(delay_secs));
            report.scheduled_retry += 1;
        }
    }

    /// Resets a delivery to `"pending"` with `attempts = 0` and `next_retry_at = Some(now)`
    /// for immediate 1-click replay.
    pub async fn replay(&self, delivery_id: i64) -> Result<(), WebhookError> {
        let mut delivery = WebhookDelivery::objects()
            .filter(q!(id = delivery_id))?
            .get(&self.db)
            .await
            .map_err(|err| match err {
                OrmError::NotFound { .. } => WebhookError::NotFound,
                other => WebhookError::Orm(other),
            })?;

        delivery.status = STATUS_PENDING.to_string();
        delivery.attempts = 0;
        delivery.next_retry_at = Some(Utc::now());
        delivery.delivered_at = None;
        delivery.update(&self.db).await?;
        Ok(())
    }
}

/// Creates the `djangors_webhook_endpoint` and `djangors_webhook_delivery` tables
/// and their indexes if they do not already exist.
pub async fn create_webhooks_tables(db: &Database) -> Result<(), WebhookError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let bigint = dialect.bigint_type();
    let bool_type = dialect.bool_type();
    let ts = dialect.timestamp_type();

    let stmts = [
        format!(
            "CREATE TABLE IF NOT EXISTS djangors_webhook_endpoint (
                id {pk},
                owner_id VARCHAR(128) NOT NULL,
                url VARCHAR(1024) NOT NULL,
                secret VARCHAR(255) NOT NULL,
                events_json TEXT NOT NULL,
                enabled {bool_type} NOT NULL,
                description TEXT NOT NULL,
                created_at {ts} NOT NULL
            )"
        ),
        "CREATE INDEX IF NOT EXISTS djangors_webhook_endpoint_owner_idx \
         ON djangors_webhook_endpoint (owner_id)"
            .to_string(),
        format!(
            "CREATE TABLE IF NOT EXISTS djangors_webhook_delivery (
                id {pk},
                endpoint_id {bigint} NOT NULL,
                event VARCHAR(128) NOT NULL,
                payload_json TEXT NOT NULL,
                status VARCHAR(32) NOT NULL,
                attempts INTEGER NOT NULL,
                max_attempts INTEGER NOT NULL,
                last_status_code INTEGER,
                last_response_body TEXT,
                next_retry_at {ts},
                created_at {ts} NOT NULL,
                delivered_at {ts}
            )"
        ),
        "CREATE INDEX IF NOT EXISTS djangors_webhook_delivery_endpoint_idx \
         ON djangors_webhook_delivery (endpoint_id)"
            .to_string(),
        "CREATE INDEX IF NOT EXISTS djangors_webhook_delivery_status_idx \
         ON djangors_webhook_delivery (status)"
            .to_string(),
    ];

    for sql in stmts {
        conn.execute(&sql, &[]).await.map_err(OrmError::Query)?;
    }
    Ok(())
}

/// Registers [`WebhookEndpoint`] and [`WebhookDelivery`] with an [`djangors_admin::AdminSite`].
#[cfg(feature = "admin")]
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register::<WebhookEndpoint>();
    site.register::<WebhookDelivery>();
}

#[cfg(test)]
mod tests;
