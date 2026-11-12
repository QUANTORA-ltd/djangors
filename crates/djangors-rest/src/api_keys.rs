//! Machine-to-machine API key authentication, scopes, and admin registration.

use djangors_core::error::DjangorsError;
use djangors_core::extract::FromRequest;
use djangors_core::request::Request;
use djangors_orm::meta::Model;
use rand::Rng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::permissions::Permission;

const INVALID_API_KEY_MSG: &str = "invalid API key";

/// A database-backed API key for machine-to-machine authentication.
///
/// Keys have the form `<prefix>.<secret>`, where `prefix` is an 8-character
/// alphanumeric identifier stored in plaintext for lookup and `secret` is a
/// 32-byte random hex string whose SHA-256 digest is stored in `hashed_key`.
/// The full key is only returned once at creation time.
#[derive(djangors_macros::Model, Debug, Clone)]
#[djangors(app = "djangors_rest", table_name = "djangors_rest_apikey")]
pub struct ApiKey {
    /// Primary key for the API key record.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Human-readable label for the key (e.g. `"billing-worker"`).
    #[djangors(max_length = 100)]
    pub name: String,
    /// Unique 8-character alphanumeric prefix used for constant-lookup indexing.
    #[djangors(max_length = 16, unique)]
    pub prefix: String,
    /// Lowercase hex-encoded SHA-256 digest of the secret portion of the key.
    #[djangors(max_length = 64)]
    pub hashed_key: String,
    /// Space-separated list of granted scopes (may be empty).
    #[djangors(max_length = 500)]
    pub scopes: String,
    /// Timestamp when this key was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Optional expiration timestamp after which the key is rejected.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this key has been revoked.
    pub revoked: bool,
    /// Timestamp when this key was last used to authenticate a request.
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl ApiKey {
    /// Returns `true` if this key's space-separated `scopes` include `scope`.
    pub fn has_scope(&self, scope: &str) -> bool {
        !scope.is_empty() && self.scopes.split_whitespace().any(|s| s == scope)
    }
}

/// Creates the `djangors_rest_apikey` table if it does not already exist.
pub async fn create_api_key_table(db: &djangors_db::Database) -> Result<(), djangors_db::DbError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk_type = dialect.auto_pk_type();
    let ts_type = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_rest_apikey (
            id {pk_type},
            name VARCHAR(100) NOT NULL,
            prefix VARCHAR(16) NOT NULL UNIQUE,
            hashed_key VARCHAR(64) NOT NULL,
            scopes VARCHAR(500) NOT NULL DEFAULT '',
            created_at {ts_type} NOT NULL,
            expires_at {ts_type},
            revoked BOOLEAN NOT NULL DEFAULT FALSE,
            last_used_at {ts_type}
        );"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

fn generate_prefix() -> String {
    let mut rng = rand::rngs::OsRng;
    (0..8)
        .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
        .collect()
}

fn generate_secret() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hash_secret(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Creates and persists a new [`ApiKey`], returning the saved row and the
/// plaintext `<prefix>.<secret>` key string.
///
/// The plaintext secret is never stored and cannot be recovered after this call.
pub async fn create_api_key(
    db: &djangors_db::Database,
    name: &str,
    scopes: &[&str],
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(ApiKey, String), djangors_orm::OrmError> {
    let prefix = generate_prefix();
    let secret = generate_secret();
    let full_key = format!("{prefix}.{secret}");
    let hashed_key = hash_secret(&secret);

    let key = ApiKey {
        id: 0,
        name: name.to_string(),
        prefix,
        hashed_key,
        scopes: scopes.join(" "),
        created_at: chrono::Utc::now(),
        expires_at,
        revoked: false,
        last_used_at: None,
    }
    .save(db)
    .await?;

    Ok((key, full_key))
}

/// Revokes the API key matching `prefix`.
///
/// Returns `Ok(true)` if a matching row was updated, or `Ok(false)` if no key
/// with that prefix exists.
pub async fn revoke_api_key(
    db: &djangors_db::Database,
    prefix: &str,
) -> Result<bool, djangors_orm::OrmError> {
    let updated = ApiKey::objects()
        .filter(djangors_orm::q!(prefix = prefix))?
        .update(
            db,
            vec![(
                "revoked",
                djangors_orm::SetExpr::Literal(djangors_orm::Value::Bool(true)),
            )],
        )
        .await?;
    Ok(updated > 0)
}

fn extract_raw_api_key(req: &Request) -> Option<&str> {
    if let Some(value) = req.header("x-api-key").and_then(|v| v.to_str().ok()) {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !trimmed.chars().any(char::is_whitespace) {
            return Some(trimmed);
        }
        return None;
    }

    req.header("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Api-Key "))
        .filter(|key| !key.is_empty() && !key.chars().any(char::is_whitespace))
}

/// Extractor that authenticates a request via `X-Api-Key` or
/// `Authorization: Api-Key <key>`.
#[derive(Debug, Clone)]
pub struct ApiKeyAuth(pub ApiKey);

#[async_trait::async_trait]
impl FromRequest for ApiKeyAuth {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let raw_key = extract_raw_api_key(req)
            .ok_or_else(|| DjangorsError::Unauthorized(INVALID_API_KEY_MSG.to_string()))?;

        let (prefix, secret) = raw_key
            .split_once('.')
            .filter(|(p, s)| !p.is_empty() && !s.is_empty())
            .ok_or_else(|| DjangorsError::Unauthorized(INVALID_API_KEY_MSG.to_string()))?;

        let db = req.require_state::<djangors_db::Database>()?;
        let mut api_key = ApiKey::objects()
            .filter(djangors_orm::q!(prefix = prefix))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    DjangorsError::Unauthorized(INVALID_API_KEY_MSG.to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;

        let candidate_hash = hash_secret(secret);
        let hash_matches = bool::from(
            candidate_hash
                .as_bytes()
                .ct_eq(api_key.hashed_key.as_bytes()),
        );
        let now = chrono::Utc::now();
        let is_expired = api_key.expires_at.is_some_and(|exp| exp <= now);

        if !hash_matches || api_key.revoked || is_expired {
            return Err(DjangorsError::Unauthorized(INVALID_API_KEY_MSG.to_string()));
        }

        let update_res = match ApiKey::objects().filter(djangors_orm::q!(id = api_key.id)) {
            Ok(qs) => {
                qs.update(
                    db,
                    vec![(
                        "last_used_at",
                        djangors_orm::SetExpr::Literal(djangors_orm::Value::DateTime(now)),
                    )],
                )
                .await
            }
            Err(err) => Err(err),
        };
        if let Err(err) = update_res {
            tracing::warn!(prefix = %api_key.prefix, "failed to update api key last_used_at: {err}");
        }
        api_key.last_used_at = Some(now);

        Ok(ApiKeyAuth(api_key))
    }
}

/// Permission policy requiring any valid, non-revoked, non-expired [`ApiKey`].
pub struct HasApiKey;

#[async_trait::async_trait]
impl Permission for HasApiKey {
    async fn has_permission(&self, req: &Request) -> bool {
        ApiKeyAuth::from_request(req).await.is_ok()
    }
}

/// Permission policy requiring a valid [`ApiKey`] that grants the specified scope.
pub struct HasApiKeyScope(pub &'static str);

#[async_trait::async_trait]
impl Permission for HasApiKeyScope {
    async fn has_permission(&self, req: &Request) -> bool {
        ApiKeyAuth::from_request(req)
            .await
            .is_ok_and(|ApiKeyAuth(key)| key.has_scope(self.0))
    }
}

/// Registers [`ApiKey`] with the given [`djangors_admin::AdminSite`].
#[cfg(feature = "admin")]
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register_with::<ApiKey>(djangors_admin::ModelAdminConfig {
        list_display: Some(&[
            "name",
            "prefix",
            "scopes",
            "revoked",
            "expires_at",
            "last_used_at",
        ]),
        search_fields: Some(&["name", "prefix"]),
        ..Default::default()
    });
}
