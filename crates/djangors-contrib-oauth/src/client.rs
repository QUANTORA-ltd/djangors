//! OAuth 2.0 and OpenID Connect relying-party (client) implementation.
//!
//! Security properties:
//! - Per-flow `state`, PKCE S256 verifier, and OIDC `nonce` are stored in the
//!   server/cookie session and consumed once on callback (`subtle::ConstantTimeEq`).
//! - OIDC ID tokens are cryptographically verified via provider JWKS, checking
//!   `iss`, `aud`, `exp`, `nonce`, and (when present) `at_hash`.
//! - Email-based account linking strictly requires `email_verified == true`.
//! - Disconnecting a [`SocialAccount`] refuses if it would leave the user with
//!   neither a usable local password nor another linked [`SocialAccount`].

use crate::error::OAuthError;
use chrono::{DateTime, Utc};
use djangors_auth::{login, User, SESSION_USER_ID_KEY};
use djangors_core::{PathParams, Request, Response, Router, StatusCode};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_orm::{q, ForeignKey, Model as _};
use djangors_sessions::Session;
use hyper::Method;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId as OAuth2ClientId, ClientSecret as OAuth2ClientSecret, TokenResponse as _,
};
use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreIdTokenClaims, CoreProviderMetadata, CoreUserInfoClaims,
};
use openidconnect::{
    AccessToken, AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl,
    Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse as _,
};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;

const SESSION_PENDING_PREFIX: &str = "_oauth_pending_";
const DEFAULT_PENDING_TTL_SECS: i64 = 600;
const MICROSOFT_ISSUER_PREFIX: &str = "https://login.microsoftonline.com/";
const APPLE_ISSUER: &str = "https://appleid.apple.com";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";

/// Database record linking an external OAuth/OIDC account (`(provider, subject)`)
/// to a local [`User`].
#[derive(Model, Debug, Clone)]
#[djangors(
    app = "oauth",
    table_name = "oauth_social_account",
    unique_together = [["provider", "subject"]]
)]
pub struct SocialAccount {
    /// Auto-increment primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Provider slug (e.g. `"google"`, `"github"`, `"apple"`).
    #[djangors(max_length = 64)]
    pub provider: String,
    /// Provider-unique subject identifier (`sub` claim or GitHub numeric user ID).
    #[djangors(max_length = 255)]
    pub subject: String,
    /// Linked local user.
    pub user: ForeignKey<User>,
    /// Email address reported by the provider (empty string if none).
    #[djangors(max_length = 254)]
    pub email: String,
    /// Whether the provider attested that `email` is verified.
    pub email_verified: bool,
    /// Serialized raw profile/claim metadata JSON.
    pub extra_json: String,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Creates the `oauth_social_account` table using the dialect-appropriate SQL.
///
/// Call once at application startup or in migrations/tests (not on per-request hot paths).
pub async fn create_social_account_table(db: &Database) -> Result<(), djangors_orm::OrmError> {
    let d = db.dialect();
    let pk = d.auto_pk_type();
    let b = d.bool_type();
    let ts = d.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS oauth_social_account (
            id {pk},
            provider VARCHAR(64) NOT NULL,
            subject VARCHAR(255) NOT NULL,
            \"user\" BIGINT NOT NULL,
            email VARCHAR(254) NOT NULL,
            email_verified {b} NOT NULL,
            extra_json TEXT NOT NULL,
            created_at {ts} NOT NULL,
            UNIQUE (provider, subject)
        )"
    );
    db.conn().execute(&sql, &[]).await?;
    Ok(())
}

/// Registers [`SocialAccount`] with an [`djangors_admin::AdminSite`].
#[cfg(feature = "admin")]
pub fn register_client_admin(site: &djangors_admin::AdminSite) {
    site.register::<SocialAccount>();
}

/// Verified user identity returned by an [`OAuthProvider`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuthIdentity {
    /// Registered provider name (e.g. `"google"`).
    pub provider: String,
    /// Stable unique subject identifier at that provider.
    pub subject: String,
    /// Verified or unverified email address, if supplied.
    pub email: Option<String>,
    /// True only when the provider verified ownership of `email`.
    pub email_verified: bool,
    /// Optional preferred username / login handle hint.
    pub username_hint: Option<String>,
    /// Optional display name.
    pub name: Option<String>,
    /// Raw claims/profile JSON from the provider.
    pub extra: serde_json::Value,
}

/// Trait implemented by OAuth 2.0 and OIDC social login providers.
///
/// Custom providers implement this trait and register via
/// [`OAuthRegistry::register`].
#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Short lowercase route identifier used in `/auth/{provider}/...` (e.g. `"google"`).
    fn name(&self) -> &str;

    /// Builds the upstream authorization redirect URL using caller-supplied
    /// `state`, S256 `pkce_challenge`, and optional OIDC `nonce`.
    async fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        pkce_challenge: &str,
        nonce: Option<&str>,
    ) -> Result<String, OAuthError>;

    /// Exchanges an authorization `code` for tokens, validates OIDC signatures and
    /// claims (`iss`, `aud`, `exp`, `nonce`) if applicable, and returns [`OAuthIdentity`].
    async fn exchange_and_fetch_identity(
        &self,
        redirect_uri: &str,
        code: &str,
        pkce_verifier: Option<&str>,
        expected_nonce: Option<&str>,
    ) -> Result<OAuthIdentity, OAuthError>;
}

/// Builds a shared `reqwest::Client` using `rustls` (`ring` provider) with
/// automatic HTTP redirect following disabled (SSRF mitigation).
pub fn default_http_client() -> Result<reqwest::Client, OAuthError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| OAuthError::Http(format!("failed to construct reqwest client: {e}")))
}

fn validate_issuer_url(raw: &str, allow_http: bool) -> Result<IssuerUrl, OAuthError> {
    let trimmed = raw.trim();
    if !allow_http && !trimmed.starts_with("https://") {
        return Err(OAuthError::InvalidConfig(format!(
            "OIDC issuer URL must use https:// (got '{trimmed}')"
        )));
    }
    IssuerUrl::new(trimmed.to_string())
        .map_err(|e| OAuthError::InvalidConfig(format!("invalid issuer URL '{trimmed}': {e}")))
}

fn validate_redirect_uri(raw: &str) -> Result<RedirectUrl, OAuthError> {
    RedirectUrl::new(raw.to_string())
        .map_err(|e| OAuthError::InvalidConfig(format!("invalid redirect_uri '{raw}': {e}")))
}

/// Configuration for a generic OpenID Connect provider with discovery.
#[derive(Clone)]
pub struct OidcConfig {
    /// Provider slug (e.g. `"okta"`, `"google"`).
    pub name: String,
    /// OIDC issuer URL (used for `/.well-known/openid-configuration` discovery).
    pub issuer_url: String,
    /// Client identifier.
    pub client_id: String,
    /// Optional confidential client secret (redacted in `Debug`).
    pub client_secret: Option<SecretString>,
    /// Scopes requested in addition to `"openid"` (`"openid"` is always ensured).
    pub scopes: Vec<String>,
    /// Additional query parameters appended to the authorization URL.
    pub extra_auth_params: Vec<(String, String)>,
    /// Use `response_mode=form_post` on authorization requests (Apple Sign In).
    pub use_form_post: bool,
    /// Fetch `/userinfo` when email is absent from ID token claims.
    pub fetch_userinfo_if_missing_email: bool,
    /// Allow `http://` issuer/endpoints (tests only; defaults to `false`).
    pub allow_insecure_http: bool,
    /// Optional custom `reqwest::Client`.
    pub http_client: Option<reqwest::Client>,
}

impl fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcConfig")
            .field("name", &self.name)
            .field("issuer_url", &self.issuer_url)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scopes", &self.scopes)
            .field("use_form_post", &self.use_form_post)
            .field("allow_insecure_http", &self.allow_insecure_http)
            .finish()
    }
}

impl OidcConfig {
    /// Creates a new generic OIDC provider configuration.
    pub fn new(
        name: impl Into<String>,
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            issuer_url: issuer_url.into(),
            client_id: client_id.into(),
            client_secret: client_secret.map(SecretString::from),
            scopes: vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ],
            extra_auth_params: Vec::new(),
            use_form_post: false,
            fetch_userinfo_if_missing_email: true,
            allow_insecure_http: false,
            http_client: None,
        }
    }

    /// Replaces the requested scope list (`"openid"` is always added automatically).
    pub fn with_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// Appends an extra query parameter to authorization requests (e.g. `("hd", "example.com")`).
    pub fn with_extra_auth_param(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.extra_auth_params.push((key.into(), value.into()));
        self
    }

    /// Allows plain `http://` issuer URLs (intended for local test servers only).
    pub fn with_allow_insecure_http(mut self, allow: bool) -> Self {
        self.allow_insecure_http = allow;
        self
    }
}

#[derive(Clone, Debug)]
enum IssuerPolicy {
    Exact,
    MicrosoftMultiTenant,
}

#[async_trait::async_trait]
trait DynamicClientSecretSource: Send + Sync {
    async fn client_secret(&self) -> Result<Option<ClientSecret>, OAuthError>;
}

struct StaticSecretSource(Option<SecretString>);

#[async_trait::async_trait]
impl DynamicClientSecretSource for StaticSecretSource {
    async fn client_secret(&self) -> Result<Option<ClientSecret>, OAuthError> {
        Ok(self
            .0
            .as_ref()
            .map(|s| ClientSecret::new(s.expose_secret().to_string())))
    }
}

/// Generic OpenID Connect provider supporting `/.well-known/openid-configuration`
/// discovery, PKCE S256, `state`, `nonce`, and JWKS ID token validation.
pub struct OidcProvider {
    config: OidcConfig,
    http_client: reqwest::Client,
    cached_metadata: RwLock<Option<CoreProviderMetadata>>,
    issuer_policy: IssuerPolicy,
    secret_source: Arc<dyn DynamicClientSecretSource>,
}

impl OidcProvider {
    /// Creates an [`OidcProvider`] that discovers provider metadata lazily and caches it.
    pub fn new(config: OidcConfig) -> Result<Self, OAuthError> {
        let http_client = match &config.http_client {
            Some(c) => c.clone(),
            None => default_http_client()?,
        };
        validate_issuer_url(&config.issuer_url, config.allow_insecure_http)?;
        let secret_source = Arc::new(StaticSecretSource(config.client_secret.clone()));
        Ok(Self {
            config,
            http_client,
            cached_metadata: RwLock::new(None),
            issuer_policy: IssuerPolicy::Exact,
            secret_source,
        })
    }

    /// Creates an [`OidcProvider`] pre-populated with known [`CoreProviderMetadata`].
    pub fn from_metadata(
        config: OidcConfig,
        metadata: CoreProviderMetadata,
    ) -> Result<Self, OAuthError> {
        let provider = Self::new(config)?;
        Ok(Self {
            cached_metadata: RwLock::new(Some(metadata)),
            ..provider
        })
    }

    async fn metadata(&self) -> Result<CoreProviderMetadata, OAuthError> {
        if let Some(cached) = self.cached_metadata.read().await.clone() {
            return Ok(cached);
        }
        let issuer = validate_issuer_url(&self.config.issuer_url, self.config.allow_insecure_http)?;
        let discovered = CoreProviderMetadata::discover_async(issuer, &self.http_client)
            .await
            .map_err(|e| OAuthError::InvalidConfig(format!("OIDC discovery failed: {e}")))?;
        let mut guard = self.cached_metadata.write().await;
        *guard = Some(discovered.clone());
        Ok(discovered)
    }

    fn verify_microsoft_multi_tenant_issuer(iss: &str) -> Result<(), OAuthError> {
        let Some(rest) = iss.strip_prefix(MICROSOFT_ISSUER_PREFIX) else {
            return Err(OAuthError::InvalidIdToken(format!(
                "unexpected Microsoft multi-tenant issuer '{iss}'"
            )));
        };
        let Some(tenant_id) = rest.strip_suffix("/v2.0") else {
            return Err(OAuthError::InvalidIdToken(format!(
                "unexpected Microsoft issuer path '{iss}'"
            )));
        };
        if tenant_id.is_empty() || tenant_id.contains('/') || tenant_id.contains('{') {
            return Err(OAuthError::InvalidIdToken(format!(
                "invalid tenant segment in Microsoft issuer '{iss}'"
            )));
        }
        Ok(())
    }
}

fn pkce_challenge_from_verifier(verifier: &str) -> PkceCodeChallenge {
    PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(verifier.to_string()))
}

#[async_trait::async_trait]
impl OAuthProvider for OidcProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        pkce_challenge: &str,
        nonce: Option<&str>,
    ) -> Result<String, OAuthError> {
        let metadata = self.metadata().await?;
        let redirect = validate_redirect_uri(redirect_uri)?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(self.config.client_id.clone()),
            None,
        )
        .set_redirect_uri(redirect);

        let state_owned = state.to_string();
        let nonce_owned = nonce.unwrap_or_default().to_string();
        let mut req = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || CsrfToken::new(state_owned.clone()),
            move || Nonce::new(nonce_owned.clone()),
        );

        let mut has_openid = false;
        for scope in &self.config.scopes {
            if scope == "openid" {
                has_openid = true;
            }
            req = req.add_scope(Scope::new(scope.clone()));
        }
        if !has_openid {
            req = req.add_scope(Scope::new("openid".to_string()));
        }

        // Caller passes either the already-computed challenge string (when custom) or
        // verifier-derived challenge; rebuild challenge cleanly from its value if S256.
        // Note: `oauth_login_view` calls `authorization_url` with a precomputed S256 challenge
        // computed from `pkce_challenge_from_verifier(&verifier)`.
        let pkce_val: oauth2::PkceCodeChallenge = serde_json::from_value(serde_json::json!({
            "code_challenge": pkce_challenge,
            "code_challenge_method": "S256",
        }))
        .map_err(|e| OAuthError::InvalidConfig(format!("invalid PKCE challenge: {e}")))?;
        req = req.set_pkce_challenge(pkce_val);

        if self.config.use_form_post {
            req = req.add_extra_param("response_mode", "form_post");
        }
        for (k, v) in &self.config.extra_auth_params {
            req = req.add_extra_param(k, v);
        }

        let (url, _, _) = req.url();
        Ok(url.to_string())
    }

    async fn exchange_and_fetch_identity(
        &self,
        redirect_uri: &str,
        code: &str,
        pkce_verifier: Option<&str>,
        expected_nonce: Option<&str>,
    ) -> Result<OAuthIdentity, OAuthError> {
        let expected_nonce_str = expected_nonce.ok_or(OAuthError::MissingNonce)?;
        let pkce_verifier_str = pkce_verifier.ok_or(OAuthError::MissingPkceVerifier)?;
        let metadata = self.metadata().await?;
        let redirect = validate_redirect_uri(redirect_uri)?;
        let client_secret = self.secret_source.client_secret().await?;

        let client = CoreClient::from_provider_metadata(
            metadata.clone(),
            ClientId::new(self.config.client_id.clone()),
            client_secret,
        )
        .set_redirect_uri(redirect);

        let token_request = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| OAuthError::TokenExchange(format!("token endpoint missing: {e}")))?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier_str.to_string()));

        let token_response = token_request
            .request_async(&self.http_client)
            .await
            .map_err(|e| OAuthError::TokenExchange(e.to_string()))?;

        let id_token = token_response.id_token().ok_or_else(|| {
            OAuthError::InvalidIdToken("missing id_token in response".to_string())
        })?;

        let mut verifier = client.id_token_verifier();
        if matches!(self.issuer_policy, IssuerPolicy::MicrosoftMultiTenant) {
            verifier = verifier.require_issuer_match(false);
        }

        let nonce = Nonce::new(expected_nonce_str.to_string());
        let claims: &CoreIdTokenClaims = id_token.claims(&verifier, &nonce).map_err(|e| {
            let msg = e.to_string();
            if msg.to_ascii_lowercase().contains("nonce") {
                OAuthError::NonceMismatch
            } else {
                OAuthError::InvalidIdToken(msg)
            }
        })?;

        if matches!(self.issuer_policy, IssuerPolicy::MicrosoftMultiTenant) {
            Self::verify_microsoft_multi_tenant_issuer(claims.issuer().as_str())?;
        }

        // Verify RFC 6749 / OIDC Core `at_hash` when provided in ID token claims.
        if let Some(expected_at_hash) = claims.access_token_hash() {
            let alg = id_token
                .signing_alg()
                .map_err(|e| OAuthError::InvalidIdToken(format!("missing signing alg: {e}")))?;
            let signing_key = id_token
                .signing_key(&verifier)
                .map_err(|e| OAuthError::InvalidIdToken(format!("signing key error: {e}")))?;
            let actual_at_hash =
                AccessTokenHash::from_token(token_response.access_token(), alg, signing_key)
                    .map_err(|e| OAuthError::InvalidIdToken(format!("at_hash error: {e}")))?;
            if actual_at_hash != *expected_at_hash {
                return Err(OAuthError::InvalidIdToken(
                    "access_token hash (at_hash) mismatch".to_string(),
                ));
            }
        }

        let subject = claims.subject().as_str().to_string();
        let mut email = claims.email().map(|e| e.as_str().to_string());
        let mut email_verified = claims.email_verified().unwrap_or(false);
        let mut name = claims
            .name()
            .and_then(|n| n.get(None))
            .map(|s| s.as_str().to_string());
        let mut username_hint = claims.preferred_username().map(|u| u.as_str().to_string());

        if email.is_none()
            && self.config.fetch_userinfo_if_missing_email
            && metadata.userinfo_endpoint().is_some()
        {
            if let Ok(req) = client.user_info(
                AccessToken::new(token_response.access_token().secret().clone()),
                Some(claims.subject().clone()),
            ) {
                if let Ok(userinfo) = req.request_async(&self.http_client).await {
                    let ui: CoreUserInfoClaims = userinfo;
                    if let Some(u_email) = ui.email() {
                        email = Some(u_email.as_str().to_string());
                        email_verified = ui.email_verified().unwrap_or(false);
                    }
                    if name.is_none() {
                        name = ui
                            .name()
                            .and_then(|n| n.get(None))
                            .map(|s| s.as_str().to_string());
                    }
                    if username_hint.is_none() {
                        username_hint = ui.preferred_username().map(|u| u.as_str().to_string());
                    }
                }
            }
        }

        let extra = serde_json::to_value(claims).unwrap_or(serde_json::Value::Null);

        Ok(OAuthIdentity {
            provider: self.config.name.clone(),
            subject,
            email,
            email_verified,
            username_hint,
            name,
            extra,
        })
    }
}

/// Google Sign-In OIDC provider (`https://accounts.google.com`).
pub struct GoogleProvider {
    inner: OidcProvider,
}

impl GoogleProvider {
    /// Creates a Google OIDC provider for `client_id` / `client_secret`.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Result<Self, OAuthError> {
        let cfg = OidcConfig::new(
            "google",
            GOOGLE_ISSUER,
            client_id,
            Some(client_secret.into()),
        );
        Ok(Self {
            inner: OidcProvider::new(cfg)?,
        })
    }

    /// Creates a [`GoogleProvider`] from a customized [`OidcConfig`].
    pub fn with_config(config: OidcConfig) -> Result<Self, OAuthError> {
        Ok(Self {
            inner: OidcProvider::new(config)?,
        })
    }
}

#[async_trait::async_trait]
impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        pkce_challenge: &str,
        nonce: Option<&str>,
    ) -> Result<String, OAuthError> {
        self.inner
            .authorization_url(redirect_uri, state, pkce_challenge, nonce)
            .await
    }

    async fn exchange_and_fetch_identity(
        &self,
        redirect_uri: &str,
        code: &str,
        pkce_verifier: Option<&str>,
        expected_nonce: Option<&str>,
    ) -> Result<OAuthIdentity, OAuthError> {
        self.inner
            .exchange_and_fetch_identity(redirect_uri, code, pkce_verifier, expected_nonce)
            .await
    }
}

/// Microsoft Entra ID tenant mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicrosoftTenant {
    /// Multi-tenant personal + work/school (`common`).
    Common,
    /// Multi-tenant work/school (`organizations`).
    Organizations,
    /// Personal Microsoft accounts (`consumers`).
    Consumers,
    /// Single Azure AD / Entra ID tenant GUID or domain.
    Tenant(String),
}

impl MicrosoftTenant {
    fn segment(&self) -> &str {
        match self {
            MicrosoftTenant::Common => "common",
            MicrosoftTenant::Organizations => "organizations",
            MicrosoftTenant::Consumers => "consumers",
            MicrosoftTenant::Tenant(id) => id.trim(),
        }
    }

    fn is_multi_tenant(&self) -> bool {
        !matches!(self, MicrosoftTenant::Tenant(_))
    }
}

/// Microsoft Entra ID (Azure AD) v2.0 OIDC provider supporting `common` and single tenants.
pub struct MicrosoftProvider {
    inner: OidcProvider,
}

impl MicrosoftProvider {
    /// Creates a Microsoft OIDC provider targeting `tenant`.
    pub fn new(
        tenant: MicrosoftTenant,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Result<Self, OAuthError> {
        let seg = tenant.segment();
        if seg.is_empty() || seg.contains('/') {
            return Err(OAuthError::InvalidConfig(
                "Microsoft tenant must be a non-empty identifier".to_string(),
            ));
        }
        let issuer = format!("{MICROSOFT_ISSUER_PREFIX}{seg}/v2.0");
        let cfg = OidcConfig::new("microsoft", issuer, client_id, Some(client_secret.into()))
            .with_scopes(["openid", "profile", "email"]);
        let mut inner = OidcProvider::new(cfg)?;
        if tenant.is_multi_tenant() {
            inner.issuer_policy = IssuerPolicy::MicrosoftMultiTenant;
        }
        Ok(Self { inner })
    }

    /// Creates a multi-tenant [`MicrosoftProvider`] pre-loaded with [`CoreProviderMetadata`]
    /// (useful for tests or air-gapped environments).
    pub fn multi_tenant_from_metadata(
        config: OidcConfig,
        metadata: CoreProviderMetadata,
    ) -> Result<Self, OAuthError> {
        let mut inner = OidcProvider::from_metadata(config, metadata)?;
        inner.issuer_policy = IssuerPolicy::MicrosoftMultiTenant;
        Ok(Self { inner })
    }
}

#[async_trait::async_trait]
impl OAuthProvider for MicrosoftProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        pkce_challenge: &str,
        nonce: Option<&str>,
    ) -> Result<String, OAuthError> {
        self.inner
            .authorization_url(redirect_uri, state, pkce_challenge, nonce)
            .await
    }

    async fn exchange_and_fetch_identity(
        &self,
        redirect_uri: &str,
        code: &str,
        pkce_verifier: Option<&str>,
        expected_nonce: Option<&str>,
    ) -> Result<OAuthIdentity, OAuthError> {
        self.inner
            .exchange_and_fetch_identity(redirect_uri, code, pkce_verifier, expected_nonce)
            .await
    }
}

/// Configuration for Apple Sign In (OIDC with `response_mode=form_post` and ES256 client-secret JWT).
#[derive(Clone)]
pub struct AppleConfig {
    /// Apple Services ID (`client_id`, used as `sub` in client-secret JWT).
    pub client_id: String,
    /// Apple Developer 10-character Team ID (`iss` claim in client-secret JWT).
    pub team_id: String,
    /// Key ID matching the ES256 private key (`kid` header in client-secret JWT).
    pub key_id: String,
    /// PKCS#8 PEM-encoded P-256 private key (`-----BEGIN PRIVATE KEY-----...`).
    pub private_key_pem: SecretString,
    /// Client-secret JWT validity in seconds (capped at 15_777_000 per Apple spec; defaults to 300s).
    pub jwt_ttl_secs: u64,
    /// Override issuer URL for testing (defaults to `"https://appleid.apple.com"`).
    pub issuer_url: String,
    /// Allow `http://` issuer URLs in tests (defaults to `false`).
    pub allow_insecure_http: bool,
}

impl fmt::Debug for AppleConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppleConfig")
            .field("client_id", &self.client_id)
            .field("team_id", &self.team_id)
            .field("key_id", &self.key_id)
            .field("private_key_pem", &"[REDACTED]")
            .field("jwt_ttl_secs", &self.jwt_ttl_secs)
            .field("issuer_url", &self.issuer_url)
            .finish()
    }
}

impl AppleConfig {
    /// Constructs a new [`AppleConfig`] targeting `https://appleid.apple.com`.
    pub fn new(
        client_id: impl Into<String>,
        team_id: impl Into<String>,
        key_id: impl Into<String>,
        private_key_pem: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            team_id: team_id.into(),
            key_id: key_id.into(),
            private_key_pem: SecretString::from(private_key_pem.into()),
            jwt_ttl_secs: 300,
            issuer_url: APPLE_ISSUER.to_string(),
            allow_insecure_http: false,
        }
    }
}

#[derive(Serialize)]
struct AppleClientSecretClaims<'a> {
    iss: &'a str,
    iat: u64,
    exp: u64,
    aud: &'static str,
    sub: &'a str,
}

struct AppleSecretSource {
    config: AppleConfig,
}

impl AppleSecretSource {
    fn generate_client_secret_jwt(&self) -> Result<String, OAuthError> {
        let now = Utc::now().timestamp().max(0) as u64;
        // Apple rejects client secrets whose lifetime exceeds 6 months (15,777,000s).
        let ttl = self.config.jwt_ttl_secs.clamp(60, 15_777_000);
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());
        let claims = AppleClientSecretClaims {
            iss: &self.config.team_id,
            iat: now,
            exp: now + ttl,
            aud: APPLE_ISSUER,
            sub: &self.config.client_id,
        };
        let key = EncodingKey::from_ec_pem(self.config.private_key_pem.expose_secret().as_bytes())
            .map_err(|e| {
                OAuthError::InvalidConfig(format!("invalid Apple ES256 private key: {e}"))
            })?;
        jsonwebtoken::encode(&header, &claims, &key).map_err(|e| {
            OAuthError::InvalidConfig(format!("failed to sign Apple client secret: {e}"))
        })
    }
}

#[async_trait::async_trait]
impl DynamicClientSecretSource for AppleSecretSource {
    async fn client_secret(&self) -> Result<Option<ClientSecret>, OAuthError> {
        let jwt = self.generate_client_secret_jwt()?;
        Ok(Some(ClientSecret::new(jwt)))
    }
}

/// Apple Sign In OIDC provider (`response_mode=form_post`, dynamic ES256 client-secret JWT).
pub struct AppleProvider {
    inner: OidcProvider,
    secret_source: Arc<AppleSecretSource>,
}

impl AppleProvider {
    /// Creates an [`AppleProvider`] from [`AppleConfig`].
    pub fn new(config: AppleConfig) -> Result<Self, OAuthError> {
        let source = Arc::new(AppleSecretSource {
            config: config.clone(),
        });
        // Validate ES256 key eagerly so misconfiguration fails fast.
        let _ = source.generate_client_secret_jwt()?;

        let mut oidc_cfg = OidcConfig::new("apple", &config.issuer_url, &config.client_id, None)
            .with_scopes(["openid", "name", "email"])
            .with_allow_insecure_http(config.allow_insecure_http);
        oidc_cfg.use_form_post = true;
        oidc_cfg.fetch_userinfo_if_missing_email = false;

        let mut inner = OidcProvider::new(oidc_cfg)?;
        inner.secret_source = source.clone();

        Ok(Self {
            inner,
            secret_source: source,
        })
    }

    /// Generates an ES256 client-secret JWT matching Apple's client authentication spec.
    pub fn generate_client_secret_jwt(&self) -> Result<String, OAuthError> {
        self.secret_source.generate_client_secret_jwt()
    }
}

#[async_trait::async_trait]
impl OAuthProvider for AppleProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        pkce_challenge: &str,
        nonce: Option<&str>,
    ) -> Result<String, OAuthError> {
        self.inner
            .authorization_url(redirect_uri, state, pkce_challenge, nonce)
            .await
    }

    async fn exchange_and_fetch_identity(
        &self,
        redirect_uri: &str,
        code: &str,
        pkce_verifier: Option<&str>,
        expected_nonce: Option<&str>,
    ) -> Result<OAuthIdentity, OAuthError> {
        self.inner
            .exchange_and_fetch_identity(redirect_uri, code, pkce_verifier, expected_nonce)
            .await
    }
}

/// Configuration for the plain OAuth 2.0 GitHub provider (`/user` + `/user/emails`).
#[derive(Clone)]
pub struct GitHubConfig {
    /// GitHub OAuth App / GitHub App client ID.
    pub client_id: String,
    /// GitHub client secret (redacted in `Debug`).
    pub client_secret: SecretString,
    /// Authorization URL (default `"https://github.com/login/oauth/authorize"`).
    pub auth_url: String,
    /// Token URL (default `"https://github.com/login/oauth/access_token"`).
    pub token_url: String,
    /// GitHub REST API base URL (default `"https://api.github.com"`).
    pub api_base_url: String,
    /// Requested OAuth scopes (default `["read:user", "user:email"]`).
    pub scopes: Vec<String>,
    /// Allow `http://` URLs (tests only; defaults to `false`).
    pub allow_insecure_http: bool,
}

impl fmt::Debug for GitHubConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("auth_url", &self.auth_url)
            .field("token_url", &self.token_url)
            .field("api_base_url", &self.api_base_url)
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl GitHubConfig {
    /// Creates a new [`GitHubConfig`] pointing at `github.com`.
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: SecretString::from(client_secret.into()),
            auth_url: "https://github.com/login/oauth/authorize".to_string(),
            token_url: "https://github.com/login/oauth/access_token".to_string(),
            api_base_url: "https://api.github.com".to_string(),
            scopes: vec!["read:user".to_string(), "user:email".to_string()],
            allow_insecure_http: false,
        }
    }

    /// Overrides the GitHub endpoints for Enterprise Server or local mock tests.
    pub fn with_endpoints(
        mut self,
        auth_url: impl Into<String>,
        token_url: impl Into<String>,
        api_base_url: impl Into<String>,
    ) -> Self {
        self.auth_url = auth_url.into();
        self.token_url = token_url.into();
        self.api_base_url = api_base_url.into();
        self
    }

    /// Allows plain `http://` URLs (local mock tests only).
    pub fn with_allow_insecure_http(mut self, allow: bool) -> Self {
        self.allow_insecure_http = allow;
        self
    }
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    id: i64,
    login: String,
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubEmailEntry {
    pub(crate) email: String,
    pub(crate) primary: bool,
    pub(crate) verified: bool,
}

pub(crate) fn select_verified_github_email(emails: &[GitHubEmailEntry]) -> (Option<String>, bool) {
    // Prefer the primary email ONLY if it is verified.
    if let Some(primary_verified) = emails.iter().find(|e| e.primary && e.verified) {
        return (Some(primary_verified.email.clone()), true);
    }
    // Otherwise fall back to the first verified email.
    if let Some(any_verified) = emails.iter().find(|e| e.verified) {
        return (Some(any_verified.email.clone()), true);
    }
    // Never accept unverified email addresses for account linking.
    (None, false)
}

/// GitHub OAuth 2.0 provider using `/user` and `/user/emails` (verified email only).
pub struct GitHubProvider {
    config: GitHubConfig,
    http_client: reqwest::Client,
}

impl GitHubProvider {
    /// Creates a [`GitHubProvider`] with default `rustls` `reqwest` client.
    pub fn new(config: GitHubConfig) -> Result<Self, OAuthError> {
        if !config.allow_insecure_http
            && (!config.auth_url.starts_with("https://")
                || !config.token_url.starts_with("https://")
                || !config.api_base_url.starts_with("https://"))
        {
            return Err(OAuthError::InvalidConfig(
                "GitHub OAuth endpoints must use https://".to_string(),
            ));
        }
        Ok(Self {
            config,
            http_client: default_http_client()?,
        })
    }
}

#[async_trait::async_trait]
impl OAuthProvider for GitHubProvider {
    fn name(&self) -> &str {
        "github"
    }

    async fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        pkce_challenge: &str,
        _nonce: Option<&str>,
    ) -> Result<String, OAuthError> {
        let auth_url = AuthUrl::new(self.config.auth_url.clone())
            .map_err(|e| OAuthError::InvalidConfig(format!("invalid GitHub auth URL: {e}")))?;
        let token_url = oauth2::TokenUrl::new(self.config.token_url.clone())
            .map_err(|e| OAuthError::InvalidConfig(format!("invalid GitHub token URL: {e}")))?;
        let redirect = oauth2::RedirectUrl::new(redirect_uri.to_string())
            .map_err(|e| OAuthError::InvalidConfig(format!("invalid redirect_uri: {e}")))?;

        let client = BasicClient::new(OAuth2ClientId::new(self.config.client_id.clone()))
            .set_client_secret(OAuth2ClientSecret::new(
                self.config.client_secret.expose_secret().to_string(),
            ))
            .set_auth_uri(auth_url)
            .set_token_uri(token_url)
            .set_redirect_uri(redirect);

        let state_owned = state.to_string();
        let mut req = client.authorize_url(move || oauth2::CsrfToken::new(state_owned.clone()));
        for scope in &self.config.scopes {
            req = req.add_scope(oauth2::Scope::new(scope.clone()));
        }
        let pkce_val: oauth2::PkceCodeChallenge = serde_json::from_value(serde_json::json!({
            "code_challenge": pkce_challenge,
            "code_challenge_method": "S256",
        }))
        .map_err(|e| OAuthError::InvalidConfig(format!("invalid PKCE challenge: {e}")))?;
        req = req.set_pkce_challenge(pkce_val);
        let (url, _) = req.url();
        Ok(url.to_string())
    }

    async fn exchange_and_fetch_identity(
        &self,
        redirect_uri: &str,
        code: &str,
        pkce_verifier: Option<&str>,
        _expected_nonce: Option<&str>,
    ) -> Result<OAuthIdentity, OAuthError> {
        let auth_url = AuthUrl::new(self.config.auth_url.clone())
            .map_err(|e| OAuthError::InvalidConfig(format!("invalid GitHub auth URL: {e}")))?;
        let token_url = oauth2::TokenUrl::new(self.config.token_url.clone())
            .map_err(|e| OAuthError::InvalidConfig(format!("invalid GitHub token URL: {e}")))?;
        let redirect = oauth2::RedirectUrl::new(redirect_uri.to_string())
            .map_err(|e| OAuthError::InvalidConfig(format!("invalid redirect_uri: {e}")))?;

        let client = BasicClient::new(OAuth2ClientId::new(self.config.client_id.clone()))
            .set_client_secret(OAuth2ClientSecret::new(
                self.config.client_secret.expose_secret().to_string(),
            ))
            .set_auth_uri(auth_url)
            .set_token_uri(token_url)
            .set_redirect_uri(redirect);

        let mut token_req = client.exchange_code(oauth2::AuthorizationCode::new(code.to_string()));
        if let Some(verifier) = pkce_verifier {
            token_req =
                token_req.set_pkce_verifier(oauth2::PkceCodeVerifier::new(verifier.to_string()));
        }

        let token_res = token_req
            .request_async(&self.http_client)
            .await
            .map_err(|e| OAuthError::TokenExchange(e.to_string()))?;

        let access_token = token_res.access_token().secret();
        let api_base = self.config.api_base_url.trim_end_matches('/');

        let user_resp = self
            .http_client
            .get(format!("{api_base}/user"))
            .bearer_auth(access_token)
            .header("User-Agent", "djangors-contrib-oauth")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| OAuthError::UserInfo(e.to_string()))?;
        if !user_resp.status().is_success() {
            return Err(OAuthError::UserInfo(format!(
                "GitHub /user returned status {}",
                user_resp.status()
            )));
        }
        let gh_user: GitHubUser = user_resp
            .json()
            .await
            .map_err(|e| OAuthError::UserInfo(format!("invalid GitHub /user JSON: {e}")))?;

        let emails_resp = self
            .http_client
            .get(format!("{api_base}/user/emails"))
            .bearer_auth(access_token)
            .header("User-Agent", "djangors-contrib-oauth")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| OAuthError::UserInfo(e.to_string()))?;

        let emails: Vec<GitHubEmailEntry> = if emails_resp.status().is_success() {
            emails_resp.json().await.map_err(|e| {
                OAuthError::UserInfo(format!("invalid GitHub /user/emails JSON: {e}"))
            })?
        } else {
            return Err(OAuthError::UserInfo(format!(
                "GitHub /user/emails returned status {}",
                emails_resp.status()
            )));
        };

        let (email, email_verified) = select_verified_github_email(&emails);

        Ok(OAuthIdentity {
            provider: "github".to_string(),
            subject: gh_user.id.to_string(),
            email,
            email_verified,
            username_hint: Some(gh_user.login.clone()),
            name: gh_user.name.clone(),
            extra: serde_json::json!({
                "id": gh_user.id,
                "login": gh_user.login,
                "name": gh_user.name,
            }),
        })
    }
}

/// Application-wide registry of social login providers and redirect configuration.
#[derive(Clone)]
pub struct OAuthRegistry {
    providers: HashMap<String, Arc<dyn OAuthProvider>>,
    /// Base origin + prefix used to compute `/auth/{provider}/callback` redirect URIs.
    pub base_url: String,
    /// Where to redirect after successful login (default `"/"`).
    pub post_login_redirect: String,
    /// Where to redirect after connecting/disconnecting an account (default `"/"`).
    pub post_connect_redirect: String,
    /// Expiry in seconds for session-stored `state` / PKCE / nonce tuples (default 600s).
    pub pending_ttl_secs: i64,
}

impl fmt::Debug for OAuthRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthRegistry")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .field("base_url", &self.base_url)
            .field("post_login_redirect", &self.post_login_redirect)
            .field("post_connect_redirect", &self.post_connect_redirect)
            .field("pending_ttl_secs", &self.pending_ttl_secs)
            .finish()
    }
}

impl OAuthRegistry {
    /// Creates an empty [`OAuthRegistry`] rooted at `base_url` (e.g. `"https://app.example.com"`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            providers: HashMap::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            post_login_redirect: "/".to_string(),
            post_connect_redirect: "/".to_string(),
            pending_ttl_secs: DEFAULT_PENDING_TTL_SECS,
        }
    }

    /// Registers a provider under its [`OAuthProvider::name`].
    pub fn register<P: OAuthProvider + 'static>(mut self, provider: P) -> Self {
        self.providers
            .insert(provider.name().to_string(), Arc::new(provider));
        self
    }

    /// Sets the URL redirected to after login succeeds.
    pub fn with_post_login_redirect(mut self, url: impl Into<String>) -> Self {
        self.post_login_redirect = url.into();
        self
    }

    /// Sets the URL redirected to after connect/disconnect succeeds.
    pub fn with_post_connect_redirect(mut self, url: impl Into<String>) -> Self {
        self.post_connect_redirect = url.into();
        self
    }

    /// Looks up a registered provider by slug.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn OAuthProvider>> {
        self.providers.get(name)
    }

    /// Computes the full callback redirect URI for `provider`.
    pub fn callback_url(&self, provider: &str) -> String {
        format!("{}/auth/{provider}/callback", self.base_url)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuthFlow {
    provider: String,
    state: String,
    pkce_verifier: String,
    nonce: String,
    connect_user_id: Option<i64>,
    expires_at_ts: i64,
}

fn random_urlsafe_token() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pending_session_key(state: &str) -> String {
    format!("{SESSION_PENDING_PREFIX}{state}")
}

async fn start_flow(
    req: &Request,
    provider_name: &str,
    connect_user_id: Option<i64>,
) -> Result<Response, OAuthError> {
    let registry = req
        .state::<OAuthRegistry>()
        .ok_or_else(|| OAuthError::InvalidConfig("OAuthRegistry not in AppState".to_string()))?;
    let provider = registry
        .get(provider_name)
        .ok_or_else(|| OAuthError::UnknownProvider(provider_name.to_string()))?;
    let session = req.ext::<Session>().ok_or(OAuthError::Unauthorized)?;

    let state = random_urlsafe_token();
    let pkce_verifier = random_urlsafe_token();
    let nonce = random_urlsafe_token();
    let pkce_challenge = pkce_challenge_from_verifier(&pkce_verifier)
        .as_str()
        .to_string();

    let pending = PendingOAuthFlow {
        provider: provider_name.to_string(),
        state: state.clone(),
        pkce_verifier,
        nonce: nonce.clone(),
        connect_user_id,
        expires_at_ts: Utc::now().timestamp() + registry.pending_ttl_secs,
    };
    session.set(&pending_session_key(&state), &pending);

    let redirect_uri = registry.callback_url(provider_name);
    let url = provider
        .authorization_url(&redirect_uri, &state, &pkce_challenge, Some(&nonce))
        .await?;
    Ok(Response::redirect(&url))
}

#[derive(Debug, Default, Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    user: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppleFormUser {
    name: Option<AppleFormUserName>,
}

#[derive(Debug, Deserialize)]
struct AppleFormUserName {
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
}

fn parse_apple_form_name(user_json: Option<&str>) -> Option<String> {
    let raw = user_json?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed: AppleFormUser = serde_json::from_str(raw).ok()?;
    let n = parsed.name?;
    let full = [n.first_name, n.last_name]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if full.is_empty() {
        None
    } else {
        Some(full)
    }
}

async fn extract_callback_params(req: &Request) -> CallbackParams {
    if *req.method() == Method::POST {
        let body = req.body_bytes().await;
        if let Ok(parsed) = serde_urlencoded::from_bytes::<CallbackParams>(body) {
            if parsed.code.is_some() || parsed.state.is_some() || parsed.error.is_some() {
                return parsed;
            }
        }
    }
    if let Some(raw_q) = req.raw_query() {
        if let Ok(parsed) = serde_urlencoded::from_str::<CallbackParams>(raw_q) {
            return parsed;
        }
    }
    CallbackParams::default()
}

fn sanitize_username(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "user".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

async fn generate_unique_username(
    db: &Database,
    identity: &OAuthIdentity,
) -> Result<String, OAuthError> {
    let base_raw = identity
        .username_hint
        .as_deref()
        .or_else(|| {
            identity
                .email
                .as_deref()
                .filter(|_| identity.email_verified)
                .and_then(|e| e.split('@').next())
        })
        .unwrap_or(&identity.provider);
    let base = sanitize_username(base_raw);
    if User::objects()
        .filter(q!(username = base.as_str()))?
        .all(db)
        .await?
        .is_empty()
    {
        return Ok(base);
    }
    for _ in 0..8 {
        let suffix = &random_urlsafe_token()[..8];
        let candidate = format!("{base}_{suffix}");
        if User::objects()
            .filter(q!(username = candidate.as_str()))?
            .all(db)
            .await?
            .is_empty()
        {
            return Ok(candidate);
        }
    }
    Ok(format!("{}_{}", identity.provider, random_urlsafe_token()))
}

/// Resolves an authenticated [`OAuthIdentity`] to a local [`User`] and [`SocialAccount`]
/// using the security lookup order:
/// 1. Existing [`SocialAccount`] matching `(provider, subject)`
/// 2. If `connect_user_id` is `Some(uid)`, link the new [`SocialAccount`] to `uid`
/// 3. Existing [`User`] matching `email` **only when `email_verified == true`**
/// 4. Just-in-time create a new [`User`] (unusable password `"!"`) and [`SocialAccount`]
pub async fn resolve_or_create_social_user(
    db: &Database,
    identity: &OAuthIdentity,
    connect_user_id: Option<i64>,
) -> Result<(User, SocialAccount), OAuthError> {
    let existing_social = SocialAccount::objects()
        .filter(q!(provider = identity.provider.as_str()))?
        .filter(q!(subject = identity.subject.as_str()))?
        .all(db)
        .await?
        .into_iter()
        .next();

    if let Some(mut social) = existing_social {
        if let Some(uid) = connect_user_id {
            if social.user.id != uid {
                return Err(OAuthError::AccountAlreadyLinked);
            }
        }
        let user = User::objects()
            .filter(q!(id = social.user.id))?
            .all(db)
            .await?
            .into_iter()
            .next()
            .ok_or(OAuthError::InactiveUser)?;
        if !user.is_active {
            return Err(OAuthError::InactiveUser);
        }
        let new_email = identity.email.clone().unwrap_or_default();
        let new_extra = serde_json::to_string(&identity.extra).unwrap_or_else(|_| "{}".to_string());
        if social.email != new_email
            || social.email_verified != identity.email_verified
            || social.extra_json != new_extra
        {
            social.email = new_email;
            social.email_verified = identity.email_verified;
            social.extra_json = new_extra;
            social.update(db).await?;
        }
        return Ok((user, social));
    }

    let user = if let Some(uid) = connect_user_id {
        let u = User::objects()
            .filter(q!(id = uid))?
            .all(db)
            .await?
            .into_iter()
            .next()
            .ok_or(OAuthError::Unauthorized)?;
        if !u.is_active {
            return Err(OAuthError::InactiveUser);
        }
        u
    } else if identity.email_verified {
        if let Some(verified_email) = identity
            .email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let matches = User::objects()
                .filter(q!(email = verified_email))?
                .all(db)
                .await?;
            match matches.len() {
                0 => create_user_for_identity(db, identity).await?,
                1 => {
                    let u = matches.into_iter().next().unwrap();
                    if !u.is_active {
                        return Err(OAuthError::InactiveUser);
                    }
                    u
                }
                _ => return Err(OAuthError::AmbiguousEmail),
            }
        } else {
            create_user_for_identity(db, identity).await?
        }
    } else {
        // NEVER link by email when `email_verified` is false.
        create_user_for_identity(db, identity).await?
    };

    let social = SocialAccount {
        id: 0,
        provider: identity.provider.clone(),
        subject: identity.subject.clone(),
        user: ForeignKey::new(user.id),
        email: identity.email.clone().unwrap_or_default(),
        email_verified: identity.email_verified,
        extra_json: serde_json::to_string(&identity.extra).unwrap_or_else(|_| "{}".to_string()),
        created_at: Utc::now(),
    }
    .save(db)
    .await?;

    Ok((user, social))
}

async fn create_user_for_identity(
    db: &Database,
    identity: &OAuthIdentity,
) -> Result<User, OAuthError> {
    let username = generate_unique_username(db, identity).await?;
    let email = if identity.email_verified {
        identity.email.clone().unwrap_or_default()
    } else {
        String::new()
    };
    let user = User {
        id: 0,
        username,
        email,
        password: "!".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: Utc::now(),
        last_login: None,
    }
    .save(db)
    .await?;
    Ok(user)
}

/// `GET /auth/{provider}/login` — begins an OAuth 2 / OIDC login flow.
pub async fn oauth_login_view(
    req: Request,
    params: PathParams,
) -> Result<Response, djangors_core::DjangorsError> {
    let provider = params
        .get("provider")
        .ok_or_else(|| djangors_core::DjangorsError::BadRequest("missing provider".to_string()))?;
    start_flow(&req, provider, None).await.map_err(Into::into)
}

/// `GET|POST /auth/{provider}/callback` — validates one-time `state`, PKCE,
/// `nonce`, ID token / userinfo, resolves/links the local user, and logs them in.
pub async fn oauth_callback_view(
    req: Request,
    params: PathParams,
) -> Result<Response, djangors_core::DjangorsError> {
    let provider_name = params
        .get("provider")
        .ok_or_else(|| djangors_core::DjangorsError::BadRequest("missing provider".to_string()))?
        .to_string();
    let registry = req
        .state::<OAuthRegistry>()
        .ok_or_else(|| OAuthError::InvalidConfig("OAuthRegistry not in AppState".to_string()))?
        .clone();
    let db = req.require_state::<Database>()?;
    let session = req
        .ext::<Session>()
        .ok_or(OAuthError::Unauthorized)?
        .clone();

    let cb = extract_callback_params(&req).await;
    if let Some(err_code) = cb.error {
        return Err(OAuthError::ProviderReturnedError {
            error: err_code,
            description: cb.error_description.unwrap_or_default(),
        }
        .into());
    }

    let state = cb.state.as_deref().ok_or(OAuthError::StateMismatch)?;
    let code = cb
        .code
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| OAuthError::TokenExchange("missing authorization code".to_string()))?;

    let key = pending_session_key(state);
    let pending: PendingOAuthFlow = session.get(&key).ok_or(OAuthError::StateMismatch)?;
    // Single-use state: consume immediately before calling external endpoints.
    session.remove(&key);

    if pending.provider != provider_name
        || !bool::from(pending.state.as_bytes().ct_eq(state.as_bytes()))
        || Utc::now().timestamp() > pending.expires_at_ts
    {
        return Err(OAuthError::StateMismatch.into());
    }

    if let Some(expected_uid) = pending.connect_user_id {
        if session.get::<i64>(SESSION_USER_ID_KEY) != Some(expected_uid) {
            return Err(OAuthError::Unauthorized.into());
        }
    }

    let provider = registry
        .get(&provider_name)
        .ok_or_else(|| OAuthError::UnknownProvider(provider_name.clone()))?;
    let redirect_uri = registry.callback_url(&provider_name);

    let mut identity = provider
        .exchange_and_fetch_identity(
            &redirect_uri,
            code,
            Some(&pending.pkce_verifier),
            Some(&pending.nonce),
        )
        .await?;

    if identity.name.is_none() {
        if let Some(apple_name) = parse_apple_form_name(cb.user.as_deref()) {
            identity.name = Some(apple_name);
        }
    }

    let (user, _) = resolve_or_create_social_user(db, &identity, pending.connect_user_id).await?;

    if pending.connect_user_id.is_some() {
        Ok(Response::redirect(&registry.post_connect_redirect))
    } else {
        login(&session, &user);
        Ok(Response::redirect(&registry.post_login_redirect))
    }
}

/// `GET /auth/{provider}/connect` — links a social provider to the currently logged-in user.
pub async fn connect_provider_view(
    req: Request,
    params: PathParams,
) -> Result<Response, djangors_core::DjangorsError> {
    let provider = params
        .get("provider")
        .ok_or_else(|| djangors_core::DjangorsError::BadRequest("missing provider".to_string()))?;
    let session = req.ext::<Session>().ok_or(OAuthError::Unauthorized)?;
    let uid = session
        .get::<i64>(SESSION_USER_ID_KEY)
        .ok_or(OAuthError::Unauthorized)?;
    start_flow(&req, provider, Some(uid))
        .await
        .map_err(Into::into)
}

/// `POST /auth/{provider}/disconnect` — unlinks `provider` from the currently logged-in user.
///
/// Refuses (`400 Bad Request`) if removing this account would leave the user with
/// neither a usable password nor another linked [`SocialAccount`].
pub async fn disconnect_provider_view(
    req: Request,
    params: PathParams,
) -> Result<Response, djangors_core::DjangorsError> {
    let provider = params
        .get("provider")
        .ok_or_else(|| djangors_core::DjangorsError::BadRequest("missing provider".to_string()))?
        .to_string();
    let db = req.require_state::<Database>()?;
    let session = req.ext::<Session>().ok_or(OAuthError::Unauthorized)?;
    let uid = session
        .get::<i64>(SESSION_USER_ID_KEY)
        .ok_or(OAuthError::Unauthorized)?;

    let user = User::objects()
        .filter(q!(id = uid))
        .map_err(OAuthError::from)?
        .first(db)
        .await
        .map_err(OAuthError::from)?
        .ok_or(OAuthError::Unauthorized)?;

    let accounts = SocialAccount::objects()
        .filter(q!(user = uid))
        .map_err(OAuthError::from)?
        .all(db)
        .await
        .map_err(OAuthError::from)?;

    let matching: Vec<SocialAccount> = accounts
        .iter()
        .filter(|a| a.provider == provider)
        .cloned()
        .collect();
    if matching.is_empty() {
        return Err(djangors_core::DjangorsError::api(
            StatusCode::NOT_FOUND,
            "not_linked",
            format!("no linked '{provider}' social account found"),
        ));
    }

    let remaining_other_accounts = accounts.iter().filter(|a| a.provider != provider).count();
    let has_usable_password = !user.password.is_empty() && !user.password.starts_with('!');
    if !has_usable_password && remaining_other_accounts == 0 {
        return Err(OAuthError::CannotDisconnectLastAuthMethod.into());
    }

    for acct in matching {
        acct.delete(db).await.map_err(OAuthError::from)?;
    }

    Response::json(
        StatusCode::OK,
        &serde_json::json!({ "disconnected": provider }),
    )
}

/// Mounts all social-login client routes:
/// - `GET /auth/{provider}/login`
/// - `GET|POST /auth/{provider}/callback`
/// - `GET /auth/{provider}/connect`
/// - `POST /auth/{provider}/disconnect`
pub fn oauth_client_router() -> Router {
    Router::new()
        .get("/auth/{provider}/login", oauth_login_view)
        .get("/auth/{provider}/callback", oauth_callback_view)
        .post("/auth/{provider}/callback", oauth_callback_view)
        .get("/auth/{provider}/connect", connect_provider_view)
        .post("/auth/{provider}/disconnect", disconnect_provider_view)
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
