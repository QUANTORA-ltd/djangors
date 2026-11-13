//! Error types for SAML 2.0 Service Provider workflows.

use djangors_core::{DjangorsError, StatusCode};
use thiserror::Error;

/// Errors raised during SAML 2.0 metadata parsing, XML-DSig verification,
/// assertion validation, and SSO user resolution.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SamlError {
    /// Requested IdP slug is not configured in `saml_identity_provider`.
    #[error("unknown SAML identity provider '{0}'")]
    UnknownProvider(String),

    /// Requested IdP is marked inactive (`is_active = false`).
    #[error("SAML identity provider '{0}' is disabled")]
    InactiveProvider(String),

    /// IdP metadata XML or SP configuration is malformed or missing required elements.
    #[error("invalid SAML metadata or configuration: {0}")]
    InvalidMetadata(String),

    /// X.509 certificate PEM/DER could not be parsed or extracted.
    #[error("invalid X.509 certificate: {0}")]
    InvalidCertificate(String),

    /// SAML Response or Assertion carried no valid XML-DSig signature.
    #[error("unsigned SAML response or assertion is not allowed")]
    UnsignedResponse,

    /// XML-DSig digest or cryptographic signature verification failed.
    #[error("SAML XML signature verification failed: {0}")]
    SignatureVerification(String),

    /// Structural XML Signature Wrapping (XSW) or DTD/entity anomaly detected.
    #[error("unsafe SAML XML structure rejected: {0}")]
    SignatureWrapping(String),

    /// SAML assertion ID has already been consumed (replay attack).
    #[error("SAML assertion replay detected for ID '{0}'")]
    ReplayDetected(String),

    /// Assertion `NotBefore` or `NotOnOrAfter` window check failed.
    #[error("SAML assertion expired or not yet valid: {0}")]
    ExpiredAssertion(String),

    /// Assertion `<Audience>` did not match SP entity ID.
    #[error("SAML audience mismatch: expected '{expected}', got '{actual}'")]
    AudienceMismatch {
        /// Configured SP entity ID.
        expected: String,
        /// Audience(s) found inside the assertion.
        actual: String,
    },

    /// `Destination` or `Recipient` did not match the SP ACS URL.
    #[error("SAML recipient mismatch: expected '{expected}', got '{actual}'")]
    RecipientMismatch {
        /// Configured SP ACS URL.
        expected: String,
        /// Recipient/Destination received in SAML payload.
        actual: String,
    },

    /// `InResponseTo` did not match a pending AuthnRequest ID in session.
    #[error("SAML InResponseTo validation failed: {0}")]
    InResponseToMismatch(String),

    /// SAML StatusCode was not `urn:oasis:names:tc:SAML:2.0:status:Success`.
    #[error("SAML IdP returned non-success status: {0}")]
    StatusNotSuccess(String),

    /// Required identity attribute (`username` / `NameID` / `email`) was absent.
    #[error("missing required SAML assertion attribute: {0}")]
    MissingAttribute(String),

    /// Matched local user is disabled (`is_active == false`).
    #[error("user account is inactive")]
    InactiveUser,

    /// Pending AuthnRequest / RelayState session validation failed.
    #[error("missing or mismatched SAML session request state")]
    RelayStateMismatch,

    /// Database / ORM error.
    #[error("database error: {0}")]
    Orm(#[from] djangors_orm::OrmError),

    /// Underlying authentication error.
    #[error("auth error: {0}")]
    Auth(#[from] djangors_auth::AuthError),

    /// Replay cache error.
    #[error("cache error: {0}")]
    Cache(#[from] djangors_cache::CacheError),
}

impl From<SamlError> for DjangorsError {
    fn from(err: SamlError) -> Self {
        match err {
            SamlError::UnknownProvider(_) => {
                DjangorsError::api(StatusCode::NOT_FOUND, "unknown_saml_idp", err.to_string())
            }
            SamlError::InactiveProvider(_) | SamlError::InactiveUser => {
                DjangorsError::Forbidden(err.to_string())
            }
            SamlError::UnsignedResponse
            | SamlError::SignatureVerification(_)
            | SamlError::SignatureWrapping(_)
            | SamlError::ReplayDetected(_)
            | SamlError::ExpiredAssertion(_)
            | SamlError::AudienceMismatch { .. }
            | SamlError::RecipientMismatch { .. }
            | SamlError::InResponseToMismatch(_)
            | SamlError::StatusNotSuccess(_)
            | SamlError::MissingAttribute(_)
            | SamlError::RelayStateMismatch => {
                DjangorsError::api(StatusCode::UNAUTHORIZED, "saml_rejected", err.to_string())
            }
            SamlError::InvalidMetadata(_) | SamlError::InvalidCertificate(_) => {
                DjangorsError::BadRequest(err.to_string())
            }
            SamlError::Orm(e) => DjangorsError::Internal(e.to_string()),
            SamlError::Auth(e) => e.into(),
            SamlError::Cache(e) => DjangorsError::Internal(e.to_string()),
        }
    }
}
