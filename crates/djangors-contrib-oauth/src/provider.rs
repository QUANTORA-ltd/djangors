//! OAuth 2.0 and OpenID Connect authorization server (provider) implementation.
//!
//! Provides RFC 6749 (`authorization_code` + PKCE S256, `client_credentials`,
//! `refresh_token` with rotation & reuse detection), RFC 7009 token revocation,
//! RFC 7662 token introspection, OIDC Discovery, JWKS, UserInfo, database-backed
//! [`SigningKey`] pairs (RS256/ES256, private PEM protected with
//! `#[djangors(encrypted)]`), and [`OAuthBearer`] / [`HasOAuthScope`] request guards.

use crate::error::OAuthError;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use djangors_auth::email_verification::has_verified_email;
use djangors_auth::{Auth, User};
use djangors_core::extract::FromRequest;
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_db::{BindValue, Database};
use djangors_macros::Model;
use djangors_orm::{ForeignKey, Model as _};
use hyper::http::HeaderValue;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::pkcs8::{EncodePrivateKey as _, LineEnding as EcLineEnding};
use rand::rngs::OsRng;
use rand::RngCore;
use rsa::pkcs8::LineEnding as RsaLineEnding;
use rsa::traits::PublicKeyParts as _;
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use subtle::ConstantTimeEq;
use url::Url;

const DEFAULT_ACCESS_TOKEN_TTL_SECS: u64 = 3600;
const DEFAULT_REFRESH_TOKEN_TTL_SECS: u64 = 30 * 24 * 3600;
const DEFAULT_AUTH_CODE_TTL_SECS: u64 = 120;

/// Configuration for the Djangors OAuth 2.0 / OIDC provider endpoints.
#[derive(Debug, Clone)]
pub struct OAuthProviderConfig {
    /// Issuer identifier URL (e.g. `"https://auth.example.com"`), with no trailing slash.
    pub issuer: String,
    /// Lifetime of issued access tokens (default: 3600 seconds / 1 hour).
    pub access_token_ttl: Duration,
    /// Lifetime of issued refresh tokens (default: 30 days).
    pub refresh_token_ttl: Duration,
    /// Lifetime of issued authorization codes (default: 120 seconds).
    pub auth_code_ttl: Duration,
    /// Path or URL to redirect unauthenticated users to during `/o/authorize`.
    pub login_url: String,
    /// Scopes advertised in discovery metadata and accepted by default.
    pub supported_scopes: Vec<String>,
    /// Require PKCE S256 for confidential clients too (public clients always require PKCE S256).
    pub require_pkce_for_all_clients: bool,
}

impl OAuthProviderConfig {
    /// Creates provider configuration for the given `issuer` URL.
    pub fn new(issuer: impl Into<String>) -> Self {
        let issuer = issuer.into().trim_end_matches('/').to_string();
        Self {
            issuer,
            access_token_ttl: Duration::from_secs(DEFAULT_ACCESS_TOKEN_TTL_SECS),
            refresh_token_ttl: Duration::from_secs(DEFAULT_REFRESH_TOKEN_TTL_SECS),
            auth_code_ttl: Duration::from_secs(DEFAULT_AUTH_CODE_TTL_SECS),
            login_url: "/accounts/login".to_string(),
            supported_scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
                "read".to_string(),
                "write".to_string(),
            ],
            require_pkce_for_all_clients: false,
        }
    }

    /// Sets the access token lifetime.
    pub fn access_token_ttl(mut self, ttl: Duration) -> Self {
        self.access_token_ttl = ttl;
        self
    }

    /// Sets the refresh token lifetime.
    pub fn refresh_token_ttl(mut self, ttl: Duration) -> Self {
        self.refresh_token_ttl = ttl;
        self
    }

    /// Sets the authorization code lifetime.
    pub fn auth_code_ttl(mut self, ttl: Duration) -> Self {
        self.auth_code_ttl = ttl;
        self
    }

    /// Sets the login URL used when `/o/authorize` requires session authentication.
    pub fn login_url(mut self, url: impl Into<String>) -> Self {
        self.login_url = url.into();
        self
    }

    /// Sets the list of supported scopes advertised by discovery metadata.
    pub fn supported_scopes(mut self, scopes: Vec<String>) -> Self {
        self.supported_scopes = scopes;
        self
    }

    /// Forces PKCE `S256` even for confidential clients.
    pub fn require_pkce_for_all_clients(mut self, require: bool) -> Self {
        self.require_pkce_for_all_clients = require;
        self
    }
}

/// Registered OAuth 2.0 / OIDC client application.
#[derive(Model, Debug, Clone)]
#[djangors(app = "oauth", table_name = "oauth_application")]
pub struct OAuthApplication {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Display name shown on the authorization consent page.
    #[djangors(max_length = 150)]
    pub name: String,
    /// Public client identifier (`client_id`).
    #[djangors(max_length = 128, unique)]
    pub client_id: String,
    /// Lowercase SHA-256 hex digest of `client_secret` (`""` for public clients).
    #[djangors(max_length = 128)]
    pub client_secret_hash: String,
    /// Whitespace-separated list of exact allowed redirect URIs.
    #[djangors(max_length = 2000)]
    pub redirect_uris: String,
    /// Space-separated allowed grant types (e.g. `"authorization_code refresh_token"`).
    #[djangors(max_length = 255)]
    pub grant_types: String,
    /// Space-separated scopes this application is allowed to request.
    #[djangors(max_length = 500)]
    pub allowed_scopes: String,
    /// `true` for confidential clients that hold a secret; `false` for public clients.
    pub confidential: bool,
    /// Skip the interactive consent page for trusted first-party applications.
    pub skip_consent: bool,
    /// Optional owning user ID.
    pub owner_id: Option<i64>,
    /// Whether the application is enabled.
    pub is_active: bool,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
}

impl OAuthApplication {
    /// Creates and saves a new confidential client application, returning the saved
    /// record and the raw `client_secret` (shown only once).
    pub async fn create_confidential(
        db: &Database,
        name: impl Into<String>,
        redirect_uris: &[&str],
        grant_types: &[&str],
        allowed_scopes: &[&str],
        skip_consent: bool,
        owner_id: Option<i64>,
    ) -> Result<(Self, String), OAuthError> {
        let client_id = format!("cli_{}", random_urlsafe_token(18));
        let client_secret = format!("sec_{}", random_urlsafe_token(32));
        let app = Self {
            id: 0,
            name: name.into(),
            client_id,
            client_secret_hash: sha256_hex(&client_secret),
            redirect_uris: redirect_uris.join(" "),
            grant_types: grant_types.join(" "),
            allowed_scopes: allowed_scopes.join(" "),
            confidential: true,
            skip_consent,
            owner_id,
            is_active: true,
            created_at: Utc::now(),
        };
        let saved = app.save(db).await?;
        Ok((saved, client_secret))
    }

    /// Creates and saves a new public client application (PKCE S256 required, no secret).
    pub async fn create_public(
        db: &Database,
        name: impl Into<String>,
        redirect_uris: &[&str],
        grant_types: &[&str],
        allowed_scopes: &[&str],
        skip_consent: bool,
        owner_id: Option<i64>,
    ) -> Result<Self, OAuthError> {
        let app = Self {
            id: 0,
            name: name.into(),
            client_id: format!("pub_{}", random_urlsafe_token(18)),
            client_secret_hash: String::new(),
            redirect_uris: redirect_uris.join(" "),
            grant_types: grant_types.join(" "),
            allowed_scopes: allowed_scopes.join(" "),
            confidential: false,
            skip_consent,
            owner_id,
            is_active: true,
            created_at: Utc::now(),
        };
        Ok(app.save(db).await?)
    }

    /// Checks whether `uri` matches one of [`Self::redirect_uris`] by exact equality.
    pub fn allows_redirect_uri(&self, uri: &str) -> bool {
        !uri.is_empty() && self.redirect_uris.split_whitespace().any(|u| u == uri)
    }

    /// Checks whether `grant` is permitted for this application.
    pub fn allows_grant_type(&self, grant: &str) -> bool {
        !grant.is_empty() && self.grant_types.split_whitespace().any(|g| g == grant)
    }

    /// Verifies `candidate_secret` in constant time against [`Self::client_secret_hash`].
    pub fn verify_secret(&self, candidate_secret: &str) -> bool {
        if !self.confidential || candidate_secret.is_empty() || self.client_secret_hash.is_empty() {
            return false;
        }
        let digest = sha256_hex(candidate_secret);
        digest
            .as_bytes()
            .ct_eq(self.client_secret_hash.as_bytes())
            .into()
    }
}

/// Short-lived, single-use OAuth 2.0 authorization code (stored hashed).
#[derive(Model, Debug, Clone)]
#[djangors(app = "oauth", table_name = "oauth_authorization_code")]
pub struct AuthorizationCode {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Client application the code was issued to.
    pub application: ForeignKey<OAuthApplication>,
    /// Authorizing user.
    pub user: ForeignKey<User>,
    /// SHA-256 hex digest of the raw authorization code.
    #[djangors(max_length = 128, unique)]
    pub code_hash: String,
    /// Redirect URI bound to this code (must match `/o/token` request).
    #[djangors(max_length = 1000)]
    pub redirect_uri: String,
    /// Space-separated granted scopes.
    #[djangors(max_length = 500)]
    pub scopes: String,
    /// PKCE code challenge (base64url S256 digest), or `""` when unused.
    #[djangors(max_length = 128)]
    pub code_challenge: String,
    /// PKCE challenge method (`"S256"` or `""`).
    #[djangors(max_length = 16)]
    pub code_challenge_method: String,
    /// OIDC `nonce` bound to this authorization request (`""` if absent).
    #[djangors(max_length = 255)]
    pub nonce: String,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Expiration timestamp (UTC).
    pub expires_at: DateTime<Utc>,
    /// Timestamp when the code was exchanged, if already used.
    pub used_at: Option<DateTime<Utc>>,
}

/// OAuth 2.0 Bearer access token (stored hashed).
#[derive(Model, Debug, Clone)]
#[djangors(app = "oauth", table_name = "oauth_access_token")]
pub struct AccessToken {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Client application the token belongs to.
    pub application: ForeignKey<OAuthApplication>,
    /// Subject user ID, or `None` for `client_credentials` tokens.
    pub user_id: Option<i64>,
    /// SHA-256 hex digest of the raw Bearer token.
    #[djangors(max_length = 128, unique)]
    pub token_hash: String,
    /// Space-separated granted scopes.
    #[djangors(max_length = 500)]
    pub scopes: String,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Expiration timestamp (UTC).
    pub expires_at: DateTime<Utc>,
    /// Revocation timestamp (UTC), if revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl AccessToken {
    /// Returns `true` if `scope` is included in this token's granted scopes.
    pub fn has_scope(&self, scope: &str) -> bool {
        !scope.is_empty() && self.scopes.split_whitespace().any(|s| s == scope)
    }

    /// Returns `true` if every scope in `required` is present on this token.
    pub fn has_all_scopes(&self, required: &[&str]) -> bool {
        required.iter().all(|s| self.has_scope(s))
    }
}

/// Rotating OAuth 2.0 refresh token with token-family reuse detection (stored hashed).
#[derive(Model, Debug, Clone)]
#[djangors(app = "oauth", table_name = "oauth_refresh_token")]
pub struct RefreshToken {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Client application the token belongs to.
    pub application: ForeignKey<OAuthApplication>,
    /// User the token represents.
    pub user: ForeignKey<User>,
    /// SHA-256 hex digest of the raw refresh token.
    #[djangors(max_length = 128, unique)]
    pub token_hash: String,
    /// Token family identifier shared across rotations to detect replay attacks.
    #[djangors(max_length = 64)]
    pub family_id: String,
    /// Space-separated granted scopes.
    #[djangors(max_length = 500)]
    pub scopes: String,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Expiration timestamp (UTC).
    pub expires_at: DateTime<Utc>,
    /// Timestamp when this refresh token was rotated, if already exchanged.
    pub used_at: Option<DateTime<Utc>>,
    /// Timestamp when this token (or its entire family) was revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Asymmetric OIDC ID-token signing key pair (`RS256` or `ES256`).
///
/// The PKCS#8 private key PEM is encrypted at rest via `#[djangors(encrypted)]`.
#[derive(Model, Debug, Clone)]
#[djangors(app = "oauth", table_name = "oauth_signing_key")]
pub struct SigningKey {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Key ID advertised in JWT headers and the JWKS endpoint.
    #[djangors(max_length = 64, unique)]
    pub kid: String,
    /// JWS algorithm (`"RS256"` or `"ES256"`).
    #[djangors(max_length = 16)]
    pub algorithm: String,
    /// PKCS#8 private key PEM (encrypted at rest).
    #[djangors(encrypted)]
    pub private_key_pem: String,
    /// Public JWK JSON exposed by `/o/jwks`.
    pub public_jwk_json: String,
    /// Whether this key is active for verification/signing.
    pub is_active: bool,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
}

impl SigningKey {
    /// Generates a new 2048-bit RSA (`RS256`) signing key (not yet saved to the database).
    pub fn generate_rs256() -> Result<Self, OAuthError> {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048)
            .map_err(|e| OAuthError::InvalidConfig(format!("failed generating RSA key: {e}")))?;
        let pem = private_key
            .to_pkcs8_pem(RsaLineEnding::LF)
            .map_err(|e| OAuthError::InvalidConfig(format!("failed encoding RSA PEM: {e}")))?
            .to_string();
        let pub_key = private_key.to_public_key();
        let n_b64 = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
        let kid = format!("rsa-{}", &sha256_hex(&format!("{n_b64}.{e_b64}"))[..16]);
        let jwk = serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": kid,
            "n": n_b64,
            "e": e_b64,
        });
        Ok(Self {
            id: 0,
            kid,
            algorithm: "RS256".to_string(),
            private_key_pem: pem,
            public_jwk_json: jwk.to_string(),
            is_active: true,
            created_at: Utc::now(),
        })
    }

    /// Generates a new P-256 ECDSA (`ES256`) signing key (not yet saved to the database).
    pub fn generate_es256() -> Result<Self, OAuthError> {
        let mut rng = OsRng;
        let signing_key = p256::ecdsa::SigningKey::random(&mut rng);
        let pem = signing_key
            .to_pkcs8_pem(EcLineEnding::LF)
            .map_err(|e| OAuthError::InvalidConfig(format!("failed encoding EC PEM: {e}")))?
            .to_string();
        let verifying_key = p256::ecdsa::VerifyingKey::from(&signing_key);
        let point = verifying_key.to_encoded_point(false);
        let x_bytes = point.x().ok_or_else(|| {
            OAuthError::InvalidConfig("missing EC P-256 X coordinate".to_string())
        })?;
        let y_bytes = point.y().ok_or_else(|| {
            OAuthError::InvalidConfig("missing EC P-256 Y coordinate".to_string())
        })?;
        let x_b64 = URL_SAFE_NO_PAD.encode(&x_bytes[..]);
        let y_b64 = URL_SAFE_NO_PAD.encode(&y_bytes[..]);
        let kid = format!("ec-{}", &sha256_hex(&format!("{x_b64}.{y_b64}"))[..16]);
        let jwk = serde_json::json!({
            "kty": "EC",
            "use": "sig",
            "alg": "ES256",
            "kid": kid,
            "crv": "P-256",
            "x": x_b64,
            "y": y_b64,
        });
        Ok(Self {
            id: 0,
            kid,
            algorithm: "ES256".to_string(),
            private_key_pem: pem,
            public_jwk_json: jwk.to_string(),
            is_active: true,
            created_at: Utc::now(),
        })
    }

    fn encoding_key(&self) -> Result<(Header, EncodingKey), OAuthError> {
        let (alg, key) = match self.algorithm.as_str() {
            "RS256" => {
                let k =
                    EncodingKey::from_rsa_pem(self.private_key_pem.as_bytes()).map_err(|e| {
                        OAuthError::InvalidConfig(format!("invalid RSA signing key PEM: {e}"))
                    })?;
                (Algorithm::RS256, k)
            }
            "ES256" => {
                let k = EncodingKey::from_ec_pem(self.private_key_pem.as_bytes()).map_err(|e| {
                    OAuthError::InvalidConfig(format!("invalid EC signing key PEM: {e}"))
                })?;
                (Algorithm::ES256, k)
            }
            other => {
                return Err(OAuthError::InvalidConfig(format!(
                    "unsupported signing algorithm '{other}'"
                )));
            }
        };
        let mut header = Header::new(alg);
        header.kid = Some(self.kid.clone());
        Ok((header, key))
    }
}

/// Creates the database tables required by the OAuth 2.0 / OIDC provider models.
///
/// Call once at application startup or in migrations/tests (never per request).
pub async fn create_oauth_provider_tables(db: &Database) -> Result<(), djangors_orm::OrmError> {
    let mut conn = db.conn();
    let d = conn.dialect();
    let pk = d.auto_pk_type();
    let bigint = d.bigint_type();
    let b = d.bool_type();
    let ts = d.timestamp_type();

    let stmts = [
        format!(
            "CREATE TABLE IF NOT EXISTS oauth_application (
                id {pk},
                name VARCHAR(150) NOT NULL,
                client_id VARCHAR(128) NOT NULL UNIQUE,
                client_secret_hash VARCHAR(128) NOT NULL,
                redirect_uris VARCHAR(2000) NOT NULL,
                grant_types VARCHAR(255) NOT NULL,
                allowed_scopes VARCHAR(500) NOT NULL,
                confidential {b} NOT NULL,
                skip_consent {b} NOT NULL,
                owner_id {bigint},
                is_active {b} NOT NULL,
                created_at {ts} NOT NULL
            )"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS oauth_authorization_code (
                id {pk},
                application {bigint} NOT NULL REFERENCES oauth_application(id) ON DELETE CASCADE,
                \"user\" {bigint} NOT NULL REFERENCES auth_user(id) ON DELETE CASCADE,
                code_hash VARCHAR(128) NOT NULL UNIQUE,
                redirect_uri VARCHAR(1000) NOT NULL,
                scopes VARCHAR(500) NOT NULL,
                code_challenge VARCHAR(128) NOT NULL,
                code_challenge_method VARCHAR(16) NOT NULL,
                nonce VARCHAR(255) NOT NULL,
                created_at {ts} NOT NULL,
                expires_at {ts} NOT NULL,
                used_at {ts}
            )"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS oauth_access_token (
                id {pk},
                application {bigint} NOT NULL REFERENCES oauth_application(id) ON DELETE CASCADE,
                user_id {bigint},
                token_hash VARCHAR(128) NOT NULL UNIQUE,
                scopes VARCHAR(500) NOT NULL,
                created_at {ts} NOT NULL,
                expires_at {ts} NOT NULL,
                revoked_at {ts}
            )"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS oauth_refresh_token (
                id {pk},
                application {bigint} NOT NULL REFERENCES oauth_application(id) ON DELETE CASCADE,
                \"user\" {bigint} NOT NULL REFERENCES auth_user(id) ON DELETE CASCADE,
                token_hash VARCHAR(128) NOT NULL UNIQUE,
                family_id VARCHAR(64) NOT NULL,
                scopes VARCHAR(500) NOT NULL,
                created_at {ts} NOT NULL,
                expires_at {ts} NOT NULL,
                used_at {ts},
                revoked_at {ts}
            )"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS oauth_signing_key (
                id {pk},
                kid VARCHAR(64) NOT NULL UNIQUE,
                algorithm VARCHAR(16) NOT NULL,
                private_key_pem TEXT NOT NULL,
                public_jwk_json TEXT NOT NULL,
                is_active {b} NOT NULL,
                created_at {ts} NOT NULL
            )"
        ),
    ];
    for sql in stmts {
        conn.execute(&sql, &[])
            .await
            .map_err(djangors_orm::OrmError::Query)?;
    }
    Ok(())
}

/// Registers the OAuth 2.0 / OIDC provider models with an [`djangors_admin::AdminSite`].
#[cfg(feature = "admin")]
pub fn register_provider_admin(site: &djangors_admin::AdminSite) {
    site.register::<OAuthApplication>();
    site.register::<AuthorizationCode>();
    site.register::<AccessToken>();
    site.register::<RefreshToken>();
    site.register::<SigningKey>();
}

/// Revokes all active access and refresh tokens belonging to `application_id`.
///
/// Returns the total number of token records revoked.
pub async fn revoke_app_tokens(db: &Database, application_id: i64) -> Result<u64, OAuthError> {
    let now = Utc::now();
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);

    let access_sql = format!(
        "UPDATE oauth_access_token SET revoked_at = {p1}
         WHERE application = {p2} AND revoked_at IS NULL"
    );
    let access_revoked = conn
        .execute(
            &access_sql,
            &[BindValue::DateTime(now), BindValue::I64(application_id)],
        )
        .await
        .map_err(djangors_orm::OrmError::Query)?;

    let refresh_sql = format!(
        "UPDATE oauth_refresh_token SET revoked_at = {p1}
         WHERE application = {p2} AND revoked_at IS NULL"
    );
    let refresh_revoked = conn
        .execute(
            &refresh_sql,
            &[BindValue::DateTime(now), BindValue::I64(application_id)],
        )
        .await
        .map_err(djangors_orm::OrmError::Query)?;

    Ok(access_revoked + refresh_revoked)
}

/// Authenticated OAuth 2.0 Bearer token extracted from `Authorization: Bearer <token>`.
#[derive(Debug, Clone)]
pub struct OAuthBearer {
    /// Persisted [`AccessToken`] record.
    pub token: AccessToken,
    /// ID of the [`OAuthApplication`] that issued this token.
    pub application_id: i64,
    /// Authenticated user ID, or `None` for client-credentials tokens.
    pub user_id: Option<i64>,
    /// Granted scopes parsed from `token.scopes`.
    pub scopes: Vec<String>,
}

impl OAuthBearer {
    /// Returns `true` if `scope` was granted to this token.
    pub fn has_scope(&self, scope: &str) -> bool {
        !scope.is_empty() && self.scopes.iter().any(|s| s == scope)
    }

    /// Returns `true` if all `scopes` were granted to this token.
    pub fn has_all_scopes(&self, scopes: &[&str]) -> bool {
        scopes.iter().all(|s| self.has_scope(s))
    }

    /// Succeeds if `scope` is granted, returning `403 Forbidden` otherwise.
    pub fn require_scope(&self, scope: &str) -> Result<(), DjangorsError> {
        if self.has_scope(scope) {
            Ok(())
        } else {
            Err(DjangorsError::api(
                StatusCode::FORBIDDEN,
                "insufficient_scope",
                format!("missing required OAuth scope '{scope}'"),
            ))
        }
    }

    /// Succeeds if every scope in `scopes` is granted, returning `403 Forbidden` otherwise.
    pub fn require_scopes(&self, scopes: &[&str]) -> Result<(), DjangorsError> {
        for s in scopes {
            self.require_scope(s)?;
        }
        Ok(())
    }

    /// Loads the owning [`User`] for user-delegated tokens (`None` for client-credentials tokens).
    pub async fn load_user(&self, db: &Database) -> Result<Option<User>, OAuthError> {
        let Some(uid) = self.user_id else {
            return Ok(None);
        };
        Ok(User::objects()
            .filter(djangors_orm::q!(id = uid))?
            .first(db)
            .await?)
    }
}

#[async_trait::async_trait]
impl FromRequest for OAuthBearer {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                DjangorsError::Unauthorized("missing Authorization header".to_string())
            })?;

        let raw_token = extract_bearer_token(auth_header).ok_or_else(|| {
            DjangorsError::Unauthorized("expected Bearer authorization scheme".to_string())
        })?;
        if raw_token.is_empty() {
            return Err(DjangorsError::Unauthorized(
                "empty Bearer token".to_string(),
            ));
        }

        let db = req
            .state::<Database>()
            .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

        let token_hash = sha256_hex(raw_token);
        let token = AccessToken::objects()
            .filter(djangors_orm::q!(token_hash = token_hash.as_str()))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .first(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .ok_or_else(|| DjangorsError::Unauthorized("invalid access token".to_string()))?;

        let now = Utc::now();
        if token.revoked_at.is_some() || token.expires_at <= now {
            return Err(DjangorsError::Unauthorized(
                "access token is expired or revoked".to_string(),
            ));
        }

        let app = OAuthApplication::objects()
            .filter(djangors_orm::q!(id = token.application.id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .first(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if !matches!(app, Some(a) if a.is_active) {
            return Err(DjangorsError::Unauthorized(
                "OAuth application disabled".to_string(),
            ));
        }

        if let Some(uid) = token.user_id {
            let user = User::objects()
                .filter(djangors_orm::q!(id = uid))
                .map_err(|e| DjangorsError::Internal(e.to_string()))?
                .first(db)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            if !matches!(user, Some(u) if u.is_active) {
                return Err(DjangorsError::Unauthorized(
                    "token user is inactive".to_string(),
                ));
            }
        }

        let scopes = token
            .scopes
            .split_whitespace()
            .map(ToString::to_string)
            .collect();

        Ok(Self {
            application_id: token.application.id,
            user_id: token.user_id,
            scopes,
            token,
        })
    }
}

/// Permission helper descriptor enforcing a Bearer token scope (`HasOAuthScope("read")`).
#[derive(Debug, Clone, Copy)]
pub struct HasOAuthScope(pub &'static str);

impl HasOAuthScope {
    /// Verifies that `req` carries a valid [`OAuthBearer`] token with scope `self.0`.
    pub async fn check(&self, req: &Request) -> Result<OAuthBearer, DjangorsError> {
        let bearer = OAuthBearer::from_request(req).await?;
        bearer.require_scope(self.0)?;
        Ok(bearer)
    }

    /// Verifies that `req` carries a valid [`OAuthBearer`] token holding all `scopes`.
    pub async fn check_all(req: &Request, scopes: &[&str]) -> Result<OAuthBearer, DjangorsError> {
        let bearer = OAuthBearer::from_request(req).await?;
        bearer.require_scopes(scopes)?;
        Ok(bearer)
    }
}

/// Returns a [`Router`] mounting the OAuth 2.0 / OIDC provider endpoints:
///
/// - `GET /o/authorize`
/// - `POST /o/authorize`
/// - `POST /o/token`
/// - `POST /o/revoke`
/// - `POST /o/introspect`
/// - `GET /o/userinfo`
/// - `POST /o/userinfo`
/// - `GET /.well-known/openid-configuration`
/// - `GET /o/jwks`
pub fn oauth_provider_router() -> Router {
    Router::new()
        .get("/o/authorize", authorize_get_view)
        .post("/o/authorize", authorize_post_view)
        .post("/o/token", token_view)
        .post("/o/revoke", revoke_view)
        .post("/o/introspect", introspect_view)
        .get("/o/userinfo", userinfo_view)
        .post("/o/userinfo", userinfo_view)
        .get(
            "/.well-known/openid-configuration",
            openid_configuration_view,
        )
        .get("/o/jwks", jwks_view)
}

#[derive(Deserialize, Default)]
struct AuthorizeParams {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    nonce: Option<String>,
    decision: Option<String>,
}

/// `GET /o/authorize`: validates parameters and renders consent (or auto-approves when `skip_consent`).
pub async fn authorize_get_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let db = require_db(&req)?;
    let cfg = require_config(&req)?;
    let query = req.raw_query().unwrap_or_default();
    let params: AuthorizeParams = serde_urlencoded::from_str(query).unwrap_or_default();

    let validated = match validate_authorize_params(db, &cfg, &params).await {
        Ok(v) => v,
        Err(AuthorizeOutcome::DirectError(msg)) => {
            return Ok(Response::html(
                StatusCode::BAD_REQUEST,
                render_error_html(&msg),
            ));
        }
        Err(AuthorizeOutcome::RedirectError(url)) => return Ok(redirect_found(&url)),
    };

    let user = match load_logged_in_user(&req, db).await? {
        Some(u) => u,
        None => {
            let next_target = match req.raw_query() {
                Some(q) if !q.is_empty() => format!("{}?{}", req.path(), q),
                _ => req.path().to_string(),
            };
            let sep = if cfg.login_url.contains('?') {
                '&'
            } else {
                '?'
            };
            let encoded_next: String =
                url::form_urlencoded::byte_serialize(next_target.as_bytes()).collect();
            return Ok(redirect_found(&format!(
                "{}{sep}next={encoded_next}",
                cfg.login_url
            )));
        }
    };

    if validated.app.skip_consent {
        let code = issue_authorization_code(db, &cfg, &validated, user.id).await?;
        let location = append_query_params(
            &validated.redirect_uri,
            &build_code_redirect_params(&code, validated.state.as_deref()),
        );
        return Ok(redirect_found(&location));
    }

    let csrf_token = extract_csrf_cookie(&req);
    let html = render_consent_html(&validated, &user, csrf_token.as_deref());
    Ok(Response::html(StatusCode::OK, html))
}

/// `POST /o/authorize`: processes consent submission and issues an authorization code.
pub async fn authorize_post_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let db = require_db(&req)?;
    let cfg = require_config(&req)?;
    let params = parse_urlencoded_body::<AuthorizeParams>(&req)
        .await
        .unwrap_or_default();

    let validated = match validate_authorize_params(db, &cfg, &params).await {
        Ok(v) => v,
        Err(AuthorizeOutcome::DirectError(msg)) => {
            return Ok(Response::html(
                StatusCode::BAD_REQUEST,
                render_error_html(&msg),
            ));
        }
        Err(AuthorizeOutcome::RedirectError(url)) => return Ok(redirect_found(&url)),
    };

    let user = match load_logged_in_user(&req, db).await? {
        Some(u) => u,
        None => {
            return Err(DjangorsError::Unauthorized(
                "user session required to authorize OAuth client".to_string(),
            ));
        }
    };

    let approved = params.decision.as_deref() == Some("approve");
    if !approved {
        let location = append_error_redirect(
            &validated.redirect_uri,
            "access_denied",
            "Resource owner denied the authorization request",
            validated.state.as_deref(),
        );
        return Ok(redirect_found(&location));
    }

    let code = issue_authorization_code(db, &cfg, &validated, user.id).await?;
    let location = append_query_params(
        &validated.redirect_uri,
        &build_code_redirect_params(&code, validated.state.as_deref()),
    );
    Ok(redirect_found(&location))
}

/// `POST /o/token`: implements `authorization_code`, `client_credentials`, and `refresh_token` grants.
pub async fn token_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = require_db(&req)?;
    let cfg = require_config(&req)?;
    let form = parse_urlencoded_map(&req).await;

    let app = match authenticate_token_client(&req, db, &form, true).await {
        Ok(app) => app,
        Err((status, err_code, desc)) => return token_error_response(status, err_code, &desc),
    };

    let grant_type = form.get("grant_type").map(String::as_str).unwrap_or("");
    match grant_type {
        "authorization_code" => handle_authorization_code_grant(db, &cfg, &app, &form).await,
        "client_credentials" => handle_client_credentials_grant(db, &cfg, &app, &form).await,
        "refresh_token" => handle_refresh_token_grant(db, &cfg, &app, &form).await,
        "" => token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "missing grant_type",
        ),
        _ => token_error_response(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "unsupported grant_type",
        ),
    }
}

/// `POST /o/revoke`: RFC 7009 token revocation.
pub async fn revoke_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = require_db(&req)?;
    let form = parse_urlencoded_map(&req).await;

    let app = match authenticate_token_client(&req, db, &form, true).await {
        Ok(app) => app,
        Err((status, err_code, desc)) => return token_error_response(status, err_code, &desc),
    };

    let raw_token = form.get("token").map(String::as_str).unwrap_or("");
    if raw_token.is_empty() {
        return token_error_response(StatusCode::BAD_REQUEST, "invalid_request", "missing token");
    }

    let hint = form.get("token_type_hint").map(String::as_str);
    let token_hash = sha256_hex(raw_token);
    let now = Utc::now();

    if hint != Some("refresh_token") {
        revoke_access_token_hash(db, app.id, &token_hash, now).await?;
    }
    if hint != Some("access_token") {
        revoke_refresh_token_and_family_by_hash(db, app.id, &token_hash, now).await?;
    }
    if hint == Some("refresh_token") {
        // RFC 7009 §2.1: if hint wasn't found, also check the other token type.
        revoke_access_token_hash(db, app.id, &token_hash, now).await?;
    } else if hint == Some("access_token") {
        revoke_refresh_token_and_family_by_hash(db, app.id, &token_hash, now).await?;
    }

    let mut resp = Response::text(StatusCode::OK, "");
    resp = resp.header("cache-control", "no-store");
    Ok(resp)
}

/// `POST /o/introspect`: RFC 7662 token introspection.
pub async fn introspect_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = require_db(&req)?;
    let cfg = require_config(&req)?;
    let form = parse_urlencoded_map(&req).await;

    // RFC 7662 requires authenticated client/resource-server caller.
    let _caller_app = match authenticate_token_client(&req, db, &form, true).await {
        Ok(app) => app,
        Err((status, err_code, desc)) => return token_error_response(status, err_code, &desc),
    };

    let raw_token = form.get("token").map(String::as_str).unwrap_or("");
    if raw_token.is_empty() {
        return no_store_json(StatusCode::OK, &serde_json::json!({ "active": false }));
    }

    let token_hash = sha256_hex(raw_token);
    let hint = form.get("token_type_hint").map(String::as_str);
    let now = Utc::now();

    if hint != Some("refresh_token") {
        if let Some(payload) = introspect_access_token(db, &cfg, &token_hash, now).await? {
            return no_store_json(StatusCode::OK, &payload);
        }
    }
    if let Some(payload) = introspect_refresh_token(db, &cfg, &token_hash, now).await? {
        return no_store_json(StatusCode::OK, &payload);
    }
    if hint == Some("refresh_token") {
        if let Some(payload) = introspect_access_token(db, &cfg, &token_hash, now).await? {
            return no_store_json(StatusCode::OK, &payload);
        }
    }

    no_store_json(StatusCode::OK, &serde_json::json!({ "active": false }))
}

/// `GET|POST /o/userinfo`: returns OIDC claims filtered by the Bearer token's scopes.
pub async fn userinfo_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let bearer = OAuthBearer::from_request(&req).await?;
    bearer.require_scope("openid")?;
    let db = require_db(&req)?;
    let user = bearer
        .load_user(db)
        .await
        .map_err(DjangorsError::from)?
        .ok_or_else(|| {
            DjangorsError::Unauthorized("userinfo requires a user-delegated token".to_string())
        })?;

    let mut claims = serde_json::Map::new();
    claims.insert(
        "sub".to_string(),
        serde_json::Value::String(user.id.to_string()),
    );
    if bearer.has_scope("profile") {
        claims.insert(
            "preferred_username".to_string(),
            serde_json::Value::String(user.username.clone()),
        );
    }
    if bearer.has_scope("email") && !user.email.is_empty() {
        let verified = has_verified_email(db, user.id).await.unwrap_or(false);
        claims.insert("email".to_string(), serde_json::Value::String(user.email));
        claims.insert(
            "email_verified".to_string(),
            serde_json::Value::Bool(verified),
        );
    }

    no_store_json(StatusCode::OK, &serde_json::Value::Object(claims))
}

/// `GET /.well-known/openid-configuration`: serves OpenID Connect Provider metadata.
pub async fn openid_configuration_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let cfg = require_config(&req)?;
    let iss = cfg.issuer.trim_end_matches('/');
    let metadata = serde_json::json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}/o/authorize"),
        "token_endpoint": format!("{iss}/o/token"),
        "userinfo_endpoint": format!("{iss}/o/userinfo"),
        "revocation_endpoint": format!("{iss}/o/revoke"),
        "introspection_endpoint": format!("{iss}/o/introspect"),
        "jwks_uri": format!("{iss}/o/jwks"),
        "response_types_supported": ["code"],
        "grant_types_supported": [
            "authorization_code",
            "client_credentials",
            "refresh_token"
        ],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256", "ES256"],
        "scopes_supported": cfg.supported_scopes,
        "token_endpoint_auth_methods_supported": [
            "client_secret_basic",
            "client_secret_post",
            "none"
        ],
        "code_challenge_methods_supported": ["S256"],
        "claims_supported": [
            "sub",
            "iss",
            "aud",
            "exp",
            "iat",
            "nonce",
            "preferred_username",
            "email",
            "email_verified"
        ]
    });
    Response::json(StatusCode::OK, &metadata)
}

/// `GET /o/jwks`: serves active public JSON Web Keys (`keys` array).
pub async fn jwks_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = require_db(&req)?;
    let keys = SigningKey::objects()
        .filter(djangors_orm::q!(is_active = true))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let mut jwks = Vec::with_capacity(keys.len());
    for key in keys {
        let val: serde_json::Value = serde_json::from_str(&key.public_jwk_json).map_err(|e| {
            DjangorsError::Internal(format!("corrupt public JWK on key '{}': {e}", key.kid))
        })?;
        jwks.push(val);
    }

    let resp = Response::json(StatusCode::OK, &serde_json::json!({ "keys": jwks }))?;
    Ok(resp.header("cache-control", "public, max-age=300"))
}

// ---------------------------------------------------------------------------
// Internal validation & grant handlers
// ---------------------------------------------------------------------------

struct ValidatedAuthorizeRequest {
    app: OAuthApplication,
    redirect_uri: String,
    scopes: Vec<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    nonce: Option<String>,
}

enum AuthorizeOutcome {
    DirectError(String),
    RedirectError(String),
}

async fn validate_authorize_params(
    db: &Database,
    cfg: &OAuthProviderConfig,
    params: &AuthorizeParams,
) -> Result<ValidatedAuthorizeRequest, AuthorizeOutcome> {
    let client_id = params.client_id.as_deref().unwrap_or("").trim();
    if client_id.is_empty() {
        return Err(AuthorizeOutcome::DirectError(
            "Missing client_id parameter.".to_string(),
        ));
    }

    let app = OAuthApplication::objects()
        .filter(djangors_orm::q!(client_id = client_id))
        .map_err(|_| AuthorizeOutcome::DirectError("Invalid client lookup.".to_string()))?
        .first(db)
        .await
        .map_err(|_| AuthorizeOutcome::DirectError("Failed loading client.".to_string()))?
        .filter(|a| a.is_active)
        .ok_or_else(|| {
            AuthorizeOutcome::DirectError("Unknown or inactive client_id.".to_string())
        })?;

    let redirect_uri = params.redirect_uri.as_deref().unwrap_or("").trim();
    if !app.allows_redirect_uri(redirect_uri) {
        return Err(AuthorizeOutcome::DirectError(
            "Invalid or unregistered redirect_uri.".to_string(),
        ));
    }

    let state = params.state.clone().filter(|s| !s.is_empty());

    let response_type = params.response_type.as_deref().unwrap_or("");
    if response_type != "code" {
        return Err(AuthorizeOutcome::RedirectError(append_error_redirect(
            redirect_uri,
            "unsupported_response_type",
            "Only response_type=code is supported",
            state.as_deref(),
        )));
    }

    if !app.allows_grant_type("authorization_code") {
        return Err(AuthorizeOutcome::RedirectError(append_error_redirect(
            redirect_uri,
            "unauthorized_client",
            "Application is not authorized for authorization_code grant",
            state.as_deref(),
        )));
    }

    let scopes = match resolve_requested_scopes(params.scope.as_deref().unwrap_or(""), &app, cfg) {
        Ok(s) => s,
        Err(msg) => {
            return Err(AuthorizeOutcome::RedirectError(append_error_redirect(
                redirect_uri,
                "invalid_scope",
                &msg,
                state.as_deref(),
            )));
        }
    };

    let requires_pkce = !app.confidential || cfg.require_pkce_for_all_clients;
    let challenge = params.code_challenge.as_deref().unwrap_or("").trim();
    let method = params.code_challenge_method.as_deref().unwrap_or("").trim();

    if challenge.is_empty() {
        if requires_pkce {
            return Err(AuthorizeOutcome::RedirectError(append_error_redirect(
                redirect_uri,
                "invalid_request",
                "PKCE code_challenge with method S256 is required",
                state.as_deref(),
            )));
        }
    } else if method != "S256" || !is_valid_pkce_challenge(challenge) {
        return Err(AuthorizeOutcome::RedirectError(append_error_redirect(
            redirect_uri,
            "invalid_request",
            "code_challenge_method must be S256 with a valid challenge",
            state.as_deref(),
        )));
    }

    Ok(ValidatedAuthorizeRequest {
        app,
        redirect_uri: redirect_uri.to_string(),
        scopes,
        state,
        code_challenge: if challenge.is_empty() {
            None
        } else {
            Some(challenge.to_string())
        },
        nonce: params.nonce.clone().filter(|n| !n.is_empty()),
    })
}

fn resolve_requested_scopes(
    raw: &str,
    app: &OAuthApplication,
    cfg: &OAuthProviderConfig,
) -> Result<Vec<String>, String> {
    let app_allowed: Vec<&str> = app.allowed_scopes.split_whitespace().collect();
    let requested: Vec<String> = raw
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();

    if requested.is_empty() {
        if !app_allowed.is_empty() {
            return Ok(app_allowed.into_iter().map(ToString::to_string).collect());
        }
        return Err("missing requested scope".to_string());
    }

    for scope in &requested {
        let permitted_by_app = if app_allowed.is_empty() {
            cfg.supported_scopes.iter().any(|s| s == scope)
        } else {
            app_allowed.contains(&scope.as_str())
        };
        if !permitted_by_app {
            return Err(format!("scope '{scope}' is not permitted for this client"));
        }
    }
    Ok(requested)
}

async fn issue_authorization_code(
    db: &Database,
    cfg: &OAuthProviderConfig,
    validated: &ValidatedAuthorizeRequest,
    user_id: i64,
) -> Result<String, DjangorsError> {
    let raw_code = random_urlsafe_token(32);
    let now = Utc::now();
    let ttl = ChronoDuration::from_std(cfg.auth_code_ttl)
        .unwrap_or_else(|_| ChronoDuration::seconds(DEFAULT_AUTH_CODE_TTL_SECS as i64));

    let record = AuthorizationCode {
        id: 0,
        application: ForeignKey::new(validated.app.id),
        user: ForeignKey::new(user_id),
        code_hash: sha256_hex(&raw_code),
        redirect_uri: validated.redirect_uri.clone(),
        scopes: validated.scopes.join(" "),
        code_challenge: validated.code_challenge.clone().unwrap_or_default(),
        code_challenge_method: if validated.code_challenge.is_some() {
            "S256".to_string()
        } else {
            String::new()
        },
        nonce: validated.nonce.clone().unwrap_or_default(),
        created_at: now,
        expires_at: now + ttl,
        used_at: None,
    };
    let _saved = record
        .save(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok(raw_code)
}

async fn authenticate_token_client(
    req: &Request,
    db: &Database,
    form: &HashMap<String, String>,
    allow_public_without_secret: bool,
) -> Result<OAuthApplication, (StatusCode, &'static str, String)> {
    let basic = parse_basic_auth(req);
    let form_id = form
        .get("client_id")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty());
    let form_secret = form
        .get("client_secret")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty());

    // RFC 6749 §2.3: client MUST NOT use more than one authentication method.
    if basic.is_some() && (form_id.is_some() || form_secret.is_some()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "multiple client authentication methods used".to_string(),
        ));
    }

    let (client_id, client_secret, used_basic) = match basic {
        Some((cid, csec)) => (cid, Some(csec), true),
        None => {
            let cid = form_id.unwrap_or("").to_string();
            let csec = form_secret.map(ToString::to_string);
            (cid, csec, false)
        }
    };

    if client_id.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "missing client_id".to_string(),
        ));
    }

    let app = OAuthApplication::objects()
        .filter(djangors_orm::q!(client_id = client_id.as_str()))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                e.to_string(),
            )
        })?
        .first(db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                e.to_string(),
            )
        })?
        .filter(|a| a.is_active)
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                "unknown or disabled client".to_string(),
            )
        })?;

    if app.confidential {
        let Some(secret) = client_secret.as_deref() else {
            return Err((
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                "confidential client requires client_secret".to_string(),
            ));
        };
        if !app.verify_secret(secret) {
            return Err((
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                "invalid client credentials".to_string(),
            ));
        }
    } else if !allow_public_without_secret || used_basic || client_secret.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "public client must not present client_secret".to_string(),
        ));
    }

    Ok(app)
}

async fn handle_authorization_code_grant(
    db: &Database,
    cfg: &OAuthProviderConfig,
    app: &OAuthApplication,
    form: &HashMap<String, String>,
) -> Result<Response, DjangorsError> {
    if !app.allows_grant_type("authorization_code") {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "unauthorized_client",
            "client is not authorized for authorization_code grant",
        );
    }

    let raw_code = form.get("code").map(String::as_str).unwrap_or("");
    let redirect_uri = form.get("redirect_uri").map(String::as_str).unwrap_or("");
    if raw_code.is_empty() || redirect_uri.is_empty() {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code and redirect_uri are required",
        );
    }

    let code_hash = sha256_hex(raw_code);
    let now = Utc::now();

    // Atomic single-use claim bound to both code_hash and application ID.
    let claimed = {
        let mut conn = db.conn();
        let p1 = conn.dialect().placeholder(1);
        let p2 = conn.dialect().placeholder(2);
        let p3 = conn.dialect().placeholder(3);
        let p4 = conn.dialect().placeholder(4);
        let sql = format!(
            "UPDATE oauth_authorization_code SET used_at = {p1}
             WHERE code_hash = {p2} AND application = {p3} AND used_at IS NULL AND expires_at > {p4}"
        );
        conn.execute(
            &sql,
            &[
                BindValue::DateTime(now),
                BindValue::Text(code_hash.clone()),
                BindValue::I64(app.id),
                BindValue::DateTime(now),
            ],
        )
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };

    if claimed != 1 {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "invalid, expired, or already-used authorization code",
        );
    }

    let auth_code = AuthorizationCode::objects()
        .filter(djangors_orm::q!(code_hash = code_hash.as_str()))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .ok_or_else(|| DjangorsError::Internal("claimed code vanished".to_string()))?;

    if auth_code.redirect_uri != redirect_uri {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "redirect_uri mismatch",
        );
    }

    // Verify PKCE S256 whenever a challenge was stored or client is public / policy requires PKCE.
    let requires_pkce = !auth_code.code_challenge.is_empty()
        || !app.confidential
        || cfg.require_pkce_for_all_clients;
    if requires_pkce {
        let verifier = form.get("code_verifier").map(String::as_str).unwrap_or("");
        if auth_code.code_challenge_method != "S256"
            || !is_valid_pkce_verifier(verifier)
            || !verify_pkce_s256(verifier, &auth_code.code_challenge)
        {
            return token_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "PKCE code_verifier verification failed",
            );
        }
    }

    let user = User::objects()
        .filter(djangors_orm::q!(id = auth_code.user.id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let Some(user) = user.filter(|u| u.is_active) else {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "user account is inactive",
        );
    };

    let nonce = if auth_code.nonce.is_empty() {
        None
    } else {
        Some(auth_code.nonce.as_str())
    };
    issue_user_token_bundle(db, cfg, app, &user, &auth_code.scopes, nonce, None).await
}

async fn handle_client_credentials_grant(
    db: &Database,
    cfg: &OAuthProviderConfig,
    app: &OAuthApplication,
    form: &HashMap<String, String>,
) -> Result<Response, DjangorsError> {
    if !app.confidential || !app.allows_grant_type("client_credentials") {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "unauthorized_client",
            "client is not authorized for client_credentials grant",
        );
    }

    let scopes = match resolve_requested_scopes(
        form.get("scope").map(String::as_str).unwrap_or(""),
        app,
        cfg,
    ) {
        Ok(s) => s,
        Err(msg) => return token_error_response(StatusCode::BAD_REQUEST, "invalid_scope", &msg),
    };
    let scope_str = scopes.join(" ");

    let (raw_access, expires_in) = create_access_token(db, cfg, app.id, None, &scope_str).await?;

    let payload = serde_json::json!({
        "access_token": raw_access,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "scope": scope_str,
    });
    no_store_json(StatusCode::OK, &payload)
}

async fn handle_refresh_token_grant(
    db: &Database,
    cfg: &OAuthProviderConfig,
    app: &OAuthApplication,
    form: &HashMap<String, String>,
) -> Result<Response, DjangorsError> {
    if !app.allows_grant_type("refresh_token") {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "unauthorized_client",
            "client is not authorized for refresh_token grant",
        );
    }

    let raw_refresh = form.get("refresh_token").map(String::as_str).unwrap_or("");
    if raw_refresh.is_empty() {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "missing refresh_token",
        );
    }

    let token_hash = sha256_hex(raw_refresh);
    let existing = RefreshToken::objects()
        .filter(djangors_orm::q!(token_hash = token_hash.as_str()))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let Some(rt) = existing else {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "invalid refresh_token",
        );
    };

    if rt.application.id != app.id {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token does not belong to this client",
        );
    }

    let now = Utc::now();

    // Token reuse detection: if an already-rotated or revoked refresh token is re-presented,
    // revoke the entire token family immediately.
    if rt.used_at.is_some() || rt.revoked_at.is_some() {
        revoke_refresh_family(db, &rt.family_id, now).await?;
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token reuse detected; token family revoked",
        );
    }

    if rt.expires_at <= now {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token expired",
        );
    }

    // Optional scope down-scoping (RFC 6749 §6); never allow escalating beyond original grant.
    let granted_scope = if let Some(req_scope) = form.get("scope").filter(|s| !s.trim().is_empty())
    {
        let original: Vec<&str> = rt.scopes.split_whitespace().collect();
        let requested: Vec<&str> = req_scope.split_whitespace().collect();
        if !requested.iter().all(|s| original.contains(s)) {
            return token_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_scope",
                "requested scope exceeds original grant",
            );
        }
        requested.join(" ")
    } else {
        rt.scopes.clone()
    };

    // Atomically mark this refresh token as used; if a concurrent request beat us, revoke family.
    let claimed = {
        let mut conn = db.conn();
        let p1 = conn.dialect().placeholder(1);
        let p2 = conn.dialect().placeholder(2);
        let p3 = conn.dialect().placeholder(3);
        let sql = format!(
            "UPDATE oauth_refresh_token SET used_at = {p1}
             WHERE id = {p2} AND used_at IS NULL AND revoked_at IS NULL AND expires_at > {p3}"
        );
        conn.execute(
            &sql,
            &[
                BindValue::DateTime(now),
                BindValue::I64(rt.id),
                BindValue::DateTime(now),
            ],
        )
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };

    if claimed != 1 {
        revoke_refresh_family(db, &rt.family_id, now).await?;
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token reuse detected; token family revoked",
        );
    }

    let user = User::objects()
        .filter(djangors_orm::q!(id = rt.user.id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let Some(user) = user.filter(|u| u.is_active) else {
        return token_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "user account is inactive",
        );
    };

    issue_user_token_bundle(
        db,
        cfg,
        app,
        &user,
        &granted_scope,
        None,
        Some(rt.family_id),
    )
    .await
}

async fn issue_user_token_bundle(
    db: &Database,
    cfg: &OAuthProviderConfig,
    app: &OAuthApplication,
    user: &User,
    scope_str: &str,
    nonce: Option<&str>,
    existing_family_id: Option<String>,
) -> Result<Response, DjangorsError> {
    let (raw_access, expires_in) =
        create_access_token(db, cfg, app.id, Some(user.id), scope_str).await?;

    let mut body = serde_json::Map::new();
    body.insert(
        "access_token".to_string(),
        serde_json::Value::String(raw_access.clone()),
    );
    body.insert(
        "token_type".to_string(),
        serde_json::Value::String("Bearer".to_string()),
    );
    body.insert(
        "expires_in".to_string(),
        serde_json::Value::from(expires_in),
    );
    body.insert(
        "scope".to_string(),
        serde_json::Value::String(scope_str.to_string()),
    );

    let scope_list: Vec<&str> = scope_str.split_whitespace().collect();
    if app.allows_grant_type("refresh_token")
        && (existing_family_id.is_some() || scope_list.contains(&"offline_access"))
    {
        let raw_refresh =
            create_refresh_token(db, cfg, app.id, user.id, scope_str, existing_family_id).await?;
        body.insert(
            "refresh_token".to_string(),
            serde_json::Value::String(raw_refresh),
        );
    }

    if scope_list.contains(&"openid") {
        let id_token = sign_id_token(db, cfg, app, user, &scope_list, &raw_access, nonce).await?;
        body.insert("id_token".to_string(), serde_json::Value::String(id_token));
    }

    no_store_json(StatusCode::OK, &serde_json::Value::Object(body))
}

async fn create_access_token(
    db: &Database,
    cfg: &OAuthProviderConfig,
    application_id: i64,
    user_id: Option<i64>,
    scope_str: &str,
) -> Result<(String, i64), DjangorsError> {
    let raw = random_urlsafe_token(32);
    let now = Utc::now();
    let ttl_secs = cfg.access_token_ttl.as_secs().max(1) as i64;
    let expires_at = now + ChronoDuration::seconds(ttl_secs);

    let record = AccessToken {
        id: 0,
        application: ForeignKey::new(application_id),
        user_id,
        token_hash: sha256_hex(&raw),
        scopes: scope_str.to_string(),
        created_at: now,
        expires_at,
        revoked_at: None,
    };
    let _saved = record
        .save(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok((raw, ttl_secs))
}

async fn create_refresh_token(
    db: &Database,
    cfg: &OAuthProviderConfig,
    application_id: i64,
    user_id: i64,
    scope_str: &str,
    existing_family_id: Option<String>,
) -> Result<String, DjangorsError> {
    let raw = random_urlsafe_token(36);
    let family_id = existing_family_id.unwrap_or_else(|| random_urlsafe_token(16));
    let now = Utc::now();
    let ttl_secs = cfg.refresh_token_ttl.as_secs().max(1) as i64;

    let record = RefreshToken {
        id: 0,
        application: ForeignKey::new(application_id),
        user: ForeignKey::new(user_id),
        token_hash: sha256_hex(&raw),
        family_id,
        scopes: scope_str.to_string(),
        created_at: now,
        expires_at: now + ChronoDuration::seconds(ttl_secs),
        used_at: None,
        revoked_at: None,
    };
    let _saved = record
        .save(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok(raw)
}

#[derive(Serialize)]
struct IdTokenClaims {
    iss: String,
    sub: String,
    aud: String,
    iat: i64,
    exp: i64,
    at_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email_verified: Option<bool>,
}

async fn sign_id_token(
    db: &Database,
    cfg: &OAuthProviderConfig,
    app: &OAuthApplication,
    user: &User,
    scopes: &[&str],
    access_token: &str,
    nonce: Option<&str>,
) -> Result<String, DjangorsError> {
    let keys = SigningKey::objects()
        .filter(djangors_orm::q!(is_active = true))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .order_by("-id")
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let signing_key = keys.into_iter().next().ok_or_else(|| {
        DjangorsError::Internal("no active OAuth signing key configured".to_string())
    })?;
    let (header, enc_key) = signing_key.encoding_key().map_err(DjangorsError::from)?;

    let now = Utc::now().timestamp();
    let exp = now + cfg.access_token_ttl.as_secs().max(60) as i64;
    let at_digest = Sha256::digest(access_token.as_bytes());
    let at_hash = URL_SAFE_NO_PAD.encode(&at_digest[..at_digest.len() / 2]);

    let (email, email_verified) = if scopes.contains(&"email") && !user.email.is_empty() {
        let verified = has_verified_email(db, user.id).await.unwrap_or(false);
        (Some(user.email.clone()), Some(verified))
    } else {
        (None, None)
    };

    let preferred_username = if scopes.contains(&"profile") {
        Some(user.username.clone())
    } else {
        None
    };

    let claims = IdTokenClaims {
        iss: cfg.issuer.trim_end_matches('/').to_string(),
        sub: user.id.to_string(),
        aud: app.client_id.clone(),
        iat: now,
        exp,
        at_hash,
        nonce: nonce.map(ToString::to_string),
        preferred_username,
        email,
        email_verified,
    };

    jsonwebtoken::encode(&header, &claims, &enc_key)
        .map_err(|e| DjangorsError::Internal(format!("failed signing id_token: {e}")))
}

async fn revoke_access_token_hash(
    db: &Database,
    app_id: i64,
    token_hash: &str,
    now: DateTime<Utc>,
) -> Result<(), DjangorsError> {
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let p3 = conn.dialect().placeholder(3);
    let sql = format!(
        "UPDATE oauth_access_token SET revoked_at = {p1}
         WHERE token_hash = {p2} AND application = {p3} AND revoked_at IS NULL"
    );
    conn.execute(
        &sql,
        &[
            BindValue::DateTime(now),
            BindValue::Text(token_hash.to_string()),
            BindValue::I64(app_id),
        ],
    )
    .await
    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok(())
}

async fn revoke_refresh_token_and_family_by_hash(
    db: &Database,
    app_id: i64,
    token_hash: &str,
    now: DateTime<Utc>,
) -> Result<(), DjangorsError> {
    let rt = RefreshToken::objects()
        .filter(djangors_orm::q!(token_hash = token_hash))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    if let Some(row) = rt {
        if row.application.id == app_id {
            revoke_refresh_family(db, &row.family_id, now).await?;
        }
    }
    Ok(())
}

async fn revoke_refresh_family(
    db: &Database,
    family_id: &str,
    now: DateTime<Utc>,
) -> Result<(), DjangorsError> {
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let sql = format!(
        "UPDATE oauth_refresh_token SET revoked_at = {p1}
         WHERE family_id = {p2} AND revoked_at IS NULL"
    );
    conn.execute(
        &sql,
        &[
            BindValue::DateTime(now),
            BindValue::Text(family_id.to_string()),
        ],
    )
    .await
    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok(())
}

async fn introspect_access_token(
    db: &Database,
    cfg: &OAuthProviderConfig,
    token_hash: &str,
    now: DateTime<Utc>,
) -> Result<Option<serde_json::Value>, DjangorsError> {
    let row = AccessToken::objects()
        .filter(djangors_orm::q!(token_hash = token_hash))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let Some(token) = row else {
        return Ok(None);
    };
    if token.revoked_at.is_some() || token.expires_at <= now {
        return Ok(None);
    }

    let app = OAuthApplication::objects()
        .filter(djangors_orm::q!(id = token.application.id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let Some(app) = app.filter(|a| a.is_active) else {
        return Ok(None);
    };

    let (sub, username) = if let Some(uid) = token.user_id {
        let user = User::objects()
            .filter(djangors_orm::q!(id = uid))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .first(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let Some(u) = user.filter(|u| u.is_active) else {
            return Ok(None);
        };
        (Some(u.id.to_string()), Some(u.username))
    } else {
        (Some(app.client_id.clone()), None)
    };

    Ok(Some(serde_json::json!({
        "active": true,
        "scope": token.scopes,
        "client_id": app.client_id,
        "username": username,
        "token_type": "Bearer",
        "exp": token.expires_at.timestamp(),
        "iat": token.created_at.timestamp(),
        "sub": sub,
        "iss": cfg.issuer.trim_end_matches('/'),
    })))
}

async fn introspect_refresh_token(
    db: &Database,
    cfg: &OAuthProviderConfig,
    token_hash: &str,
    now: DateTime<Utc>,
) -> Result<Option<serde_json::Value>, DjangorsError> {
    let row = RefreshToken::objects()
        .filter(djangors_orm::q!(token_hash = token_hash))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let Some(rt) = row else {
        return Ok(None);
    };
    if rt.revoked_at.is_some() || rt.used_at.is_some() || rt.expires_at <= now {
        return Ok(None);
    }

    let app = OAuthApplication::objects()
        .filter(djangors_orm::q!(id = rt.application.id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let Some(app) = app.filter(|a| a.is_active) else {
        return Ok(None);
    };

    let user = User::objects()
        .filter(djangors_orm::q!(id = rt.user.id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let Some(user) = user.filter(|u| u.is_active) else {
        return Ok(None);
    };

    Ok(Some(serde_json::json!({
        "active": true,
        "scope": rt.scopes,
        "client_id": app.client_id,
        "username": user.username,
        "token_type": "refresh_token",
        "exp": rt.expires_at.timestamp(),
        "iat": rt.created_at.timestamp(),
        "sub": user.id.to_string(),
        "iss": cfg.issuer.trim_end_matches('/'),
    })))
}

// ---------------------------------------------------------------------------
// Utility & HTTP helpers
// ---------------------------------------------------------------------------

fn require_db(req: &Request) -> Result<&Database, DjangorsError> {
    req.state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))
}

fn require_config(req: &Request) -> Result<OAuthProviderConfig, DjangorsError> {
    req.state::<OAuthProviderConfig>()
        .cloned()
        .ok_or_else(|| DjangorsError::Internal("OAuthProviderConfig state absent".to_string()))
}

async fn load_logged_in_user(req: &Request, _db: &Database) -> Result<Option<User>, DjangorsError> {
    if let Ok(Auth(user)) = Auth::<User>::from_request(req).await {
        if user.is_active {
            return Ok(Some(user));
        }
    }
    Ok(None)
}

fn extract_bearer_token(header: &str) -> Option<&str> {
    let mut parts = header.splitn(2, ' ');
    let scheme = parts.next()?.trim();
    let token = parts.next()?.trim();
    if scheme.eq_ignore_ascii_case("Bearer") {
        Some(token)
    } else {
        None
    }
}

fn parse_basic_auth(req: &Request) -> Option<(String, String)> {
    let raw = req.headers().get("authorization")?.to_str().ok()?;
    let mut parts = raw.splitn(2, ' ');
    let scheme = parts.next()?.trim();
    if !scheme.eq_ignore_ascii_case("Basic") {
        return None;
    }
    let encoded = parts.next()?.trim();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let cred = String::from_utf8(decoded).ok()?;
    let (raw_id, raw_secret) = cred.split_once(':')?;
    // RFC 6749 §2.3.1: HTTP Basic client_id and client_secret are form-urlencoded.
    let id = percent_decode_form(raw_id);
    let secret = percent_decode_form(raw_secret);
    Some((id, secret))
}

fn percent_decode_form(input: &str) -> String {
    url::form_urlencoded::parse(format!("v={input}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| input.to_string())
}

async fn parse_urlencoded_body<T: serde::de::DeserializeOwned>(req: &Request) -> Option<T> {
    let bytes = req.body_bytes().await;
    serde_urlencoded::from_bytes::<T>(bytes).ok()
}

async fn parse_urlencoded_map(req: &Request) -> HashMap<String, String> {
    let bytes = req.body_bytes().await;
    url::form_urlencoded::parse(bytes)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

fn random_urlsafe_token(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(&bytes)
}

fn is_valid_pkce_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

fn is_valid_pkce_challenge(challenge: &str) -> bool {
    (43..=128).contains(&challenge.len())
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn verify_pkce_s256(verifier: &str, expected_challenge: &str) -> bool {
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    computed
        .as_bytes()
        .ct_eq(expected_challenge.as_bytes())
        .into()
}

fn token_error_response(
    status: StatusCode,
    error: &str,
    description: &str,
) -> Result<Response, DjangorsError> {
    let mut resp = no_store_json(
        status,
        &serde_json::json!({
            "error": error,
            "error_description": description,
        }),
    )?;
    if status == StatusCode::UNAUTHORIZED {
        resp = resp.header("www-authenticate", "Basic");
    }
    Ok(resp)
}

fn no_store_json<T: Serialize>(status: StatusCode, payload: &T) -> Result<Response, DjangorsError> {
    let resp = Response::json(status, payload)?;
    Ok(resp
        .header("cache-control", "no-store")
        .header("pragma", "no-cache"))
}

fn redirect_found(location: &str) -> Response {
    let safe_loc = if HeaderValue::from_str(location).is_ok() {
        location
    } else {
        "/"
    };
    Response::text(StatusCode::FOUND, "").header("location", safe_loc)
}

fn build_code_redirect_params<'a>(
    code: &'a str,
    state: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut params = vec![("code", code)];
    if let Some(st) = state {
        params.push(("state", st));
    }
    params
}

fn append_error_redirect(
    redirect_uri: &str,
    error: &str,
    description: &str,
    state: Option<&str>,
) -> String {
    let mut params = vec![("error", error), ("error_description", description)];
    if let Some(st) = state {
        params.push(("state", st));
    }
    append_query_params(redirect_uri, &params)
}

fn append_query_params(base: &str, params: &[(&str, &str)]) -> String {
    if let Ok(mut parsed) = Url::parse(base) {
        {
            let mut pairs = parsed.query_pairs_mut();
            for (k, v) in params {
                pairs.append_pair(k, v);
            }
        }
        return parsed.into();
    }
    let serialized = serde_urlencoded::to_string(params).unwrap_or_default();
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{serialized}")
}

fn extract_csrf_cookie(req: &Request) -> Option<String> {
    for header_val in req.headers().get_all("cookie") {
        let Ok(raw) = header_val.to_str() else {
            continue;
        };
        for part in raw.split(';') {
            let mut kv = part.trim().splitn(2, '=');
            if kv.next() == Some("csrftoken") {
                if let Some(v) = kv.next() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

fn render_error_html(message: &str) -> String {
    let msg = escape_html(message);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>OAuth Error</title></head>\
         <body><h1>Authorization Error</h1><p>{msg}</p></body></html>"
    )
}

fn render_consent_html(
    validated: &ValidatedAuthorizeRequest,
    user: &User,
    csrf_token: Option<&str>,
) -> String {
    let app_name = escape_html(&validated.app.name);
    let username = escape_html(&user.username);
    let scopes_list: String = validated
        .scopes
        .iter()
        .map(|s| format!("<li>{}</li>", escape_html(s)))
        .collect::<Vec<_>>()
        .join("");

    let csrf_input = match csrf_token {
        Some(t) => format!(
            "<input type=\"hidden\" name=\"csrfmiddlewaretoken\" value=\"{}\">",
            escape_html(t)
        ),
        None => String::new(),
    };

    let state_input = match validated.state.as_deref() {
        Some(st) => format!(
            "<input type=\"hidden\" name=\"state\" value=\"{}\">",
            escape_html(st)
        ),
        None => String::new(),
    };
    let challenge_inputs = match validated.code_challenge.as_deref() {
        Some(cc) => format!(
            "<input type=\"hidden\" name=\"code_challenge\" value=\"{}\">\
             <input type=\"hidden\" name=\"code_challenge_method\" value=\"S256\">",
            escape_html(cc)
        ),
        None => String::new(),
    };
    let nonce_input = match validated.nonce.as_deref() {
        Some(n) => format!(
            "<input type=\"hidden\" name=\"nonce\" value=\"{}\">",
            escape_html(n)
        ),
        None => String::new(),
    };

    let scope_val = escape_html(&validated.scopes.join(" "));
    let client_id = escape_html(&validated.app.client_id);
    let redirect_uri = escape_html(&validated.redirect_uri);

    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <title>Authorize {app_name}</title></head>\
         <body>\
           <h1>Authorize {app_name}</h1>\
           <p>Signed in as <strong>{username}</strong>.</p>\
           <p><strong>{app_name}</strong> requests access to:</p>\
           <ul>{scopes_list}</ul>\
           <form method=\"post\" action=\"/o/authorize\">\
             {csrf_input}\
             <input type=\"hidden\" name=\"response_type\" value=\"code\">\
             <input type=\"hidden\" name=\"client_id\" value=\"{client_id}\">\
             <input type=\"hidden\" name=\"redirect_uri\" value=\"{redirect_uri}\">\
             <input type=\"hidden\" name=\"scope\" value=\"{scope_val}\">\
             {state_input}{challenge_inputs}{nonce_input}\
             <button type=\"submit\" name=\"decision\" value=\"approve\">Authorize</button>\
             <button type=\"submit\" name=\"decision\" value=\"deny\">Deny</button>\
           </form>\
         </body></html>"
    )
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
