//! API authentication: database-backed tokens and JWT.

use djangors_auth::AuthUser;
use djangors_core::error::DjangorsError;
use djangors_core::extract::FromRequest;
use djangors_core::request::Request;
use djangors_orm::meta::Model;
use rand::RngCore;

/// A database-backed token for authenticating API requests.
#[derive(djangors_macros::Model, Debug, Clone)]
#[djangors(app = "djangors_rest", table_name = "djangors_rest_authtoken")]
pub struct AuthToken {
    /// Primary key for the token record.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Foreign key reference to the associated user.
    pub user: djangors_orm::ForeignKey<djangors_auth::User>,
    /// Unique 64-character hexadecimal token string.
    #[djangors(max_length = 64, unique)]
    pub key: String,
    /// Timestamp when this token was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Generates a 256-bit, hexadecimal API token key.
pub fn generate_token_key() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The user authenticated by the `Authorization: Token <key>` scheme.
#[derive(Debug, Clone)]
pub struct TokenAuth(pub djangors_auth::User);

#[async_trait::async_trait]
impl FromRequest for TokenAuth {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let key = req
            .header("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Token "))
            .filter(|key| !key.is_empty() && !key.chars().any(char::is_whitespace))
            .ok_or_else(|| DjangorsError::Unauthorized("not authenticated".to_string()))?;

        let db = req.require_state::<djangors_db::Database>()?;
        let token = AuthToken::objects()
            .filter(djangors_orm::q!(key = key))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    DjangorsError::Unauthorized("invalid token".to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;

        let user = djangors_auth::User::objects()
            .filter(djangors_orm::q!(id = token.user.id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    DjangorsError::Unauthorized("user not found".to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;

        if !user.is_active() {
            return Err(DjangorsError::Unauthorized("account inactive".to_string()));
        }
        Ok(TokenAuth(user))
    }
}

#[cfg(feature = "jwt")]
/// Configuration for HS256 JWT access tokens.
///
/// Attach it to the router with `Router::with_state` to override the defaults;
/// [`JwtAuth`] reads it from request state on every request.
///
/// ```no_run
/// use std::time::Duration;
/// use djangors_rest::JwtConfig;
///
/// let config = JwtConfig::new(Duration::from_secs(15 * 60));
/// ```
#[cfg(feature = "jwt")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JwtConfig {
    /// How long an access token stays valid. Defaults to one hour.
    pub access_ttl: std::time::Duration,
    /// Clock skew tolerated when validating the `exp` claim. Defaults to 0.
    pub leeway: std::time::Duration,
}

#[cfg(feature = "jwt")]
impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            access_ttl: std::time::Duration::from_secs(60 * 60),
            leeway: std::time::Duration::ZERO,
        }
    }
}

#[cfg(feature = "jwt")]
impl JwtConfig {
    /// Creates a config with the given access-token lifetime and no leeway.
    pub fn new(access_ttl: std::time::Duration) -> Self {
        Self {
            access_ttl,
            ..Self::default()
        }
    }

    /// Sets the clock-skew leeway applied when validating `exp`.
    pub fn with_leeway(mut self, leeway: std::time::Duration) -> Self {
        self.leeway = leeway;
        self
    }
}

#[cfg(feature = "jwt")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct JwtClaims {
    user_id: i64,
    exp: u64,
}

#[cfg(feature = "jwt")]
/// Encodes a user id as an HS256 JWT signed with `secret`, using the default
/// one-hour lifetime.
pub fn encode_jwt(user_id: i64, secret: &str) -> String {
    encode_jwt_with_config(user_id, secret, &JwtConfig::default())
}

#[cfg(feature = "jwt")]
/// Encodes a user id as an HS256 JWT signed with `secret` and the given
/// [`JwtConfig`].
pub fn encode_jwt_with_config(user_id: i64, secret: &str, config: &JwtConfig) -> String {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &JwtClaims {
            user_id,
            exp: jsonwebtoken::get_current_timestamp() + config.access_ttl.as_secs(),
        },
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("JWT claims are serializable")
}

#[cfg(feature = "jwt")]
/// Decodes and validates an HS256 JWT, returning its user id claim.
pub fn decode_jwt(token: &str, secret: &str) -> Result<i64, jsonwebtoken::errors::Error> {
    decode_jwt_with_config(token, secret, &JwtConfig::default())
}

#[cfg(feature = "jwt")]
/// Decodes and validates an HS256 JWT with the given [`JwtConfig`], returning
/// its user id claim.
pub fn decode_jwt_with_config(
    token: &str,
    secret: &str,
    config: &JwtConfig,
) -> Result<i64, jsonwebtoken::errors::Error> {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.leeway = config.leeway.as_secs();
    let data = jsonwebtoken::decode::<JwtClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(data.claims.user_id)
}

#[cfg(feature = "jwt")]
/// The user authenticated by the `Authorization: Bearer <jwt>` scheme.
#[derive(Debug, Clone)]
pub struct JwtAuth(pub djangors_auth::User);

#[cfg(feature = "jwt")]
#[async_trait::async_trait]
impl FromRequest for JwtAuth {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let token = req
            .header("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .filter(|token| !token.is_empty() && !token.chars().any(char::is_whitespace))
            .ok_or_else(|| DjangorsError::Unauthorized("not authenticated".to_string()))?;
        let config = req.state::<JwtConfig>().copied().unwrap_or_default();
        let user_id = decode_jwt_with_config(token, settings_secret(req)?, &config)
            .map_err(|_| DjangorsError::Unauthorized("invalid token".to_string()))?;
        let db = req.require_state::<djangors_db::Database>()?;
        let user = djangors_auth::User::objects()
            .filter(djangors_orm::q!(id = user_id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    DjangorsError::Unauthorized("user not found".to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;
        if !user.is_active() {
            return Err(DjangorsError::Unauthorized("account inactive".to_string()));
        }
        Ok(JwtAuth(user))
    }
}

#[cfg(feature = "jwt")]
fn settings_secret(req: &Request) -> Result<&str, DjangorsError> {
    req.state::<djangors_core::DjangorsSettings>()
        .map(|settings| settings.secret_key.as_str())
        .ok_or_else(|| DjangorsError::Internal("settings state absent".to_string()))
}
