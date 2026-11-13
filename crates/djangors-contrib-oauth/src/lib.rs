#![deny(missing_docs)]
//! OAuth 2.0 and OpenID Connect integration for Djangors.
//!
//! Includes:
//! - `client` (feature `client`, on by default): Social login / OIDC relying
//!   party (`GoogleProvider`, `MicrosoftProvider`, `AppleProvider`, `GitHubProvider`,
//!   generic `OidcProvider`, `OAuthProvider` trait, `SocialAccount`, connect/disconnect views).
//! - `provider` (feature `provider`, on by default): OAuth 2.0 / OIDC authorization
//!   server (`OAuthApplication`, `AuthorizationCode`, `AccessToken`, `RefreshToken`,
//!   `SigningKey`, `OAuthProviderConfig`, `/o/authorize`, `/o/token`, `/o/revoke`,
//!   `/o/introspect`, `/o/userinfo`, `/o/jwks`, `OAuthBearer`, `HasOAuthScope`).

/// Error types for OAuth 2.0 / OIDC operations.
pub mod error;

/// OAuth 2.0 and OIDC client / social login implementation.
#[cfg(feature = "client")]
pub mod client;

/// OAuth 2.0 and OIDC authorization server (provider) implementation.
#[cfg(feature = "provider")]
pub mod provider;

pub use error::OAuthError;

#[cfg(feature = "client")]
pub use client::{
    connect_provider_view, create_social_account_table, disconnect_provider_view,
    oauth_callback_view, oauth_client_router, oauth_login_view, resolve_or_create_social_user,
    AppleConfig, AppleProvider, GitHubConfig, GitHubProvider, GoogleProvider, MicrosoftProvider,
    MicrosoftTenant, OAuthIdentity, OAuthProvider, OAuthRegistry, OidcConfig, OidcProvider,
    SocialAccount,
};

#[cfg(all(feature = "client", feature = "admin"))]
pub use client::register_client_admin;

#[cfg(feature = "provider")]
pub use provider::{
    authorize_get_view, authorize_post_view, create_oauth_provider_tables, introspect_view,
    jwks_view, oauth_provider_router, openid_configuration_view, revoke_app_tokens, revoke_view,
    token_view, userinfo_view, AccessToken, AuthorizationCode, HasOAuthScope, OAuthApplication,
    OAuthBearer, OAuthProviderConfig, RefreshToken, SigningKey,
};

#[cfg(all(feature = "provider", feature = "admin"))]
pub use provider::register_provider_admin;
