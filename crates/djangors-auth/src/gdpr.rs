//! GDPR user data export (ZIP archive) and scheduled account deletion / anonymization.

use crate::{Auth, AuthError, User};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use djangors_core::settings::DjangorsSettings;
use djangors_core::signing::{Signer, SigningError};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors_db::{BindValue, Database};
use djangors_orm::Model as _;
use std::collections::BTreeMap;
use std::future::Future;
use std::io::{Cursor, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Boxed future returned by GDPR user data exporters.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Function pointer type for exporting one category of user data as JSON.
pub type ExporterFn =
    for<'a> fn(&'a Database, i64) -> BoxFuture<'a, Result<serde_json::Value, AuthError>>;

/// Registry of named user-data exporters used to assemble a GDPR export archive.
#[derive(Clone, Default)]
pub struct UserDataRegistry {
    exporters: BTreeMap<String, ExporterFn>,
}

impl UserDataRegistry {
    /// Creates an empty [`UserDataRegistry`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a [`UserDataRegistry`] pre-populated with built-in exporters:
    /// `user`, `sessions`, `social_accounts`, `api_keys`, and `history`.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register("user", export_user_profile);
        reg.register("sessions", export_user_sessions);
        reg.register("social_accounts", export_social_accounts);
        reg.register("api_keys", export_api_keys);
        reg.register("history", export_history);
        reg
    }

    /// Registers an exporter under `name`.
    pub fn register(&mut self, name: impl Into<String>, exporter: ExporterFn) {
        self.exporters.insert(name.into(), exporter);
    }

    /// Returns the registered exporter names.
    pub fn names(&self) -> Vec<&str> {
        self.exporters.keys().map(String::as_str).collect()
    }
}

fn export_user_profile<'a>(
    db: &'a Database,
    user_id: i64,
) -> BoxFuture<'a, Result<serde_json::Value, AuthError>> {
    Box::pin(async move {
        let users = User::objects()
            .filter(djangors_orm::q!(id = user_id))?
            .all(db)
            .await?;
        let val = users
            .into_iter()
            .next()
            .map(|u| {
                serde_json::json!({
                    "id": u.id,
                    "username": u.username,
                    "email": u.email,
                    "is_active": u.is_active,
                    "is_staff": u.is_staff,
                    "is_superuser": u.is_superuser,
                    "date_joined": u.date_joined,
                    "last_login": u.last_login,
                })
            })
            .unwrap_or(serde_json::Value::Null);
        Ok(val)
    })
}

fn export_user_sessions<'a>(
    db: &'a Database,
    user_id: i64,
) -> BoxFuture<'a, Result<serde_json::Value, AuthError>> {
    Box::pin(async move {
        let mut conn = db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql = format!(
            "SELECT ip, user_agent, created_at, last_seen_at, expires_at FROM djangors_session WHERE user_id = {p1}"
        );
        let rows = match conn.fetch_all(&sql, &[BindValue::I64(user_id)]).await {
            Ok(r) => r,
            Err(_) => return Ok(serde_json::json!([])),
        };
        let list: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "ip": r.try_string(0).ok().flatten(),
                    "user_agent": r.try_string(1).ok().flatten(),
                    "created_at": r.try_datetime(2).ok().flatten(),
                    "last_seen_at": r.try_datetime(3).ok().flatten(),
                    "expires_at": r.try_datetime(4).ok().flatten(),
                })
            })
            .collect();
        Ok(serde_json::Value::Array(list))
    })
}

fn export_social_accounts<'a>(
    db: &'a Database,
    user_id: i64,
) -> BoxFuture<'a, Result<serde_json::Value, AuthError>> {
    Box::pin(async move {
        let mut conn = db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql = format!(
            "SELECT provider, subject, email, created_at FROM oauth_social_account WHERE user_id = {p1}"
        );
        let rows = match conn.fetch_all(&sql, &[BindValue::I64(user_id)]).await {
            Ok(r) => r,
            Err(_) => return Ok(serde_json::json!([])),
        };
        let list: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "provider": r.try_string(0).ok().flatten(),
                    "subject": r.try_string(1).ok().flatten(),
                    "email": r.try_string(2).ok().flatten(),
                    "created_at": r.try_datetime(3).ok().flatten(),
                })
            })
            .collect();
        Ok(serde_json::Value::Array(list))
    })
}

fn export_api_keys<'a>(
    db: &'a Database,
    user_id: i64,
) -> BoxFuture<'a, Result<serde_json::Value, AuthError>> {
    Box::pin(async move {
        let mut conn = db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql = format!(
            "SELECT name, prefix, created_at, expires_at FROM rest_api_key WHERE user_id = {p1}"
        );
        let rows = match conn.fetch_all(&sql, &[BindValue::I64(user_id)]).await {
            Ok(r) => r,
            Err(_) => return Ok(serde_json::json!([])),
        };
        let list: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.try_string(0).ok().flatten(),
                    "prefix": r.try_string(1).ok().flatten(),
                    "created_at": r.try_datetime(2).ok().flatten(),
                    "expires_at": r.try_datetime(3).ok().flatten(),
                })
            })
            .collect();
        Ok(serde_json::Value::Array(list))
    })
}

fn export_history<'a>(
    db: &'a Database,
    user_id: i64,
) -> BoxFuture<'a, Result<serde_json::Value, AuthError>> {
    Box::pin(async move {
        let mut conn = db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql = format!(
            "SELECT model_name, object_id, change_reason, created_at FROM history_record WHERE changed_by_id = {p1}"
        );
        let rows = match conn.fetch_all(&sql, &[BindValue::I64(user_id)]).await {
            Ok(r) => r,
            Err(_) => return Ok(serde_json::json!([])),
        };
        let list: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "model_name": r.try_string(0).ok().flatten(),
                    "object_id": r.try_string(1).ok().flatten(),
                    "change_reason": r.try_string(2).ok().flatten(),
                    "created_at": r.try_datetime(3).ok().flatten(),
                })
            })
            .collect();
        Ok(serde_json::Value::Array(list))
    })
}

/// Runs all exporters in `registry` for `user_id` and returns a DEFLATE-compressed ZIP archive
/// containing one `<name>.json` file per exporter.
pub async fn export_user_data(
    db: &Database,
    registry: &UserDataRegistry,
    user_id: i64,
) -> Result<Vec<u8>, AuthError> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        for (name, exporter) in &registry.exporters {
            let value = exporter(db, user_id).await?;
            let json_bytes =
                serde_json::to_vec_pretty(&value).map_err(|e| AuthError::Gdpr(e.to_string()))?;
            let filename = format!("{name}.json");
            writer
                .start_file(&filename, options)
                .map_err(|e| AuthError::Gdpr(e.to_string()))?;
            writer
                .write_all(&json_bytes)
                .map_err(|e| AuthError::Gdpr(e.to_string()))?;
        }

        writer
            .finish()
            .map_err(|e| AuthError::Gdpr(e.to_string()))?;
    }
    Ok(cursor.into_inner())
}

/// Salt that domain-separates GDPR export download tokens from every other signed value.
const EXPORT_TOKEN_SALT: &str = "djangors-auth.gdpr_export.v1";

/// Generates a short-lived signed download token for a user's GDPR export.
///
/// The token is `user_id.expires_at:signature`, signed with
/// [`djangors_core::signing::Signer`] under [`EXPORT_TOKEN_SALT`]. Fails if `secret` is empty.
pub fn sign_export_download_token(
    user_id: i64,
    secret: &[u8],
    ttl: Duration,
) -> Result<String, SigningError> {
    let signer = Signer::new(secret, EXPORT_TOKEN_SALT)?;
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
        .saturating_add(ttl.as_secs());
    Ok(signer.sign(&format!("{user_id}.{expires_at}")))
}

/// Verifies a GDPR export download token and returns the `user_id` if valid and unexpired.
/// Returns `None` if `secret` is empty.
pub fn verify_export_download_token(token: &str, secret: &[u8]) -> Option<i64> {
    let signer = Signer::new(secret, EXPORT_TOKEN_SALT).ok()?;
    let value = signer.unsign(token).ok()?;
    let (user_id, expires_at) = value.split_once('.')?;
    let user_id: i64 = user_id.parse().ok()?;
    let expires_at: u64 = expires_at.parse().ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    if expires_at <= now {
        return None;
    }
    Some(user_id)
}

/// Signing key for export download tokens. Fails closed: an absent or empty
/// `SECRET_KEY` would make the tokens forgeable by anyone.
fn export_secret(req: &Request) -> Result<Vec<u8>, DjangorsError> {
    let key = req
        .state::<DjangorsSettings>()
        .map(|s| s.secret_key.as_bytes().to_vec())
        .unwrap_or_default();
    if key.is_empty() {
        return Err(DjangorsError::Internal(
            "SECRET_KEY must be configured for data exports".to_string(),
        ));
    }
    Ok(key)
}

/// HTTP view (`POST /privacy/export`) that produces a signed download URL for the
/// authenticated user's data export.
pub async fn request_export_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let Auth(user) =
        <Auth<User> as djangors_core::extract::FromRequest>::from_request(&req).await?;
    let secret = export_secret(&req)?;
    let token = sign_export_download_token(user.id, &secret, Duration::from_secs(3600))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Response::json(
        StatusCode::OK,
        &serde_json::json!({
            "download_url": format!("/privacy/export/download?token={token}")
        }),
    )
}

/// HTTP view (`GET /privacy/export/download?token=...`) that streams the ZIP archive.
pub async fn download_export_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let token = req
        .query("token")
        .ok_or_else(|| DjangorsError::Forbidden("missing export token".to_string()))?;
    let secret = export_secret(&req)?;
    let user_id = verify_export_download_token(token, &secret)
        .ok_or_else(|| DjangorsError::Forbidden("invalid or expired export token".to_string()))?;
    let db = req
        .state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
    let registry = req
        .state::<UserDataRegistry>()
        .cloned()
        .unwrap_or_else(UserDataRegistry::with_builtins);

    let zip_bytes = export_user_data(db, &registry, user_id)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    Ok(
        Response::bytes(StatusCode::OK, "application/zip", zip_bytes)
            .header(
                "content-disposition",
                "attachment; filename=\"user-data.zip\"",
            )
            .header("cache-control", "no-store"),
    )
}

/// Hook invoked before a user is deleted or anonymized during scheduled GDPR deletion.
#[async_trait::async_trait]
pub trait AnonymizeHook: Send + Sync + 'static {
    /// Anonymizes or cleans up application data owned by `user_id`.
    async fn anonymize(&self, db: &Database, user_id: i64) -> Result<(), AuthError>;
}

async fn ensure_deletion_table(db: &Database) -> Result<(), AuthError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let bigint = dialect.bigint_type();
    let ts = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS auth_account_deletion (
            user_id {bigint} PRIMARY KEY,
            deletion_scheduled_at {ts} NOT NULL,
            cancelled_at {ts},
            completed_at {ts}
        )"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(|e| AuthError::Gdpr(e.to_string()))?;
    Ok(())
}

/// Schedules account deletion for `user_id` after `grace_days` (defaults to 30 days).
/// Returns the scheduled deletion timestamp.
pub async fn request_account_deletion(
    db: &Database,
    user_id: i64,
    grace_days: Option<i64>,
) -> Result<DateTime<Utc>, AuthError> {
    ensure_deletion_table(db).await?;
    let days = grace_days.unwrap_or(30).max(0);
    let scheduled_at = Utc::now() + ChronoDuration::days(days);
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let sql = format!(
        "INSERT INTO auth_account_deletion (user_id, deletion_scheduled_at, cancelled_at, completed_at)
         VALUES ({p1}, {p2}, NULL, NULL)
         ON CONFLICT (user_id) DO UPDATE SET
            deletion_scheduled_at = EXCLUDED.deletion_scheduled_at,
            cancelled_at = NULL,
            completed_at = NULL"
    );
    conn.execute(
        &sql,
        &[BindValue::I64(user_id), BindValue::DateTime(scheduled_at)],
    )
    .await
    .map_err(|e| AuthError::Gdpr(e.to_string()))?;
    Ok(scheduled_at)
}

/// Cancels a pending account deletion request for `user_id`.
pub async fn cancel_account_deletion(db: &Database, user_id: i64) -> Result<(), AuthError> {
    ensure_deletion_table(db).await?;
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let sql = format!(
        "UPDATE auth_account_deletion SET cancelled_at = {p1}
         WHERE user_id = {p2} AND completed_at IS NULL"
    );
    conn.execute(
        &sql,
        &[BindValue::DateTime(Utc::now()), BindValue::I64(user_id)],
    )
    .await
    .map_err(|e| AuthError::Gdpr(e.to_string()))?;
    Ok(())
}

/// Returns the scheduled deletion timestamp for `user_id`, if an active request exists.
pub async fn get_scheduled_deletion(
    db: &Database,
    user_id: i64,
) -> Result<Option<DateTime<Utc>>, AuthError> {
    ensure_deletion_table(db).await?;
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let sql = format!(
        "SELECT deletion_scheduled_at FROM auth_account_deletion
         WHERE user_id = {p1} AND cancelled_at IS NULL AND completed_at IS NULL"
    );
    let row = conn
        .fetch_optional(&sql, &[BindValue::I64(user_id)])
        .await
        .map_err(|e| AuthError::Gdpr(e.to_string()))?;
    Ok(row.and_then(|r| r.try_datetime(0).ok().flatten()))
}

/// Processes all due account deletions (`deletion_scheduled_at <= now`, not cancelled/completed):
/// runs all registered [`AnonymizeHook`]s, anonymizes and deactivates the [`User`] row,
/// and marks `completed_at`. Returns the number of accounts processed.
pub async fn run_scheduled_deletions(
    db: &Database,
    hooks: &[Arc<dyn AnonymizeHook>],
) -> Result<usize, AuthError> {
    ensure_deletion_table(db).await?;
    let now = Utc::now();
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let sql = format!(
        "SELECT user_id FROM auth_account_deletion
         WHERE deletion_scheduled_at <= {p1} AND cancelled_at IS NULL AND completed_at IS NULL"
    );
    let rows = conn
        .fetch_all(&sql, &[BindValue::DateTime(now)])
        .await
        .map_err(|e| AuthError::Gdpr(e.to_string()))?;

    let mut processed = 0usize;
    for row in rows {
        let Some(user_id) = row.try_i64(0).ok().flatten() else {
            continue;
        };
        for hook in hooks {
            hook.anonymize(db, user_id).await?;
        }

        if let Some(mut user) = User::objects()
            .filter(djangors_orm::q!(id = user_id))?
            .all(db)
            .await?
            .into_iter()
            .next()
        {
            user.username = format!("deleted_user_{user_id}");
            user.email = format!("deleted_{user_id}@anonymized.invalid");
            user.password = String::new();
            user.is_active = false;
            user.is_staff = false;
            user.is_superuser = false;
            user.update(db).await?;
        }

        let p1 = conn.dialect().placeholder(1);
        let p2 = conn.dialect().placeholder(2);
        let done_sql =
            format!("UPDATE auth_account_deletion SET completed_at = {p1} WHERE user_id = {p2}");
        conn.execute(
            &done_sql,
            &[BindValue::DateTime(Utc::now()), BindValue::I64(user_id)],
        )
        .await
        .map_err(|e| AuthError::Gdpr(e.to_string()))?;
        processed += 1;
    }

    Ok(processed)
}

/// Convenience status constant used in GDPR module tests.
pub const EXPORT_STATUS_OK: StatusCode = StatusCode::OK;
