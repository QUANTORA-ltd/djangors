//! Error types for OAuth 2.0 and OpenID Connect workflows.

use djangors_core::{DjangorsError, StatusCode};
use thiserror::Error;

/// Errors raised by OAuth 2.0 / OIDC client and provider operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum OAuthError {
    /// Requested provider name is not registered in [`crate::OAuthRegistry`].
    #[error("unknown OAuth provider '{0}'")]
    UnknownProvider(String),

    /// Missing or mismatched CSRF `state` parameter on callback.
    #[error("invalid or expired OAuth state parameter")]
    StateMismatch,

    /// Callback arrived without a valid pending PKCE code verifier in session.
    #[error("missing PKCE code verifier in session")]
    MissingPkceVerifier,

    /// OIDC callback arrived without a pending nonce in session.
    #[error("missing OIDC nonce in session")]
    MissingNonce,

    /// OIDC ID token nonce claim did not match the session nonce.
    #[error("OIDC ID token nonce mismatch")]
    NonceMismatch,

    /// OIDC ID token missing, malformed, or failed cryptographic/claims verification.
    #[error("invalid OIDC ID token: {0}")]
    InvalidIdToken(String),

    /// Authorization code exchange or token endpoint call failed.
    #[error("OAuth token exchange failed: {0}")]
    TokenExchange(String),

    /// UserInfo / profile endpoint request failed.
    #[error("OAuth userinfo fetch failed: {0}")]
    UserInfo(String),

    /// Provider or endpoint configuration is invalid.
    #[error("invalid OAuth configuration: {0}")]
    InvalidConfig(String),

    /// HTTP transport error talking to upstream IdP.
    #[error("OAuth HTTP client error: {0}")]
    Http(String),

    /// Attempted to disconnect the only remaining authentication method on a user account.
    #[error("cannot disconnect the only remaining login method without a usable password")]
    CannotDisconnectLastAuthMethod,

    /// External `(provider, subject)` account is already linked to a different user.
    #[error("social account is already linked to another user")]
    AccountAlreadyLinked,

    /// Target local user account is disabled (`is_active == false`).
    #[error("user account is inactive")]
    InactiveUser,

    /// Authentication required before performing this OAuth action.
    #[error("authentication required")]
    Unauthorized,

    /// Multiple local user rows matched the verified email address.
    #[error("ambiguous verified email match across local users")]
    AmbiguousEmail,

    /// Upstream OAuth callback returned an `error` query/form parameter.
    #[error("provider returned error '{error}': {description}")]
    ProviderReturnedError {
        /// OAuth `error` code from provider.
        error: String,
        /// Optional human-readable error description.
        description: String,
    },

    /// Database / ORM query error.
    #[error("database error: {0}")]
    Orm(#[from] djangors_orm::OrmError),

    /// Underlying authentication error.
    #[error("auth error: {0}")]
    Auth(#[from] djangors_auth::AuthError),

    /// OAuth 2.0 `invalid_request` provider error.
    #[error("invalid_request: {0}")]
    InvalidRequest(String),

    /// OAuth 2.0 `invalid_client` provider error.
    #[error("invalid_client: {0}")]
    InvalidClient(String),

    /// OAuth 2.0 `invalid_grant` provider error.
    #[error("invalid_grant: {0}")]
    InvalidGrant(String),

    /// OAuth 2.0 `unauthorized_client` provider error.
    #[error("unauthorized_client: {0}")]
    UnauthorizedClient(String),

    /// OAuth 2.0 `unsupported_grant_type` provider error.
    #[error("unsupported_grant_type: {0}")]
    UnsupportedGrantType(String),

    /// OAuth 2.0 `invalid_scope` provider error.
    #[error("invalid_scope: {0}")]
    InvalidScope(String),

    /// OAuth 2.0 `access_denied` provider error.
    #[error("access_denied: {0}")]
    AccessDenied(String),

    /// OAuth 2.0 `unsupported_response_type` provider error.
    #[error("unsupported_response_type: {0}")]
    UnsupportedResponseType(String),
}

impl From<OAuthError> for DjangorsError {
    fn from(err: OAuthError) -> Self {
        match err {
            OAuthError::UnknownProvider(_) => {
                DjangorsError::api(StatusCode::NOT_FOUND, "unknown_provider", err.to_string())
            }
            OAuthError::StateMismatch
            | OAuthError::MissingPkceVerifier
            | OAuthError::MissingNonce
            | OAuthError::NonceMismatch
            | OAuthError::InvalidIdToken(_)
            | OAuthError::CannotDisconnectLastAuthMethod
            | OAuthError::ProviderReturnedError { .. } => {
                DjangorsError::api(StatusCode::BAD_REQUEST, "oauth_error", err.to_string())
            }
            OAuthError::AccountAlreadyLinked | OAuthError::AmbiguousEmail => {
                DjangorsError::api(StatusCode::CONFLICT, "oauth_conflict", err.to_string())
            }
            OAuthError::Unauthorized => DjangorsError::Unauthorized(err.to_string()),
            OAuthError::InactiveUser => DjangorsError::Forbidden(err.to_string()),
            OAuthError::TokenExchange(_)
            | OAuthError::UserInfo(_)
            | OAuthError::Http(_)
            | OAuthError::InvalidConfig(_) => DjangorsError::Internal(err.to_string()),
            OAuthError::Orm(e) => DjangorsError::Internal(e.to_string()),
            OAuthError::Auth(e) => e.into(),
            OAuthError::InvalidRequest(_)
            | OAuthError::InvalidGrant(_)
            | OAuthError::UnsupportedGrantType(_)
            | OAuthError::InvalidScope(_)
            | OAuthError::UnsupportedResponseType(_) => {
                DjangorsError::api(StatusCode::BAD_REQUEST, "oauth_error", err.to_string())
            }
            OAuthError::InvalidClient(_) => {
                DjangorsError::api(StatusCode::UNAUTHORIZED, "invalid_client", err.to_string())
            }
            OAuthError::UnauthorizedClient(_) | OAuthError::AccessDenied(_) => {
                DjangorsError::api(StatusCode::FORBIDDEN, "access_denied", err.to_string())
            }
        }
    }
}
